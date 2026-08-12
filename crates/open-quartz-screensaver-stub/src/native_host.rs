use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use open_quartz::gpu::{GpuBackend, GpuExecutor, BLIT_FRAG, FULLSCREEN_VERT_WITH_UV};
use open_quartz::native_video::{
    NativeVideoConfig, NativeVideoFrame, NativeVideoSource, NativeVideoSourceKind,
};
use open_quartz::onnx::{
    run_native_image_task, NativeOnnxOptions, NativeOnnxProvider, OnnxSession, OnnxTask,
};
use open_quartz::runtime::{DataPathMode, Runtime, RuntimeCapabilities, RuntimeFrameInput};
use open_quartz::types::{Graph, InputMode, NodeType, OnnxParamValue, VideoSourceType};
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle,
};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
    GetSystemMetrics, PeekMessageW, PostQuitMessage, RegisterClassW, ShowWindow, TranslateMessage,
    CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, HMENU, MSG, PM_REMOVE, SM_CXSCREEN, SM_CYSCREEN,
    SW_SHOW, WINDOW_EX_STYLE, WM_CLOSE, WM_DESTROY, WM_KEYDOWN, WM_LBUTTONDOWN, WM_NCCREATE,
    WM_QUIT, WNDCLASSW, WS_CHILD, WS_POPUP, WS_VISIBLE,
};
use windows_core::PCWSTR;

static EXIT_REQUESTED: AtomicBool = AtomicBool::new(false);
pub fn run(
    project_json: &str,
    renderer_node_id: &str,
    parent: Option<HWND>,
    resource_overrides: &HashMap<String, String>,
    packaged_resources: &HashMap<String, String>,
) -> Result<(), String> {
    EXIT_REQUESTED.store(false, Ordering::Release);
    let graph = parse_graph(project_json)?;
    validate_graph(&graph)?;
    let window = create_window(parent)?;
    let mut renderer = NativeRenderer::new(
        window,
        graph,
        renderer_node_id,
        resource_overrides,
        packaged_resources,
    )?;
    unsafe {
        let _ = ShowWindow(window, SW_SHOW);
    };

    let mut message = MSG::default();
    let mut next_frame = Instant::now();
    loop {
        while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
            if message.message == WM_QUIT {
                return Ok(());
            }
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        if EXIT_REQUESTED.load(Ordering::Acquire) {
            unsafe {
                let _ = DestroyWindow(window);
            }
            return Ok(());
        }
        let now = Instant::now();
        if now >= next_frame {
            renderer.render()?;
            next_frame = now + Duration::from_millis(16);
        } else {
            std::thread::sleep((next_frame - now).min(Duration::from_millis(2)));
        }
    }
}

struct NativeRenderer {
    runtime: Runtime,
    executor: GpuExecutor,
    renderer_node_id: String,
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    present_bind_group_layout: wgpu::BindGroupLayout,
    present_pipeline: wgpu::RenderPipeline,
    videos: HashMap<String, NativeVideoSource>,
    onnx: HashMap<String, OnnxResource>,
    started_at: Instant,
}

struct OnnxResource {
    session: OnnxSession,
    task: OnnxTask,
    model_id: String,
    target_size: u32,
    score_threshold: f32,
    iou_threshold: f32,
}

