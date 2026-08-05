use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::native_video::{
    find_ffmpeg, list_video_devices, NativeVideoConfig, NativeVideoDevice, NativeVideoFrame,
    NativeVideoInfo, NativeVideoSource, NativeVideoSourceKind,
};
use open_quartz::engine::ExecutionCommand;
#[cfg(windows)]
use open_quartz::gpu::{DxgiSharedTextureExporter, SharedTexturePresenter};
use open_quartz::gpu::{
    GpuBackend, GpuExecutor, GpuOutputHandle, GpuPresentationFrame, GpuPresenter, GpuPreviewReader,
    SharedTextureFrame, TextureFormat,
};
use open_quartz::onnx::{NativeOnnxImageOutput, OnnxSession, OnnxTask};
use open_quartz::Engine;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Default)]
pub struct NativeRuntimeState {
    runtime: Arc<Mutex<Option<NativeGpuRuntime>>>,
    preview: Mutex<Option<GpuPreviewReader>>,
    alive: Arc<AtomicBool>,
    playing: Arc<AtomicBool>,
    worker: Mutex<Option<JoinHandle<()>>>,
    #[cfg(windows)]
    presentation_scheduled: Arc<AtomicBool>,
}

impl Drop for NativeRuntimeState {
    fn drop(&mut self) {
        shutdown_worker(self);
    }
}

#[derive(Clone)]
struct NativeOnnxConfig {
    model_id: String,
    task: OnnxTask,
    target_size: u32,
    score_threshold: f32,
    iou_threshold: f32,
}

struct NativeOnnxResource {
    session: Arc<Mutex<OnnxSession>>,
    config: NativeOnnxConfig,
    backend: String,
}

