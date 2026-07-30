use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::native_video::{
    find_ffmpeg, list_video_devices, NativeVideoConfig, NativeVideoDevice, NativeVideoInfo,
    NativeVideoSource, NativeVideoSourceKind,
};
use open_quartz::gpu::{
    GpuBackend, GpuExecutor, GpuOutput, TextureFormat, BLIT_FRAG, FULLSCREEN_VERT_WITH_UV,
};
use open_quartz::Engine;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, Window};

const OUTPUT_WINDOW_LABEL: &str = "native-output";

#[derive(Default)]
pub struct NativeRuntimeState {
    runtime: Arc<Mutex<Option<NativeGpuRuntime>>>,
    alive: Arc<AtomicBool>,
    playing: Arc<AtomicBool>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl Drop for NativeRuntimeState {
    fn drop(&mut self) {
        shutdown_worker(self);
    }
}

#[derive(Default)]
pub struct NativeOnnxState {
    sessions: Mutex<HashMap<String, open_quartz::onnx::OnnxSession>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeRuntimeInfo {
    adapter_name: String,
    backend: String,
    device_type: String,
    surface_format: String,
    native_onnx_cpu: bool,
    native_onnx_direct_ml: bool,
    shared_onnx_wgpu_device: bool,
    native_video: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeFrameRendered {
    frame: u64,
    revision: u32,
    output_node_id: String,
    width: u32,
    height: u32,
}

struct NativeGpuRuntime {
    window: Window,
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    backend: Arc<GpuBackend>,
    executor: GpuExecutor,
    presenter: SurfacePresenter,
    engine: Engine,
    output_node_id: Option<String>,
    started_at: Instant,
    previous_frame_at: Instant,
    frame: u64,
    mouse: [f32; 4],
    videos: HashMap<String, NativeVideoSource>,
}

impl NativeGpuRuntime {
    async fn new(window: Window) -> Result<(Self, NativeRuntimeInfo), String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: native_backends(),
            ..Default::default()
        });
        let surface = instance
            .create_surface(Arc::new(window.clone()))
            .map_err(|error| format!("Cannot create native GPU surface: {error}"))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await
            .map_err(|error| format!("Cannot find a native GPU adapter: {error}"))?;
        let adapter_info = adapter.get_info();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .map_err(|error| format!("Cannot create native GPU device: {error}"))?;
        let size = window
            .inner_size()
            .map_err(|error| format!("Cannot query output window size: {error}"))?;
        let surface_config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| {
                "Native GPU surface is unsupported by the selected adapter".to_owned()
            })?;
        surface.configure(&device, &surface_config);
        let backend = Arc::new(GpuBackend::from_device(device, queue));
        let executor = GpuExecutor::new(backend.clone());
        let presenter = SurfacePresenter::new(&backend.device, surface_config.format);
        let info = NativeRuntimeInfo {
            adapter_name: adapter_info.name,
            backend: format!("{:?}", adapter_info.backend),
            device_type: format!("{:?}", adapter_info.device_type),
            surface_format: format!("{:?}", surface_config.format),
            native_onnx_cpu: true,
            native_video: find_ffmpeg().is_ok(),
            native_onnx_direct_ml: cfg!(target_os = "windows"),
            shared_onnx_wgpu_device: false,
        };
        let now = Instant::now();
        Ok((
            Self {
                window,
                surface,
                surface_config,
                backend,
                executor,
                presenter,
                engine: Engine::new_native(),
                output_node_id: None,
                started_at: now,
                previous_frame_at: now,
                frame: 0,
                mouse: [0.0; 4],
                videos: HashMap::new(),
            },
            info,
        ))
    }

    fn set_graph(&mut self, graph_json: &str) -> Result<u32, String> {
        let revision = self.engine.set_graph_json(graph_json)?;
        let (output_node_id, node_ids) = {
            let plan = self
                .engine
                .execution_plan()
                .ok_or_else(|| "Native engine did not retain an execution plan".to_owned())?;
            self.executor
                .sync_plan(plan)
                .map_err(|error| error.to_string())?;
            (
                plan.output_nodes.first().cloned(),
                plan.nodes
                    .iter()
                    .map(|node| node.id.clone())
                    .collect::<std::collections::HashSet<_>>(),
            )
        };
        self.output_node_id = output_node_id;
        self.videos.retain(|node_id, _| node_ids.contains(node_id));
        self.sync_video_nodes()?;
        self.started_at = Instant::now();
        self.previous_frame_at = self.started_at;
        self.frame = 0;
        Ok(revision)
    }

    fn upload_image(
        &mut self,
        node_id: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        self.engine.node_generation(node_id)?;
        self.executor
            .upload_rgba(node_id, rgba, width, height)
            .map_err(|error| error.to_string())?;
        self.engine.mark_dirty(node_id)
    }

    fn remove_texture(&mut self, node_id: &str) {
        self.executor.remove_texture(node_id);
    }

    fn read_output(&self, node_id: &str) -> Result<Vec<u8>, String> {
        let output = self
            .executor
            .output_texture(node_id)
            .ok_or_else(|| format!("Output node {node_id} has no native GPU texture"))?;
        if output.format != TextureFormat::Rgba8Unorm {
            return Err(format!(
                "Output node {node_id} uses {:?}; Stage E readback currently requires rgba8unorm",
                output.format
            ));
        }
        let (width, height) = (output.width, output.height);
        let rgba = pollster::block_on(self.executor.read_output_rgba(node_id))
            .map_err(|error| error.to_string())?;
        let mut payload = Vec::with_capacity(8 + rgba.len());
        payload.extend_from_slice(&width.to_le_bytes());
        payload.extend_from_slice(&height.to_le_bytes());
        payload.extend_from_slice(&rgba);
        Ok(payload)
    }

    fn attach_video(
        &mut self,
        node_id: &str,
        config: NativeVideoConfig,
    ) -> Result<NativeVideoInfo, String> {
        self.engine.node_generation(node_id)?;
        let source = NativeVideoSource::open(config)?;
        let info = source.info().clone();
        self.videos.insert(node_id.to_owned(), source);
        self.sync_video_nodes()?;
        self.engine.mark_dirty(node_id)?;
        Ok(info)
    }

    fn detach_video(&mut self, node_id: &str) -> Result<(), String> {
        self.videos.remove(node_id);
        self.executor.remove_texture(node_id);
        self.sync_video_nodes()
    }

    fn sync_video_nodes(&mut self) -> Result<(), String> {
        let node_ids = self.videos.keys().cloned().collect::<Vec<_>>();
        let json = serde_json::to_string(&node_ids)
            .map_err(|error| format!("Cannot serialize native video nodes: {error}"))?;
        self.engine.set_video_nodes_json(&json)
    }

    fn upload_video_frames(&mut self) -> Result<(), String> {
        let (videos, executor) = (&mut self.videos, &mut self.executor);
        let mut dirty = Vec::new();
        for (node_id, source) in videos {
            let uploaded = source.upload_latest(|rgba, width, height| {
                executor
                    .upload_rgba(node_id, rgba, width, height)
                    .map_err(|error| error.to_string())
            })?;
            if uploaded {
                dirty.push(node_id.clone());
            }
        }
        for node_id in dirty {
            self.engine.mark_dirty(&node_id)?;
        }
        Ok(())
    }

    fn pause_videos(&mut self) {
        for source in self.videos.values_mut() {
            source.pause();
        }
    }

    fn resume_videos(&mut self) -> Result<(), String> {
        for source in self.videos.values_mut() {
            source.resume()?;
        }
        Ok(())
    }

    fn render_next(&mut self) -> Result<NativeFrameRendered, String> {
        let now = Instant::now();
        let time = now.duration_since(self.started_at).as_secs_f64();
        let delta = now.duration_since(self.previous_frame_at).as_secs_f64();
        self.previous_frame_at = now;
        self.frame = self.frame.saturating_add(1);
        self.render(time, delta, self.frame)
    }

    fn render(&mut self, time: f64, delta: f64, frame: u64) -> Result<NativeFrameRendered, String> {
        let size = self
            .window
            .inner_size()
            .map_err(|error| format!("Cannot query output window size: {error}"))?;
        let width = size.width.max(1);
        let height = size.height.max(1);
        self.upload_video_frames()?;
        let date = utc_date_uniform(SystemTime::now());
        self.engine.run_frame(
            time,
            delta,
            frame,
            &date,
            &self.mouse,
            &[width as f32, height as f32, 1.0],
        )?;
        let plan = self
            .engine
            .execution_plan()
            .ok_or_else(|| "Native engine has no execution plan".to_owned())?;
        self.executor
            .execute_commands(plan, self.engine.pending_commands())
            .map_err(|error| error.to_string())?;
        let output_node_id = self
            .output_node_id
            .clone()
            .or_else(|| plan.output_nodes.first().cloned())
            .ok_or_else(|| "Graph has no renderer or terminal texture output".to_owned())?;
        let output = self
            .executor
            .output_texture(&output_node_id)
            .ok_or_else(|| format!("Output node {output_node_id} has no native GPU texture"))?;
        if self.surface_config.width != width || self.surface_config.height != height {
            self.surface_config.width = width;
            self.surface_config.height = height;
            self.surface
                .configure(&self.backend.device, &self.surface_config);
        }
        self.presenter
            .present(&self.surface, &self.surface_config, &self.backend, &output)?;
        Ok(NativeFrameRendered {
            frame,
            revision: self.engine.revision(),
            output_node_id,
            width: output.width,
            height: output.height,
        })
    }
}