impl NativeRenderer {
    fn new(
        window: HWND,
        graph: Graph,
        renderer_node_id: &str,
        resource_overrides: &HashMap<String, String>,
        packaged_resources: &HashMap<String, String>,
    ) -> Result<Self, String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::DX12,
            ..Default::default()
        });
        let raw_window = Win32WindowHandle::new(
            std::num::NonZeroIsize::new(window.0 as isize)
                .ok_or_else(|| "Screen saver window handle is null".to_owned())?,
        );
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: RawDisplayHandle::Windows(WindowsDisplayHandle::new()),
                raw_window_handle: RawWindowHandle::Win32(raw_window),
            })
        }
        .map_err(|error| format!("Cannot create screen saver GPU surface: {error}"))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
        }))
        .map_err(|error| format!("Cannot create screen saver GPU adapter: {error}"))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("OpenQuartz Screen Saver"),
            ..Default::default()
        }))
        .map_err(|error| format!("Cannot create screen saver GPU device: {error}"))?;
        let mut rect = RECT::default();
        unsafe { GetClientRect(window, &mut rect) }
            .map_err(|error| format!("Cannot read screen saver window size: {error}"))?;
        let surface_config = surface
            .get_default_config(
                &adapter,
                (rect.right - rect.left).max(1) as u32,
                (rect.bottom - rect.top).max(1) as u32,
            )
            .ok_or_else(|| {
                "Screen saver surface is incompatible with the GPU adapter".to_owned()
            })?;
        surface.configure(&device, &surface_config);
        let backend = Arc::new(GpuBackend::from_device(device, queue));
        let (present_bind_group_layout, present_pipeline) =
            create_present_pipeline(&backend, surface_config.format);
        let mut executor = GpuExecutor::new(backend);
        let mut runtime = Runtime::new_native(RuntimeCapabilities {
            data_paths: vec![DataPathMode::CpuCopy],
        });
        runtime.set_graph(&graph).map_err(|error| error.to_json())?;
        let plan = runtime
            .execution_plan()
            .ok_or_else(|| "Screen saver graph did not produce an execution plan".to_owned())?;
        executor
            .sync_plan(plan)
            .map_err(|error| error.to_string())?;
        upload_images(&graph, resource_overrides, &mut executor)?;
        let videos = open_videos(&graph, resource_overrides, packaged_resources)?;
        let onnx = load_onnx_resources(&graph, packaged_resources)?;
        runtime
            .set_video_nodes(&videos.keys().cloned().collect::<Vec<_>>())
            .map_err(|error| error.to_json())?;
        runtime.play(0).map_err(|error| error.to_json())?;
        Ok(Self {
            runtime,
            executor,
            renderer_node_id: renderer_node_id.to_owned(),
            surface,
            surface_config,
            present_bind_group_layout,
            present_pipeline,
            started_at: Instant::now(),
            videos,
            onnx,
        })
    }

    fn upload_video_frames(&mut self) -> Result<(), String> {
        let mut dirty = Vec::new();
        for (node_id, source) in &mut self.videos {
            if source.upload_latest(|frame, width, height| match frame {
                NativeVideoFrame::Rgba(rgba) => self
                    .executor
                    .upload_rgba(node_id, rgba, width, height)
                    .map_err(|error| error.to_string()),
            })? {
                dirty.push(node_id.clone());
            }
        }
        for node_id in dirty {
            self.runtime
                .mark_dirty(&node_id)
                .map_err(|error| error.to_json())?;
        }
        Ok(())
    }

    fn execute_commands(
        &mut self,
        commands: &[open_quartz::engine::ExecutionCommand],
    ) -> Result<(), String> {
        let mut batch = Vec::new();
        for command in commands {
            if command.kind == "onnx" {
                if !batch.is_empty() {
                    self.runtime
                        .execute_gpu(&mut self.executor, &batch)
                        .map_err(|error| error.to_json())?;
                    batch.clear();
                }
                let Some(source_id) = command.texture_inputs.values().next() else {
                    continue;
                };
                let Some(resource) = self.onnx.get_mut(&command.node_id) else {
                    continue;
                };
                let source = self
                    .executor
                    .output_handle(source_id)
                    .ok_or_else(|| format!("ONNX source {source_id} has no GPU output"))?;
                let rgba = pollster::block_on(self.executor.read_output_rgba(source_id))
                    .map_err(|error| error.to_string())?;
                let output = run_native_image_task(
                    &mut resource.session,
                    resource.task,
                    &resource.model_id,
                    &rgba,
                    source.width,
                    source.height,
                    resource.target_size,
                    resource.score_threshold,
                    resource.iou_threshold,
                )?;
                self.executor
                    .upload_rgba(&command.node_id, &output.rgba, output.width, output.height)
                    .map_err(|error| error.to_string())?;
                self.runtime
                    .mark_dirty(&command.node_id)
                    .map_err(|error| error.to_json())?;
                continue;
            }
            if command
                .texture_inputs
                .values()
                .all(|source| self.executor.output_texture(source).is_some())
            {
                batch.push(command.clone());
            }
        }
        if !batch.is_empty() {
            self.runtime
                .execute_gpu(&mut self.executor, &batch)
                .map_err(|error| error.to_json())?;
        }
        Ok(())
    }

    fn render(&mut self) -> Result<(), String> {
        self.upload_video_frames()?;
        let now_ns = self
            .started_at
            .elapsed()
            .as_nanos()
            .min(u128::from(u64::MAX)) as u64;
        let (width, height) = self
            .runtime
            .execution_plan()
            .map(|plan| (plan.default_width.max(1), plan.default_height.max(1)))
            .unwrap_or((960, 540));
        self.runtime
            .advance(&RuntimeFrameInput {
                now_ns,
                date: date_uniform(SystemTime::now()),
                mouse: [0.0; 4],
                resolution: [width as f32, height as f32, 1.0],
            })
            .map_err(|error| error.to_json())?;
        let commands = self.runtime.drain_commands();
        self.execute_commands(&commands)?;
        let output = self
            .executor
            .output_handle(&self.renderer_node_id)
            .ok_or_else(|| "Screen saver renderer has no GPU output".to_owned())?;
        let frame = match self.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(
                    self.executor.backend().device.as_ref(),
                    &self.surface_config,
                );
                self.surface
                    .get_current_texture()
                    .map_err(|error| format!("Cannot acquire screen saver surface: {error}"))?
            }
            Err(error) => return Err(format!("Cannot acquire screen saver surface: {error}")),
        };
        let bind_group =
            self.executor
                .backend()
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("open-quartz-screen-saver-present-bind-group"),
                    layout: &self.present_bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&output.view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&output.sampler),
                        },
                    ],
                });
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.executor.backend().device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor {
                label: Some("open-quartz-screen-saver-present"),
            },
        );
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("open-quartz-screen-saver-present-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
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
            pass.set_pipeline(&self.present_pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.executor.backend().queue.submit([encoder.finish()]);
        frame.present();
        Ok(())
    }
}