struct NativeOnnxCompletion {
    node_id: String,
    revision: u32,
    generation: u32,
    result: Result<NativeOnnxImageOutput, String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeOutputEvent {
    node_id: String,
    width: u32,
    height: u32,
    backend: String,
    data: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeOnnxLoadOptions {
    model_path: Option<String>,
    task: OnnxTask,
    target_size: u32,
    score_threshold: f32,
    iou_threshold: f32,
    prefer_direct_ml: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeRuntimeInfo {
    adapter_name: String,
    backend: String,
    device_type: String,
    output_mode: String,
    native_onnx_cpu: bool,
    native_onnx_direct_ml: bool,
    shared_onnx_wgpu_device: bool,
    native_video: bool,
    video_data_path: String,
    tensor_data_path: String,
    shared_texture: bool,
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
    executor: GpuExecutor,
    engine: Engine,
    output_node_id: Option<String>,
    started_at: Instant,
    previous_frame_at: Instant,
    frame: u64,
    mouse: [f32; 4],
    videos: HashMap<String, NativeVideoSource>,
    onnx_resources: HashMap<String, NativeOnnxResource>,
    onnx_pending: HashSet<String>,
    onnx_sender: mpsc::Sender<NativeOnnxCompletion>,
    onnx_receiver: mpsc::Receiver<NativeOnnxCompletion>,
    output_events: Vec<NativeOutputEvent>,
    onnx_workers: Vec<JoinHandle<()>>,
    #[cfg(windows)]
    shared_presenter: Option<SharedTexturePresenter<DxgiSharedTextureExporter>>,
    #[cfg(windows)]
    shared_texture_enabled: bool,
}

impl NativeGpuRuntime {
    async fn new() -> Result<(Self, NativeRuntimeInfo), String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: native_backends(),
            ..Default::default()
        });
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
            })
            .await
            .map_err(|error| format!("Cannot find a native GPU adapter: {error}"))?;
        let adapter_info = adapter.get_info();
        let required_features = if adapter
            .features()
            .contains(wgpu::Features::TEXTURE_FORMAT_P010)
            && adapter
                .features()
                .contains(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM)
        {
            wgpu::Features::TEXTURE_FORMAT_P010 | wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
        } else {
            wgpu::Features::empty()
        };
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_features,
                ..Default::default()
            })
            .await
            .map_err(|error| format!("Cannot create native GPU device: {error}"))?;
        let backend = Arc::new(GpuBackend::from_device(device, queue));
        let executor = GpuExecutor::new(backend.clone());
        #[cfg(windows)]
        let shared_presenter = DxgiSharedTextureExporter::new(backend.clone())
            .ok()
            .map(SharedTexturePresenter::new);
        #[cfg(windows)]
        let shared_texture = shared_presenter.is_some();
        #[cfg(not(windows))]
        let shared_texture = false;
        let info = NativeRuntimeInfo {
            adapter_name: adapter_info.name,
            backend: format!("{:?}", adapter_info.backend),
            device_type: format!("{:?}", adapter_info.device_type),
            output_mode: "embedded-readback".to_owned(),
            native_onnx_cpu: true,
            native_video: find_ffmpeg().is_ok(),
            native_onnx_direct_ml: cfg!(target_os = "windows"),
            shared_onnx_wgpu_device: false,
            video_data_path: if cfg!(windows)
                && required_features.contains(wgpu::Features::TEXTURE_FORMAT_P010)
                && required_features.contains(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM)
            {
                "d3d12va-p010-zero-copy".to_owned()
            } else {
                "cpu-copy".to_owned()
            },
            tensor_data_path: "cpu-copy".to_owned(),
            shared_texture,
        };
        let (onnx_sender, onnx_receiver) = mpsc::channel();
        let now = Instant::now();
        Ok((
            Self {
                executor,
                engine: Engine::new_native(),
                output_node_id: None,
                started_at: now,
                previous_frame_at: now,
                frame: 0,
                mouse: [0.0; 4],
                videos: HashMap::new(),
                onnx_resources: HashMap::new(),
                onnx_pending: HashSet::new(),
                onnx_sender,
                onnx_receiver,
                output_events: Vec::new(),
                onnx_workers: Vec::new(),
                #[cfg(windows)]
                shared_presenter,
                #[cfg(windows)]
                shared_texture_enabled: false,
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
        self.onnx_resources
            .retain(|node_id, _| node_ids.contains(node_id));
        self.onnx_pending
            .retain(|node_id| node_ids.contains(node_id));
        self.output_events
            .retain(|event| node_ids.contains(&event.node_id));
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
        let (rgba, width, height) = self.read_output_rgba(node_id)?;
        Ok(Self::output_payload(&rgba, width, height))
    }

    fn preview_source(&self, node_id: &str) -> Result<GpuOutputHandle, String> {
        let output = self
            .executor
            .output_handle(node_id)
            .ok_or_else(|| format!("Output node {node_id} has no native GPU texture"))?;
        if output.format != TextureFormat::Rgba8Unorm {
            return Err(format!(
                "Output node {node_id} uses {:?}; native preview requires rgba8unorm",
                output.format
            ));
        }
        Ok(output)
    }

    fn read_output_rgba(&self, node_id: &str) -> Result<(Vec<u8>, u32, u32), String> {
        let output = self
            .executor
            .output_texture(node_id)
            .ok_or_else(|| format!("Output node {node_id} has no native GPU texture"))?;
        if output.format != TextureFormat::Rgba8Unorm {
            return Err(format!(
                "Output node {node_id} uses {:?}; native readback requires rgba8unorm",
                output.format
            ));
        }
        let rgba = pollster::block_on(self.executor.read_output_rgba(node_id))
            .map_err(|error| error.to_string())?;
        Ok((rgba, output.width, output.height))
    }

    fn output_payload(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
        let mut payload = Vec::with_capacity(8 + rgba.len());
        payload.extend_from_slice(&width.to_le_bytes());
        payload.extend_from_slice(&height.to_le_bytes());
        payload.extend_from_slice(rgba);
        payload
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
            let uploaded = source.upload_latest(|frame, width, height| match frame {
                NativeVideoFrame::Rgba(rgba) => executor
                    .upload_rgba(node_id, rgba, width, height)
                    .map_err(|error| error.to_string()),
                #[cfg(windows)]
                NativeVideoFrame::D3d12(frame) => executor
                    .upload_d3d12_p010(node_id, frame)
                    .map_err(|error| error.to_string()),
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

    fn start_playback(&mut self) -> Result<(), String> {
        self.resume_videos()?;
        let now = Instant::now();
        self.started_at = now;
        self.previous_frame_at = now;
        self.frame = 0;
        Ok(())
    }

    fn resume_videos(&mut self) -> Result<(), String> {
        for source in self.videos.values_mut() {
            source.resume()?;
        }
        Ok(())
    }

    fn load_onnx(
        &mut self,
        node_id: String,
        session: OnnxSession,
        config: NativeOnnxConfig,
    ) -> Result<open_quartz::onnx::OnnxSessionInfo, String> {
        self.engine.node_generation(&node_id)?;
        let info = session.info().clone();
        self.onnx_resources.insert(
            node_id.clone(),
            NativeOnnxResource {
                session: Arc::new(Mutex::new(session)),
                config,
                backend: info.backend.clone(),
            },
        );
        self.engine.mark_dirty(&node_id)?;
        Ok(info)
    }

    fn unload_onnx(&mut self, node_id: &str) {
        self.onnx_resources.remove(node_id);
        self.onnx_pending.remove(node_id);
        self.executor.remove_texture(node_id);
    }

    fn drain_onnx_completions(&mut self) -> Result<(), String> {
        let mut running = Vec::new();
        for worker in self.onnx_workers.drain(..) {
            if worker.is_finished() {
                let _ = worker.join();
            } else {
                running.push(worker);
            }
        }
        self.onnx_workers = running;
        while let Ok(completion) = self.onnx_receiver.try_recv() {
            self.onnx_pending.remove(&completion.node_id);
            let current_generation = self.engine.node_generation(&completion.node_id).ok();
            if completion.revision != self.engine.revision()
                || current_generation != Some(completion.generation)
            {
                continue;
            }
            let output = completion.result?;
            self.executor
                .upload_rgba(
                    &completion.node_id,
                    &output.rgba,
                    output.width,
                    output.height,
                )
                .map_err(|error| error.to_string())?;
            let backend = self
                .onnx_resources
                .get(&completion.node_id)
                .map(|resource| resource.backend.clone())
                .unwrap_or_else(|| "native".to_owned());
            self.output_events.push(NativeOutputEvent {
                node_id: completion.node_id.clone(),
                width: output.width,
                height: output.height,
                backend,
                data: output.data,
            });
            let downstream = self
                .engine
                .execution_plan()
                .map(|plan| {
                    plan.nodes
                        .iter()
                        .filter(|node| {
                            node.upstream
                                .values()
                                .any(|source| source == &completion.node_id)
                        })
                        .map(|node| node.id.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            for node_id in downstream {
                self.engine.mark_dirty(&node_id)?;
            }
        }
        Ok(())
    }

    fn schedule_onnx(&mut self, command: &ExecutionCommand) -> Result<(), String> {
        if self.onnx_pending.contains(&command.node_id) {
            return Ok(());
        }
        let Some(source_id) = command.texture_inputs.values().next() else {
            return Ok(());
        };
        let Some(source) = self.executor.output_texture(source_id) else {
            return Ok(());
        };
        let source_width = source.width;
        let source_height = source.height;
        let source_rgba = pollster::block_on(self.executor.read_output_rgba(source_id))
            .map_err(|error| error.to_string())?;
        let Some(resource) = self.onnx_resources.get(&command.node_id) else {
            return Ok(());
        };
        let session = resource.session.clone();
        let config = resource.config.clone();
        let sender = self.onnx_sender.clone();
        let node_id = command.node_id.clone();
        let revision = self.engine.revision();
        let generation = self.engine.node_generation(&node_id)?;
        self.onnx_pending.insert(node_id.clone());
        let worker = std::thread::Builder::new()
            .name(format!("open-quartz-onnx-{node_id}"))
            .spawn(move || {
                let result = session
                    .lock()
                    .map_err(|_| "Native ONNX session lock is poisoned".to_owned())
                    .and_then(|mut session| {
                        open_quartz::onnx::run_native_image_task(
                            &mut session,
                            config.task,
                            &config.model_id,
                            &source_rgba,
                            source_width,
                            source_height,
                            config.target_size,
                            config.score_threshold,
                            config.iou_threshold,
                        )
                    });
                let _ = sender.send(NativeOnnxCompletion {
                    node_id,
                    revision,
                    generation,
                    result,
                });
            })
            .map_err(|error| format!("Cannot start native ONNX task: {error}"))?;
        self.onnx_workers.push(worker);
        Ok(())
    }

    fn execute_runtime_commands(
        &mut self,
        plan: &open_quartz::engine::ExecutionPlan,
        commands: &[ExecutionCommand],
    ) -> Result<(), String> {
        let mut batch = Vec::new();
        for command in commands {
            if command.kind == "onnx" {
                if !batch.is_empty() {
                    self.executor
                        .execute_commands(plan, &batch)
                        .map_err(|error| error.to_string())?;
                    batch.clear();
                }
                self.schedule_onnx(command)?;
                continue;
            }
            let inputs_ready = command
                .texture_inputs
                .values()
                .all(|source_id| self.executor.output_texture(source_id).is_some());
            if inputs_ready {
                batch.push(command.clone());
            }
        }
        if !batch.is_empty() {
            self.executor
                .execute_commands(plan, &batch)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    #[cfg(windows)]
    fn set_shared_texture_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        let presenter = self
            .shared_presenter
            .as_mut()
            .ok_or_else(|| "DXGI shared texture Presenter is unavailable".to_owned())?;
        if !enabled {
            if let Some(frame) = presenter.take_latest() {
                presenter.release(frame.lease_id)?;
            }
        }
        self.shared_texture_enabled = enabled;
        Ok(enabled)
    }

    #[cfg(windows)]
    fn take_shared_texture(&mut self) -> Option<SharedTextureFrame> {
        self.shared_presenter
            .as_mut()
            .and_then(SharedTexturePresenter::take_latest)
    }

    #[cfg(windows)]
    fn has_shared_texture_pending(&self) -> bool {
        self.shared_presenter
            .as_ref()
            .and_then(SharedTexturePresenter::latest)
            .is_some()
    }

    #[cfg(windows)]
    fn release_shared_texture(&mut self, lease_id: u64) -> Result<(), String> {
        self.shared_presenter
            .as_mut()
            .ok_or_else(|| "DXGI shared texture Presenter is unavailable".to_owned())?
            .release(lease_id)
    }

    fn take_output_events(&mut self) -> Vec<NativeOutputEvent> {
        std::mem::take(&mut self.output_events)
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
        let (width, height) = self
            .engine
            .execution_plan()
            .map(|plan| (plan.default_width.max(1), plan.default_height.max(1)))
            .unwrap_or((960, 540));
        self.upload_video_frames()?;
        self.drain_onnx_completions()?;
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
            .ok_or_else(|| "Native engine has no execution plan".to_owned())?
            .clone();
        let commands = self.engine.pending_commands().to_vec();
        self.execute_runtime_commands(&plan, &commands)?;
        let output_node_id = self
            .output_node_id
            .clone()
            .or_else(|| plan.output_nodes.first().cloned())
            .ok_or_else(|| "Graph has no renderer or terminal texture output".to_owned())?;
        let Some(output) = self.executor.output_texture(&output_node_id) else {
            if !self.onnx_pending.is_empty() {
                return Ok(NativeFrameRendered {
                    frame,
                    revision: self.engine.revision(),
                    output_node_id,
                    width: 0,
                    height: 0,
                });
            }
            return Err(format!(
                "Output node {output_node_id} has no native GPU texture"
            ));
        };
        #[cfg(windows)]
        if self.shared_texture_enabled {
            if let (Some(presenter), Some(output)) = (
                self.shared_presenter.as_mut(),
                self.executor.output_handle(&output_node_id),
            ) {
                let _ = presenter.submit(GpuPresentationFrame {
                    node_id: output_node_id.clone(),
                    frame,
                    timeline_ns: (time.max(0.0) * 1_000_000_000.0) as u64,
                    output,
                });
                let _ = presenter.process_latest();
            }
        }
        Ok(NativeFrameRendered {
            frame,
            revision: self.engine.revision(),
            output_node_id,
            width: output.width,
            height: output.height,
        })
    }
}

impl Drop for NativeGpuRuntime {
    fn drop(&mut self) {
        for worker in self.onnx_workers.drain(..) {
            let _ = worker.join();
        }
    }
}

async fn initialize_runtime(
    app: &AppHandle,
    state: &NativeRuntimeState,
) -> Result<NativeRuntimeInfo, String> {
    shutdown_worker(state);
    state
        .preview
        .lock()
        .map_err(|_| "Native preview lock is poisoned".to_owned())?
        .take();
    state
        .runtime
        .lock()
        .map_err(|_| "Native runtime lock is poisoned".to_owned())?
        .take();
    let (mut runtime, mut info) = NativeGpuRuntime::new().await?;
    #[cfg(windows)]
    if app
        .state::<crate::webview_texture_stream::TextureStreamCapabilityState>()
        .get()
        .stream_ready
        && info.shared_texture
    {
        runtime.set_shared_texture_enabled(true)?;
        info.output_mode = "webview-texture-stream".to_owned();
    }
    let preview = GpuPreviewReader::new(runtime.executor.backend().clone());
    *state
        .preview
        .lock()
        .map_err(|_| "Native preview lock is poisoned".to_owned())? = Some(preview);
    state.alive.store(true, Ordering::Release);
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
pub async fn native_gpu_read_output(
    node_id: String,
    state: State<'_, NativeRuntimeState>,
) -> Result<tauri::ipc::Response, String> {
    let (backend, output) = with_runtime(&state, |runtime| {
        let output = runtime.preview_source(&node_id)?;
        Ok((runtime.executor.backend().clone(), output))
    })?;
    let rgba = backend
        .read_texture_rgba(&output.texture, output.width, output.height)
        .await?;
    Ok(tauri::ipc::Response::new(NativeGpuRuntime::output_payload(
        &rgba,
        output.width,
        output.height,
    )))
}

fn read_preview_payload(
    state: &NativeRuntimeState,
    node_id: &str,
    max_dimension: u32,
) -> Result<Vec<u8>, String> {
    let source = with_runtime(state, |runtime| runtime.preview_source(node_id))?;
    let mut preview = state
        .preview
        .lock()
        .map_err(|_| "Native preview lock is poisoned".to_owned())?;
    let reader = preview
        .as_mut()
        .ok_or_else(|| "Native preview reader is not initialized".to_owned())?;
    let image = pollster::block_on(reader.read(&source, max_dimension))?;
    Ok(NativeGpuRuntime::output_payload(
        &image.rgba,
        image.width,
        image.height,
    ))
}

#[tauri::command]
pub fn native_gpu_read_preview(
    node_id: String,
    max_dimension: u32,
    state: State<'_, NativeRuntimeState>,
) -> Result<tauri::ipc::Response, String> {
    let payload = read_preview_payload(&state, &node_id, max_dimension)?;
    Ok(tauri::ipc::Response::new(payload))
}

#[tauri::command]
pub fn native_gpu_set_shared_texture_enabled(
    enabled: bool,
    state: State<'_, NativeRuntimeState>,
) -> Result<bool, String> {
    #[cfg(windows)]
    {
        return with_runtime(&state, |runtime| {
            runtime.set_shared_texture_enabled(enabled)
        });
    }
    #[cfg(not(windows))]
    {
        let _ = (enabled, state);
        Err("Shared textures are not implemented on this platform".to_owned())
    }
}

#[tauri::command]
pub fn native_gpu_take_shared_texture(
    state: State<'_, NativeRuntimeState>,
) -> Result<Option<SharedTextureFrame>, String> {
    #[cfg(windows)]
    {
        return with_runtime(&state, |runtime| Ok(runtime.take_shared_texture()));
    }
    #[cfg(not(windows))]
    {
        let _ = state;
        Err("Shared textures are not implemented on this platform".to_owned())
    }
}

#[tauri::command]
pub fn native_gpu_release_shared_texture(
    lease_id: u64,
    state: State<'_, NativeRuntimeState>,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        return with_runtime(&state, |runtime| runtime.release_shared_texture(lease_id));
    }
    #[cfg(not(windows))]
    {
        let _ = (lease_id, state);
        Err("Shared textures are not implemented on this platform".to_owned())
    }
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
        runtime.start_playback()
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
pub fn native_gpu_video_metrics(
    node_id: String,
    state: State<'_, NativeRuntimeState>,
) -> Result<crate::native_video::NativeVideoMetrics, String> {
    with_runtime(&state, |runtime| {
        runtime
            .videos
            .get(&node_id)
            .map(NativeVideoSource::metrics)
            .ok_or_else(|| format!("Video node {node_id} is not attached"))
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
    state
        .preview
        .lock()
        .map_err(|_| "Native preview lock is poisoned".to_owned())?
        .take();
    state
        .runtime
        .lock()
        .map_err(|_| "Native runtime lock is poisoned".to_owned())?
        .take();
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
    options: NativeOnnxLoadOptions,
    state: State<'_, NativeRuntimeState>,
) -> Result<open_quartz::onnx::OnnxSessionInfo, String> {
    let model_path = options
        .model_path
        .map(std::path::PathBuf::from)
        .unwrap_or(crate::models_dir(&app)?.join(format!("{model_id}.onnx")));
    let model = tokio::fs::read(&model_path)
        .await
        .map_err(|error| format!("Cannot read ONNX model {}: {error}", model_path.display()))?;
    let provider = if options.prefer_direct_ml && cfg!(target_os = "windows") {
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
    let config = NativeOnnxConfig {
        model_id,
        task: options.task,
        target_size: options.target_size,
        score_threshold: options.score_threshold,
        iou_threshold: options.iou_threshold,
    };
    with_runtime(&state, move |runtime| {
        runtime.load_onnx(node_id, session, config)
    })
}

#[tauri::command]
pub fn native_onnx_unload_model(
    node_id: String,
    state: State<'_, NativeRuntimeState>,
) -> Result<(), String> {
    with_runtime(&state, |runtime| {
        runtime.unload_onnx(&node_id);
        Ok(())
    })
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

fn smoke_native_onnx_graph(runtime: &mut NativeGpuRuntime) -> Result<bool, String> {
    let graph = r#"{
        "nodes": [
            {
                "id": "image", "type": "input", "position": {"x": 0.0, "y": 0.0},
                "data": {
                    "type": "input", "label": "Image", "shaderCode": "", "inputs": [],
                    "outputs": [{"id": "image_out", "label": "output", "dataType": "sampler2D", "direction": "output"}],
                    "uniforms": {}, "inputMode": "image", "inputDataType": "sampler2D",
                    "imageWidth": 2, "imageHeight": 2
                }
            },
            {
                "id": "onnx", "type": "onnx", "position": {"x": 1.0, "y": 0.0},
                "data": {
                    "type": "onnx", "label": "Image identity", "shaderCode": "",
                    "inputs": [{"id": "onnx_in", "label": "image", "dataType": "sampler2D", "direction": "input"}],
                    "outputs": [{"id": "onnx_out", "label": "output", "dataType": "sampler2D", "direction": "output"}],
                    "uniforms": {}, "onnxCatalogId": "image-identity"
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
            {"id": "e1", "source": "image", "sourceHandle": "image_out", "target": "onnx", "targetHandle": "onnx_in"},
            {"id": "e2", "source": "onnx", "sourceHandle": "onnx_out", "target": "renderer", "targetHandle": "renderer_in"}
        ]
    }"#;
    runtime.set_graph(graph)?;
    let pixels = [
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
    ];
    runtime.upload_image("image", &pixels, 2, 2)?;
    let session = open_quartz::onnx::OnnxSession::from_memory(include_bytes!(
        "../../crates/open_quartz/tests/data/image_identity.onnx"
    ))?;
    runtime.load_onnx(
        "onnx".to_owned(),
        session,
        NativeOnnxConfig {
            model_id: "image-identity".to_owned(),
            task: open_quartz::onnx::OnnxTask::Generic,
            target_size: 2,
            score_threshold: 0.25,
            iou_threshold: 0.45,
        },
    )?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while runtime.executor.output_texture("onnx").is_none() && Instant::now() < deadline {
        runtime.render_next()?;
        std::thread::sleep(Duration::from_millis(10));
    }
    let readback = runtime.read_output("renderer")?;
    let events = runtime.take_output_events();
    Ok(readback.get(8..) == Some(pixels.as_slice())
        && events
            .iter()
            .any(|event| event.node_id == "onnx" && event.backend == "cpu"))
}

#[cfg(windows)]
fn smoke_shared_texture_presenter(runtime: &mut NativeGpuRuntime) -> Result<bool, String> {
    runtime.set_shared_texture_enabled(true)?;
    runtime.render_next()?;
    let Some(frame) = runtime.take_shared_texture() else {
        return Ok(false);
    };
    let valid = frame.platform == open_quartz::gpu::SharedTexturePlatform::Dxgi
        && frame.resource_handle != 0
        && frame.sync_handle.is_some()
        && frame.sync_value > 0
        && frame.width > 0
        && frame.height > 0;
    runtime.release_shared_texture(frame.lease_id)?;
    runtime.set_shared_texture_enabled(false)?;
    Ok(valid)
}

fn smoke_native_preview_latency(
    state: &NativeRuntimeState,
    image_graph: &str,
) -> Result<f64, String> {
    let graph = image_graph
        .replace(
            "\"imageWidth\": 2, \"imageHeight\": 2",
            "\"imageWidth\": 1920, \"imageHeight\": 1080",
        )
        .replace(
            "\"width\": 2, \"height\": 2",
            "\"width\": 1920, \"height\": 1080",
        );
    with_runtime(state, |runtime| {
        runtime.set_graph(&graph)?;
        let pixels = vec![255; 1920 * 1080 * 4];
        runtime.upload_image("image", &pixels, 1920, 1080)?;
        runtime.render_next()?;
        Ok(())
    })?;
    let started = Instant::now();
    let preview = read_preview_payload(state, "renderer", 960)?;
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    if preview.get(..8) != Some([192, 3, 0, 0, 28, 2, 0, 0].as_slice()) {
        return Err("Native 1080p preview did not scale to 960x540".to_owned());
    }
    Ok(elapsed_ms)
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
            let (frame, mut readback_ok) = with_runtime(&state, |runtime| {
                runtime.set_graph(graph)?;
                let pixels = [
                    255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
                ];
                runtime.upload_image("image", &pixels, 2, 2)?;
                let frame = runtime.render_next()?;
                let readback = runtime.read_output("renderer")?;
                Ok((frame, readback.get(8..) == Some(pixels.as_slice())))
            })?;
            let preview = read_preview_payload(&state, "renderer", 1)?;
            readback_ok &= preview.len() == 12 && preview[..8] == [1, 0, 0, 0, 1, 0, 0, 0];
            #[cfg(windows)]
            let shared_texture_ok = with_runtime(&state, smoke_shared_texture_presenter)?;
            #[cfg(not(windows))]
            let shared_texture_ok = true;
            let preview_ms = smoke_native_preview_latency(&state, graph)?;
            let video_ok = with_runtime(&state, |runtime| smoke_native_video(runtime, graph))?;
            let onnx_graph_ok = with_runtime(&state, smoke_native_onnx_graph)?;
            let onnx = smoke_native_onnx()?;
            Ok::<_, String>((
                info,
                frame,
                onnx,
                readback_ok,
                video_ok,
                onnx_graph_ok,
                shared_texture_ok,
                preview_ms,
            ))
        }
        .await;
        match result {
            Ok((
                info,
                frame,
                (onnx_backend, onnx_output),
                readback_ok,
                video_ok,
                onnx_graph_ok,
                shared_texture_ok,
                preview_ms,
            )) => {
                if !readback_ok || !video_ok || !onnx_graph_ok || !shared_texture_ok {
                    eprintln!(
                        "NATIVE_GPU_SMOKE_ERROR resource mismatch image={readback_ok} video={video_ok} onnx_graph={onnx_graph_ok} shared_texture={shared_texture_ok}"
                    );
                    let _ = close_runtime(&state);
                    app.exit(1);
                    return;
                }
                println!(
                    "NATIVE_GPU_SMOKE_OK adapter={} backend={} frame={} output={} size={}x{} image_readback=true video_readback=true onnx_graph=true shared_texture=true preview_1080p_ms={:.2} onnx={} onnx_output={}",
                    info.adapter_name,
                    info.backend,
                    frame.frame,
                    frame.output_node_id,
                    frame.width,
                    frame.height,
                    preview_ms,
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

#[cfg(windows)]
fn present_latest_shared_texture(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<NativeRuntimeState>();
    let Some(frame) = with_runtime(&state, |runtime| Ok(runtime.take_shared_texture()))? else {
        return Ok(());
    };
    let result = crate::webview_texture_stream::present_shared_frame(&frame);
    let release = with_runtime(&state, |runtime| {
        runtime.release_shared_texture(frame.lease_id)
    });
    result.and(release).map(|_| ())
}

fn start_worker(app: &AppHandle, state: &NativeRuntimeState) -> Result<(), String> {
    let runtime = state.runtime.clone();
    let alive = state.alive.clone();
    let playing = state.playing.clone();
    #[cfg(windows)]
    let presentation_scheduled = state.presentation_scheduled.clone();
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
                            let runtime = guard.as_mut().ok_or_else(|| {
                                "Native GPU runtime is not initialized".to_owned()
                            })?;
                            let frame = runtime.render_next()?;
                            #[cfg(windows)]
                            let presentation_pending = runtime.has_shared_texture_pending();
                            #[cfg(not(windows))]
                            let presentation_pending = false;
                            Ok((frame, runtime.take_output_events(), presentation_pending))
                        });
                    match result {
                        Ok((frame, output_events, presentation_pending)) => {
                            for event in output_events {
                                let _ = app.emit("native-runtime-output", event);
                            }
                            if frame.width > 0 && frame.height > 0 {
                                let _ = app.emit("native-runtime-frame", frame);
                            }
                            #[cfg(windows)]
                            if presentation_pending
                                && !presentation_scheduled.swap(true, Ordering::AcqRel)
                            {
                                let callback_app = app.clone();
                                let callback_scheduled = presentation_scheduled.clone();
                                if let Err(error) = app.run_on_main_thread(move || {
                                    if let Err(error) = present_latest_shared_texture(&callback_app) {
                                        let state = callback_app.state::<NativeRuntimeState>();
                                        let _ = with_runtime(&state, |runtime| {
                                            runtime.set_shared_texture_enabled(false).map(|_| ())
                                        });
                                        let _ = callback_app.emit(
                                            "native-runtime-presentation-fallback",
                                            error,
                                        );
                                    }
                                    callback_scheduled.store(false, Ordering::Release);
                                }) {
                                    presentation_scheduled.store(false, Ordering::Release);
                                    let _ = app.emit(
                                        "native-runtime-error",
                                        format!("Cannot schedule WebView2 texture presentation: {error}"),
                                    );
                                }
                            }
                        }
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