struct SurfacePresenter {
    bind_group_layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
}

impl SurfacePresenter {
    fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("open-quartz-surface-bindings"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("open-quartz-surface-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let vertex = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("open-quartz-surface-vertex"),
            source: wgpu::ShaderSource::Wgsl(FULLSCREEN_VERT_WITH_UV.into()),
        });
        let fragment = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("open-quartz-surface-fragment"),
            source: wgpu::ShaderSource::Wgsl(BLIT_FRAG.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("open-quartz-surface-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vertex,
                entry_point: Some("main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &fragment,
                entry_point: Some("main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("open-quartz-surface-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            bind_group_layout,
            pipeline,
            sampler,
        }
    }

    fn present(
        &self,
        surface: &wgpu::Surface<'_>,
        surface_config: &wgpu::SurfaceConfiguration,
        backend: &GpuBackend,
        source: &GpuOutput<'_>,
    ) -> Result<(), String> {
        let frame = match surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                surface.configure(&backend.device, surface_config);
                surface
                    .get_current_texture()
                    .map_err(|error| format!("Cannot reacquire native output surface: {error}"))?
            }
            Err(error) => return Err(format!("Cannot acquire native output surface: {error}")),
        };
        let output_view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = backend
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("open-quartz-surface-bind-group"),
                layout: &self.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(source.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
        let mut encoder = backend
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("open-quartz-surface-frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("open-quartz-surface-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &output_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        backend.queue.submit([encoder.finish()]);
        frame.present();
        Ok(())
    }
}

async fn initialize_runtime(
    app: &AppHandle,
    state: &NativeRuntimeState,
) -> Result<NativeRuntimeInfo, String> {
    shutdown_worker(state);
    if let Some(previous) = state
        .runtime
        .lock()
        .map_err(|_| "Native runtime lock is poisoned".to_owned())?
        .take()
    {
        let _ = previous.window.close();
    }
    if let Some(window) = app.get_window(OUTPUT_WINDOW_LABEL) {
        let _ = window.close();
    }
    let window = tauri::window::WindowBuilder::new(app, OUTPUT_WINDOW_LABEL)
        .title("Open Quartz Output")
        .inner_size(960.0, 540.0)
        .resizable(true)
        .visible(true)
        .build()
        .map_err(|error| format!("Cannot create native output window: {error}"))?;
    let (runtime, info) = NativeGpuRuntime::new(window).await?;
    state.alive.store(true, Ordering::Release);
    let alive = state.alive.clone();
    let playing = state.playing.clone();
    runtime.window.on_window_event(move |event| {
        if matches!(
            event,
            tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed
        ) {
            playing.store(false, Ordering::Release);
            alive.store(false, Ordering::Release);
        }
    });
    *state
        .runtime
        .lock()
        .map_err(|_| "Native runtime lock is poisoned".to_owned())? = Some(runtime);
    state.playing.store(false, Ordering::Release);
    start_worker(app, state)?;
    Ok(info)
}

#[tauri::command]
pub async fn native_gpu_initialize(
    app: AppHandle,
    state: State<'_, NativeRuntimeState>,
) -> Result<NativeRuntimeInfo, String> {
    initialize_runtime(&app, &state).await
}

#[tauri::command]
pub fn native_gpu_set_graph(
    graph_json: String,
    state: State<'_, NativeRuntimeState>,
) -> Result<u32, String> {
    with_runtime(&state, |runtime| runtime.set_graph(&graph_json))
}

#[tauri::command]
pub fn native_gpu_upload_image(
    request: tauri::ipc::Request<'_>,
    state: State<'_, NativeRuntimeState>,
) -> Result<(), String> {
    let node_id = resource_header(&request, "x-open-quartz-node-id")?.to_owned();
    let width = resource_header(&request, "x-open-quartz-width")?
        .parse::<u32>()
        .map_err(|error| format!("Invalid image width: {error}"))?;
    let height = resource_header(&request, "x-open-quartz-height")?
        .parse::<u32>()
        .map_err(|error| format!("Invalid image height: {error}"))?;
    let rgba = match request.body() {
        tauri::ipc::InvokeBody::Raw(bytes) => bytes.as_slice(),
        tauri::ipc::InvokeBody::Json(_) => {
            return Err("Image upload requires a raw Uint8Array body".to_owned());
        }
    };
    with_runtime(&state, |runtime| {
        runtime.upload_image(&node_id, rgba, width, height)
    })
}

#[tauri::command]
pub fn native_gpu_remove_texture(
    node_id: String,
    state: State<'_, NativeRuntimeState>,
) -> Result<(), String> {
    with_runtime(&state, |runtime| {
        runtime.remove_texture(&node_id);
        Ok(())
    })
}

#[tauri::command]
pub fn native_gpu_read_output(
    node_id: String,
    state: State<'_, NativeRuntimeState>,
) -> Result<tauri::ipc::Response, String> {
    let payload = with_runtime(&state, |runtime| runtime.read_output(&node_id))?;
    Ok(tauri::ipc::Response::new(payload))
}

fn resource_header<'a>(
    request: &'a tauri::ipc::Request<'_>,
    name: &str,
) -> Result<&'a str, String> {
    request
        .headers()
        .get(name)
        .ok_or_else(|| format!("Missing image resource header {name}"))?
        .to_str()
        .map_err(|error| format!("Invalid image resource header {name}: {error}"))
}