fn create_present_pipeline(
    backend: &GpuBackend,
    surface_format: wgpu::TextureFormat,
) -> (wgpu::BindGroupLayout, wgpu::RenderPipeline) {
    let bind_group_layout =
        backend
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("open-quartz-screen-saver-present-bindings"),
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
    let layout = backend
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("open-quartz-screen-saver-present-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
    let vertex = backend
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("open-quartz-screen-saver-present-vertex"),
            source: wgpu::ShaderSource::Wgsl(FULLSCREEN_VERT_WITH_UV.into()),
        });
    let fragment = backend
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("open-quartz-screen-saver-present-fragment"),
            source: wgpu::ShaderSource::Wgsl(BLIT_FRAG.into()),
        });
    let pipeline = backend
        .device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("open-quartz-screen-saver-present-pipeline"),
            layout: Some(&layout),
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
    (bind_group_layout, pipeline)
}

fn parse_graph(project_json: &str) -> Result<Graph, String> {
    let value: serde_json::Value = serde_json::from_str(project_json)
        .map_err(|error| format!("Invalid project JSON: {error}"))?;
    let graph = value.get("graph").unwrap_or(&value);
    serde_json::from_value(graph.clone()).map_err(|error| format!("Invalid project graph: {error}"))
}

fn validate_graph(graph: &Graph) -> Result<(), String> {
    if graph.nodes.iter().any(|node| {
        node.node_type == NodeType::Input
            && node.data.input_mode == Some(InputMode::Video)
            && node.data.video_source_type == Some(VideoSourceType::Camera)
    }) {
        return Err("Camera inputs are not supported by exported screen savers".to_owned());
    }
    Ok(())
}

fn open_videos(
    graph: &Graph,
    resource_overrides: &HashMap<String, String>,
    packaged_resources: &HashMap<String, String>,
) -> Result<HashMap<String, NativeVideoSource>, String> {
    let mut videos = HashMap::new();
    for node in &graph.nodes {
        if node.node_type != NodeType::Input || node.data.input_mode != Some(InputMode::Video) {
            continue;
        }
        let source = resource_overrides
            .get(&node.id)
            .or_else(|| packaged_resources.get(&node.id))
            .or(node.data.video_file_path.as_ref())
            .ok_or_else(|| format!("Video node '{}' has no source", node.data.label))?
            .clone();
        let video = NativeVideoSource::open(NativeVideoConfig {
            kind: NativeVideoSourceKind::File,
            source,
            looping: node.data.video_loop.unwrap_or(true),
            playback_rate: node.data.video_playback_rate.unwrap_or(1.0),
        })?;
        videos.insert(node.id.clone(), video);
    }
    Ok(videos)
}