#[tauri::command]
pub fn native_gpu_play(state: State<'_, NativeRuntimeState>) -> Result<(), String> {
    with_runtime(&state, |runtime| {
        if runtime.engine.execution_plan().is_none() {
            return Err("Native runtime must receive a graph before play".to_owned());
        }
        Ok(())
    })?;
    state.playing.store(true, Ordering::Release);
    Ok(())
}

#[tauri::command]
pub fn native_gpu_pause(state: State<'_, NativeRuntimeState>) -> Result<(), String> {
    with_runtime(&state, |runtime| {
        runtime.pause_videos();
        runtime.engine.pause()
    })?;
    state.playing.store(false, Ordering::Release);
    Ok(())
}

#[tauri::command]
pub fn native_gpu_resume(state: State<'_, NativeRuntimeState>) -> Result<(), String> {
    with_runtime(&state, |runtime| {
        runtime.resume_videos()?;
        runtime.engine.resume()
    })?;
    state.playing.store(true, Ordering::Release);
    Ok(())
}

#[tauri::command]
pub fn native_gpu_stop(state: State<'_, NativeRuntimeState>) -> Result<(), String> {
    state.playing.store(false, Ordering::Release);
    with_runtime(&state, |runtime| {
        runtime.pause_videos();
        runtime.engine.stop()
    })
}

#[tauri::command]
pub fn native_gpu_attach_video(
    node_id: String,
    kind: NativeVideoSourceKind,
    source: String,
    looping: bool,
    playback_rate: f64,
    state: State<'_, NativeRuntimeState>,
) -> Result<NativeVideoInfo, String> {
    with_runtime(&state, |runtime| {
        runtime.attach_video(
            &node_id,
            NativeVideoConfig {
                kind,
                source,
                looping,
                playback_rate,
            },
        )
    })
}

#[tauri::command]
pub fn native_gpu_detach_video(
    node_id: String,
    state: State<'_, NativeRuntimeState>,
) -> Result<(), String> {
    with_runtime(&state, |runtime| runtime.detach_video(&node_id))
}
#[tauri::command]
pub fn native_video_devices() -> Result<Vec<NativeVideoDevice>, String> {
    list_video_devices()
}

#[tauri::command]
pub fn native_gpu_render_once(
    state: State<'_, NativeRuntimeState>,
) -> Result<NativeFrameRendered, String> {
    with_runtime(&state, NativeGpuRuntime::render_next)
}

#[tauri::command]
pub fn native_gpu_set_mouse(
    mouse: [f32; 4],
    state: State<'_, NativeRuntimeState>,
) -> Result<(), String> {
    with_runtime(&state, |runtime| {
        runtime.mouse = mouse;
        Ok(())
    })
}