fn load_onnx_resources(
    graph: &Graph,
    packaged_resources: &HashMap<String, String>,
) -> Result<HashMap<String, OnnxResource>, String> {
    let mut resources = HashMap::new();
    for node in &graph.nodes {
        if node.node_type != NodeType::Onnx {
            continue;
        }
        let model_path = packaged_resources
            .get(&node.id)
            .ok_or_else(|| format!("ONNX node '{}' has no packaged model", node.data.label))?;
        let model = std::fs::read(model_path)
            .map_err(|error| format!("Cannot read ONNX model {model_path}: {error}"))?;
        let session = OnnxSession::from_memory_with_options(
            &model,
            NativeOnnxOptions {
                provider: NativeOnnxProvider::DirectMl,
                allow_cpu_fallback: true,
            },
        )?;
        let model_id = node
            .data
            .onnx_model_id
            .clone()
            .or_else(|| node.data.onnx_catalog_id.clone())
            .unwrap_or_else(|| node.id.clone());
        let task = packaged_resources
            .get(&format!("{}:task", node.id))
            .and_then(|task| serde_json::from_value(serde_json::Value::String(task.clone())).ok())
            .unwrap_or(OnnxTask::Generic);
        let number = |key: &str| {
            node.data
                .onnx_params
                .as_ref()
                .and_then(|params| params.get(key))
                .and_then(|value| match value {
                    OnnxParamValue::Number(value) => Some(*value),
                    OnnxParamValue::Boolean(_) => None,
                })
        };
        resources.insert(
            node.id.clone(),
            OnnxResource {
                session,
                task,
                model_id,
                target_size: number("targetSize")
                    .map(|value| value as u32)
                    .or(node.data.onnx_target_size)
                    .unwrap_or(640),
                score_threshold: number("scoreThreshold")
                    .or(node.data.onnx_score_threshold)
                    .unwrap_or(0.25) as f32,
                iou_threshold: number("iouThreshold")
                    .or(node.data.onnx_iou_threshold)
                    .unwrap_or(0.45) as f32,
            },
        );
    }
    Ok(resources)
}

fn upload_images(
    graph: &Graph,
    resource_overrides: &HashMap<String, String>,
    executor: &mut GpuExecutor,
) -> Result<(), String> {
    for node in &graph.nodes {
        if node.node_type != NodeType::Input || node.data.input_mode != Some(InputMode::Image) {
            continue;
        }
        let image = if let Some(path) = resource_overrides.get(&node.id) {
            image::open(path).map_err(|error| format!("Cannot load {}: {error}", path))?
        } else if let Some(data_url) = &node.data.image_data_url {
            let encoded = data_url
                .split_once(',')
                .map(|(_, payload)| payload)
                .ok_or_else(|| {
                    format!("Image node '{}' has an invalid data URL", node.data.label)
                })?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|error| {
                    format!("Cannot decode image node '{}': {error}", node.data.label)
                })?;
            image::load_from_memory(&bytes).map_err(|error| {
                format!("Cannot decode image node '{}': {error}", node.data.label)
            })?
        } else {
            continue;
        };
        let rgba = image.to_rgba8();
        executor
            .upload_rgba(&node.id, rgba.as_raw(), rgba.width(), rgba.height())
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn create_window(parent: Option<HWND>) -> Result<HWND, String> {
    let instance = unsafe { GetModuleHandleW(None) }
        .map_err(|error| format!("Cannot resolve screen saver module: {error}"))?;
    let class_name = wide("OpenQuartzScreenSaverHost");
    let window_class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(window_proc),
        hInstance: instance.into(),
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..Default::default()
    };
    if unsafe { RegisterClassW(&window_class) } == 0 {
        return Err("Cannot register screen saver window class".to_owned());
    }
    let (style, x, y, width, height, parent_window) = if let Some(parent) = parent {
        let mut rect = RECT::default();
        unsafe { GetClientRect(parent, &mut rect) }
            .map_err(|error| format!("Cannot read preview window size: {error}"))?;
        (
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            (rect.right - rect.left).max(1),
            (rect.bottom - rect.top).max(1),
            Some(parent),
        )
    } else {
        (
            WS_POPUP | WS_VISIBLE,
            0,
            0,
            unsafe { GetSystemMetrics(SM_CXSCREEN) },
            unsafe { GetSystemMetrics(SM_CYSCREEN) },
            None,
        )
    };
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(wide("OpenQuartz Screen Saver").as_ptr()),
            style,
            if parent.is_some() { 0 } else { x },
            if parent.is_some() { 0 } else { y },
            width,
            height,
            parent_window,
            None::<HMENU>,
            Some(instance.into()),
            None,
        )
    }
    .map_err(|error| format!("Cannot create screen saver window: {error}"))
}

extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCCREATE => {
            let _ = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
            LRESULT(1)
        }
        WM_KEYDOWN | WM_LBUTTONDOWN | WM_CLOSE => {
            EXIT_REQUESTED.store(true, Ordering::Release);
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

fn date_uniform(now: SystemTime) -> [f32; 4] {
    let seconds = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
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

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_unix_days_for_shader_date_uniforms() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_000), (2024, 10, 4));
    }
}