#[tauri::command]
pub fn native_gpu_drain_events(state: State<'_, NativeRuntimeState>) -> Result<String, String> {
    with_runtime(&state, |runtime| Ok(runtime.engine.drain_events_json()))
}

#[tauri::command]
pub fn native_gpu_close(state: State<'_, NativeRuntimeState>) -> Result<(), String> {
    close_runtime(&state)
}

fn close_runtime(state: &NativeRuntimeState) -> Result<(), String> {
    shutdown_worker(state);
    let runtime = state
        .runtime
        .lock()
        .map_err(|_| "Native runtime lock is poisoned".to_owned())?
        .take();
    if let Some(runtime) = runtime {
        runtime
            .window
            .close()
            .map_err(|error| format!("Cannot close native output window: {error}"))?;
    }
    Ok(())
}

#[tauri::command]
pub fn native_onnx_capabilities() -> Result<open_quartz::onnx::NativeOnnxCapabilities, String> {
    open_quartz::onnx::native_onnx_capabilities()
}

#[tauri::command]
pub async fn native_onnx_load_model(
    app: AppHandle,
    node_id: String,
    model_id: String,
    prefer_direct_ml: bool,
    state: State<'_, NativeOnnxState>,
) -> Result<open_quartz::onnx::OnnxSessionInfo, String> {
    let model_path = crate::models_dir(&app)?.join(format!("{model_id}.onnx"));
    let model = tokio::fs::read(&model_path)
        .await
        .map_err(|error| format!("Cannot read ONNX model {}: {error}", model_path.display()))?;
    let provider = if prefer_direct_ml && cfg!(target_os = "windows") {
        open_quartz::onnx::NativeOnnxProvider::DirectMl
    } else {
        open_quartz::onnx::NativeOnnxProvider::Cpu
    };
    let session = open_quartz::onnx::OnnxSession::from_memory_with_options(
        &model,
        open_quartz::onnx::NativeOnnxOptions {
            provider,
            allow_cpu_fallback: true,
        },
    )?;
    let info = session.info().clone();
    state
        .sessions
        .lock()
        .map_err(|_| "Native ONNX session lock is poisoned".to_owned())?
        .insert(node_id, session);
    Ok(info)
}

#[tauri::command]
pub fn native_onnx_unload_model(
    node_id: String,
    state: State<'_, NativeOnnxState>,
) -> Result<(), String> {
    state
        .sessions
        .lock()
        .map_err(|_| "Native ONNX session lock is poisoned".to_owned())?
        .remove(&node_id);
    Ok(())
}

fn smoke_native_onnx() -> Result<(String, f32), String> {
    let capabilities = open_quartz::onnx::native_onnx_capabilities()?;
    let provider = if capabilities.direct_ml {
        open_quartz::onnx::NativeOnnxProvider::DirectMl
    } else {
        open_quartz::onnx::NativeOnnxProvider::Cpu
    };
    let mut session = open_quartz::onnx::OnnxSession::from_memory_with_options(
        include_bytes!("../../crates/open_quartz/tests/data/identity.onnx"),
        open_quartz::onnx::NativeOnnxOptions {
            provider,
            allow_cpu_fallback: false,
        },
    )?;
    let backend = session.info().backend.clone();
    let output = session.run_f32(vec![7.0], vec![1])?;
    Ok((backend, output.data[0]))
}

fn smoke_native_video(runtime: &mut NativeGpuRuntime, image_graph: &str) -> Result<bool, String> {
    let ffmpeg = find_ffmpeg()?;
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("open-quartz-smoke-video-{suffix}.mp4"));
    let status = std::process::Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=16x16:r=10",
            "-t",
            "0.4",
            "-pix_fmt",
            "yuv420p",
            "-y",
        ])
        .arg(&path)
        .status()
        .map_err(|error| format!("Cannot create native video smoke fixture: {error}"))?;
    if !status.success() {
        return Err("FFmpeg could not create native video smoke fixture".to_owned());
    }

    let result = (|| {
        runtime.remove_texture("image");
        let video_graph =
            image_graph.replacen("\"inputMode\": \"image\"", "\"inputMode\": \"video\"", 1);
        runtime.set_graph(&video_graph)?;
        runtime.attach_video(
            "image",
            NativeVideoConfig {
                kind: NativeVideoSourceKind::File,
                source: path.to_string_lossy().into_owned(),
                looping: true,
                playback_rate: 1.0,
            },
        )?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while runtime.executor.output_texture("image").is_none() && Instant::now() < deadline {
            runtime.upload_video_frames()?;
            std::thread::sleep(Duration::from_millis(20));
        }
        if runtime.executor.output_texture("image").is_none() {
            return Err("Native video decoder produced no GPU frame".to_owned());
        }
        runtime.render_next()?;
        let readback = runtime.read_output("renderer")?;
        Ok(readback.get(8..12).is_some_and(|pixel| {
            pixel[0] > 240 && pixel[1] < 16 && pixel[2] < 16 && pixel[3] == 255
        }))
    })();
    let _ = runtime.detach_video("image");
    let _ = std::fs::remove_file(path);
    result
}

pub fn maybe_start_smoke(app: &AppHandle) {
    if std::env::var_os("OPEN_QUARTZ_NATIVE_SMOKE").is_none() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<NativeRuntimeState>();
        let graph = r#"{
            "nodes": [
                {
                    "id": "image", "type": "input", "position": {"x": 0.0, "y": 0.0},
                    "data": {
                        "type": "input", "label": "Image", "shaderCode": "",
                        "inputs": [],
                        "outputs": [{"id": "image_out", "label": "output", "dataType": "sampler2D", "direction": "output"}],
                        "uniforms": {}, "inputMode": "image", "inputDataType": "sampler2D",
                        "imageWidth": 2, "imageHeight": 2
                    }
                },
                {
                    "id": "copy", "type": "shader", "position": {"x": 1.0, "y": 0.0},
                    "data": {
                        "type": "shader", "label": "Copy",
                        "shaderCode": "@group(0) @binding(0) var inputImage: texture_2d<f32>; @group(0) @binding(1) var inputImageSampler: sampler; @fragment fn main(@location(0) uv: vec2f) -> @location(0) vec4f { return textureSample(inputImage, inputImageSampler, uv); }",
                        "inputs": [{"id": "copy_in", "label": "inputImage", "dataType": "sampler2D", "direction": "input"}],
                        "outputs": [{"id": "copy_out", "label": "output", "dataType": "sampler2D", "direction": "output"}],
                        "uniforms": {}, "autoSize": false, "width": 2, "height": 2
                    }
                },
                {
                    "id": "renderer", "type": "renderer", "position": {"x": 2.0, "y": 0.0},
                    "data": {
                        "type": "renderer", "label": "Output", "shaderCode": "",
                        "inputs": [{"id": "renderer_in", "label": "inputImage", "dataType": "sampler2D", "direction": "input"}],
                        "outputs": [], "uniforms": {}
                    }
                }
            ],
            "edges": [
                {"id": "e1", "source": "image", "sourceHandle": "image_out", "target": "copy", "targetHandle": "copy_in"},
                {"id": "e2", "source": "copy", "sourceHandle": "copy_out", "target": "renderer", "targetHandle": "renderer_in"}
            ]
        }"#;
        let result = async {
            let info = initialize_runtime(&app, &state).await?;
            let (frame, readback_ok) = with_runtime(&state, |runtime| {
                runtime.set_graph(graph)?;
                let pixels = [
                    255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
                ];
                runtime.upload_image("image", &pixels, 2, 2)?;
                let frame = runtime.render_next()?;
                let readback = runtime.read_output("renderer")?;
                Ok((frame, readback.get(8..) == Some(pixels.as_slice())))
            })?;
            let video_ok = with_runtime(&state, |runtime| smoke_native_video(runtime, graph))?;
            let onnx = smoke_native_onnx()?;
            Ok::<_, String>((info, frame, onnx, readback_ok, video_ok))
        }
        .await;
        match result {
            Ok((info, frame, (onnx_backend, onnx_output), readback_ok, video_ok)) => {
                if !readback_ok || !video_ok {
                    eprintln!(
                        "NATIVE_GPU_SMOKE_ERROR resource mismatch image={readback_ok} video={video_ok}"
                    );
                    let _ = close_runtime(&state);
                    app.exit(1);
                    return;
                }
                println!(
                    "NATIVE_GPU_SMOKE_OK adapter={} backend={} frame={} output={} size={}x{} image_readback=true video_readback=true onnx={} onnx_output={}",
                    info.adapter_name,
                    info.backend,
                    frame.frame,
                    frame.output_node_id,
                    frame.width,
                    frame.height,
                    onnx_backend,
                    onnx_output
                );
                let _ = close_runtime(&state);
                app.exit(0);
            }
            Err(error) => {
                eprintln!("NATIVE_GPU_SMOKE_ERROR {error}");
                let _ = close_runtime(&state);
                app.exit(1);
            }
        }
    });
}

fn with_runtime<T>(
    state: &NativeRuntimeState,
    operation: impl FnOnce(&mut NativeGpuRuntime) -> Result<T, String>,
) -> Result<T, String> {
    let mut guard = state
        .runtime
        .lock()
        .map_err(|_| "Native runtime lock is poisoned".to_owned())?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| "Native GPU runtime is not initialized".to_owned())?;
    operation(runtime)
}

fn start_worker(app: &AppHandle, state: &NativeRuntimeState) -> Result<(), String> {
    let runtime = state.runtime.clone();
    let alive = state.alive.clone();
    let playing = state.playing.clone();
    let app = app.clone();
    let worker = std::thread::Builder::new()
        .name("open-quartz-native-render".to_owned())
        .spawn(move || {
            while alive.load(Ordering::Acquire) {
                let tick_started = Instant::now();
                if playing.load(Ordering::Acquire) {
                    let result = runtime
                        .lock()
                        .map_err(|_| "Native runtime lock is poisoned".to_owned())
                        .and_then(|mut guard| {
                            guard
                                .as_mut()
                                .ok_or_else(|| "Native GPU runtime is not initialized".to_owned())?
                                .render_next()
                        });
                    match result {
                        Ok(frame) if frame.frame % 6 == 0 => {
                            let _ = app.emit("native-runtime-frame", frame);
                        }
                        Ok(_) => {}
                        Err(error) => {
                            playing.store(false, Ordering::Release);
                            let _ = app.emit("native-runtime-error", error);
                        }
                    }
                }
                let elapsed = tick_started.elapsed();
                if elapsed < Duration::from_millis(16) {
                    std::thread::sleep(Duration::from_millis(16) - elapsed);
                }
            }
        })
        .map_err(|error| format!("Cannot start native render thread: {error}"))?;
    *state
        .worker
        .lock()
        .map_err(|_| "Native worker lock is poisoned".to_owned())? = Some(worker);
    Ok(())
}

fn shutdown_worker(state: &NativeRuntimeState) {
    state.playing.store(false, Ordering::Release);
    state.alive.store(false, Ordering::Release);
    if let Ok(mut worker) = state.worker.lock() {
        if let Some(worker) = worker.take() {
            let _ = worker.join();
        }
    }
}

fn utc_date_uniform(time: SystemTime) -> [f32; 4] {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = (seconds / 86_400) as i64;
    let seconds_of_day = (seconds % 86_400) as f32;
    let (year, month, day) = civil_from_days(days);
    [year as f32, month as f32, day as f32, seconds_of_day]
}

fn civil_from_days(days_since_epoch: i64) -> (i32, u32, u32) {
    let days = days_since_epoch + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year as i32, month as u32, day as u32)
}

fn native_backends() -> wgpu::Backends {
    #[cfg(target_os = "windows")]
    return wgpu::Backends::DX12;
    #[cfg(target_os = "macos")]
    return wgpu::Backends::METAL;
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    return wgpu::Backends::VULKAN;
}

#[cfg(test)]
mod tests {
    use super::{civil_from_days, utc_date_uniform};
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn converts_unix_time_to_wgsl_date_uniform() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(
            utc_date_uniform(UNIX_EPOCH + Duration::from_secs(86_400)),
            [1970.0, 1.0, 2.0, 0.0]
        );
    }
}
