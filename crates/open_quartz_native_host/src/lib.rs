use std::collections::{HashMap, HashSet};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use open_quartz_execution::engine::ExecutionCommand;
use open_quartz_execution::error::SdkError;
#[cfg(windows)]
use open_quartz_execution::gpu::{DxgiSharedTextureExporter, SharedTexturePresenter};
use open_quartz_execution::gpu::{GpuBackend, GpuExecutor, GpuOutputHandle, TextureFormat};
#[cfg(windows)]
use open_quartz_execution::gpu::{
    GpuPresentationFrame, GpuPresenter, GpuPreviewReader, SharedTextureFrame,
};
use open_quartz_execution::host::PlayerHost;
use open_quartz_execution::native_video::{
    find_ffmpeg, NativeVideoConfig, NativeVideoFrame, NativeVideoInfo, NativeVideoMetrics,
    NativeVideoSource,
};
use open_quartz_execution::onnx::{NativeOnnxImageOutput, OnnxSession, OnnxTask};
use open_quartz_execution::runtime::{
    AsyncCompletionEnvelope, ContentStamp, DataPathMode, DeliveryPolicy, FrameStamp, OutputKey,
    OutputPayload, OutputSubscription, OutputTransport, RuntimeCapabilities, RuntimeFrameInput,
};
use open_quartz_schema::{DataType, Graph, NodeType};
use serde::{Deserialize, Serialize};

#[cfg(windows)]
const PRESENTATION_MAX_DIMENSION: u32 = 3_840;
fn emit_perf(message: std::fmt::Arguments<'_>) {
    log::info!(target: "open_quartz::perf", "{message}");
    if std::env::var_os("OPEN_QUARTZ_NATIVE_VIDEO_BENCH").is_some()
        || std::env::var_os("OPEN_QUARTZ_PERF_LOG").is_some()
    {
        println!("[oq:perf] {message}");
    }
}

#[derive(Clone)]
pub struct NativeOnnxConfig {
    pub model_id: String,
    pub task: OnnxTask,
    pub target_size: u32,
    pub score_threshold: f32,
    pub iou_threshold: f32,
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
    input_stamp: FrameStamp,
    result: Result<NativeOnnxImageOutput, String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeOutputEvent {
    pub node_id: String,
    pub width: u32,
    pub height: u32,
    pub backend: String,
    pub data: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeOnnxLoadOptions {
    pub model_path: Option<String>,
    pub task: OnnxTask,
    pub target_size: u32,
    pub score_threshold: f32,
    pub iou_threshold: f32,
    pub prefer_direct_ml: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeRuntimeInfo {
    pub adapter_name: String,
    pub backend: String,
    pub device_type: String,
    pub output_mode: String,
    pub native_onnx_cpu: bool,
    pub native_onnx_direct_ml: bool,
    pub shared_onnx_wgpu_device: bool,
    pub native_video: bool,
    pub video_data_path: String,
    pub tensor_data_path: String,
    pub shared_texture: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeFrameRendered {
    pub frame: u64,
    pub revision: u32,
    pub output_node_id: String,
    pub width: u32,
    pub height: u32,
}
struct NativeRenderPerf {
    window_started: Instant,
    frames: u64,
    upload_video: Duration,
    engine_plan: Duration,
    execute_gpu: Duration,
    total: Duration,
    max_total: Duration,
}

impl NativeRenderPerf {
    fn new() -> Self {
        Self {
            window_started: Instant::now(),
            frames: 0,
            upload_video: Duration::ZERO,
            engine_plan: Duration::ZERO,
            execute_gpu: Duration::ZERO,
            total: Duration::ZERO,
            max_total: Duration::ZERO,
        }
    }

    fn record(
        &mut self,
        upload_video: Duration,
        engine_plan: Duration,
        execute_gpu: Duration,
        total: Duration,
        decoded_frames: u64,
        uploaded_frames: u64,
    ) {
        if self.frames == 0 {
            self.window_started = Instant::now();
        }
        self.frames += 1;
        self.upload_video += upload_video;
        self.engine_plan += engine_plan;
        self.execute_gpu += execute_gpu;
        self.total += total;
        self.max_total = self.max_total.max(total);
        let window = self.window_started.elapsed();
        if window < Duration::from_secs(1) {
            return;
        }
        let frames = self.frames.max(1) as f64;
        emit_perf(format_args!(
            "native-render fps={:.1} avg_ms={{upload_video:{:.3},engine_plan:{:.3},execute_gpu:{:.3},total:{:.3}}} max_total_ms={:.3} video={{decoded:{decoded_frames},uploaded:{uploaded_frames}}}",
            self.frames as f64 / window.as_secs_f64(),
            self.upload_video.as_secs_f64() * 1000.0 / frames,
            self.engine_plan.as_secs_f64() * 1000.0 / frames,
            self.execute_gpu.as_secs_f64() * 1000.0 / frames,
            self.total.as_secs_f64() * 1000.0 / frames,
            self.max_total.as_secs_f64() * 1000.0,
        ));
        *self = Self::new();
    }
}
pub struct NativeHost {
    executor: GpuExecutor,
    runtime: PlayerHost,
    output_node_id: Option<String>,
    clock_origin: Instant,
    #[cfg(windows)]
    presentation_started_at: Instant,
    mouse: [f32; 4],
    videos: HashMap<String, NativeVideoSource>,
    onnx_resources: HashMap<String, NativeOnnxResource>,
    onnx_pending: HashSet<String>,
    onnx_outputs: HashMap<String, Vec<(OutputKey, DataType)>>,
    onnx_event_subscriptions: HashMap<String, String>,
    onnx_sender: mpsc::Sender<NativeOnnxCompletion>,
    onnx_receiver: mpsc::Receiver<NativeOnnxCompletion>,
    output_events: Vec<NativeOutputEvent>,
    onnx_workers: Vec<JoinHandle<()>>,
    perf: NativeRenderPerf,
    #[cfg(windows)]
    shared_presenter: Option<SharedTexturePresenter<DxgiSharedTextureExporter>>,
    #[cfg(windows)]
    presentation_scaler: GpuPreviewReader,
    #[cfg(windows)]
    shared_texture_enabled: bool,
}

fn runtime_error(error: SdkError) -> String {
    error.to_json()
}

impl NativeHost {
    pub async fn new() -> Result<(Self, NativeRuntimeInfo), String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: native_backends(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
                apply_limit_buckets: false,
            })
            .await
            .map_err(|error| format!("Cannot find a native GPU adapter: {error}"))?;
        let adapter_features = adapter.features();
        let mut required_features = wgpu::Features::empty();
        if adapter_features.contains(wgpu::Features::TEXTURE_FORMAT_NV12) {
            required_features |= wgpu::Features::TEXTURE_FORMAT_NV12;
        }
        if adapter_features.contains(wgpu::Features::TEXTURE_FORMAT_P010)
            && adapter_features.contains(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM)
        {
            required_features |=
                wgpu::Features::TEXTURE_FORMAT_P010 | wgpu::Features::TEXTURE_FORMAT_16BIT_NORM;
        }
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_features,
                ..Default::default()
            })
            .await
            .map_err(|error| format!("Cannot create native GPU device: {error}"))?;
        Self::from_shared_wgpu(device, queue, &adapter)
    }

    pub fn from_shared_wgpu(
        device: wgpu::Device,
        queue: wgpu::Queue,
        adapter: &wgpu::Adapter,
    ) -> Result<(Self, NativeRuntimeInfo), String> {
        let adapter_info = adapter.get_info();
        let required_features = device.features();
        let backend = Arc::new(GpuBackend::from_device(device, queue));
        let executor = GpuExecutor::new(backend.clone());
        #[cfg(windows)]
        let shared_presenter = DxgiSharedTextureExporter::new(backend.clone())
            .ok()
            .map(SharedTexturePresenter::new);
        #[cfg(windows)]
        let presentation_scaler = GpuPreviewReader::new(backend.clone());
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
                && required_features.intersects(
                    wgpu::Features::TEXTURE_FORMAT_NV12 | wgpu::Features::TEXTURE_FORMAT_P010,
                ) {
                "d3d12va-yuv-detached-gpu-copy".to_owned()
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
                runtime: PlayerHost::new_native(RuntimeCapabilities {
                    data_paths: if shared_texture {
                        vec![DataPathMode::NativePresent]
                    } else {
                        vec![DataPathMode::CpuCopy]
                    },
                }),
                output_node_id: None,
                clock_origin: now,
                #[cfg(windows)]
                presentation_started_at: now,
                mouse: [0.0; 4],
                videos: HashMap::new(),
                onnx_resources: HashMap::new(),
                onnx_pending: HashSet::new(),
                onnx_outputs: HashMap::new(),
                onnx_event_subscriptions: HashMap::new(),
                onnx_sender,
                onnx_receiver,
                output_events: Vec::new(),
                onnx_workers: Vec::new(),
                perf: NativeRenderPerf::new(),
                #[cfg(windows)]
                presentation_scaler,
                #[cfg(windows)]
                shared_presenter,
                #[cfg(windows)]
                shared_texture_enabled: false,
            },
            info,
        ))
    }

    pub fn set_graph(&mut self, graph_json: &str) -> Result<u32, String> {
        let graph: Graph = serde_json::from_str(graph_json)
            .map_err(|error| format!("Invalid native graph JSON: {error}"))?;
        for subscription_id in self.onnx_event_subscriptions.values() {
            self.runtime
                .unsubscribe_output(subscription_id)
                .map_err(runtime_error)?;
        }
        self.onnx_event_subscriptions.clear();
        self.onnx_outputs = graph
            .nodes
            .iter()
            .filter(|node| node.node_type == NodeType::Onnx)
            .map(|node| {
                let outputs = node
                    .data
                    .outputs
                    .iter()
                    .map(|port| {
                        (
                            OutputKey::new(node.id.clone(), port.id.clone()),
                            port.data_type,
                        )
                    })
                    .collect::<Vec<_>>();
                (node.id.clone(), outputs)
            })
            .collect();
        let revision = self.runtime.set_graph(&graph).map_err(runtime_error)?;
        for (node_id, outputs) in &self.onnx_outputs {
            let selected = outputs
                .iter()
                .find(|(_, data_type)| {
                    matches!(data_type, DataType::Roi | DataType::Mesh | DataType::Json)
                })
                .or_else(|| outputs.first());
            let Some((output, data_type)) = selected else {
                continue;
            };
            let subscription_id = format!("native-output-{node_id}");
            self.runtime
                .subscribe_output(OutputSubscription {
                    subscription_id: subscription_id.clone(),
                    output: output.clone(),
                    delivery: DeliveryPolicy::OnChange,
                    transport: if matches!(data_type, DataType::Sampler2d | DataType::SamplerCube) {
                        OutputTransport::Preview
                    } else {
                        OutputTransport::Value
                    },
                    max_width: None,
                    max_height: None,
                })
                .map_err(runtime_error)?;
            self.onnx_event_subscriptions
                .insert(node_id.clone(), subscription_id);
        }
        let (output_node_id, node_ids) = {
            let plan = self
                .runtime
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
        self.executor.sync_sampling(&graph);
        self.output_node_id = output_node_id;
        self.videos.retain(|node_id, _| node_ids.contains(node_id));
        self.onnx_resources
            .retain(|node_id, _| node_ids.contains(node_id));
        self.onnx_pending
            .retain(|node_id| node_ids.contains(node_id));
        self.output_events
            .retain(|event| node_ids.contains(&event.node_id));
        self.sync_video_nodes()?;
        self.clock_origin = Instant::now();
        Ok(revision)
    }

    pub fn upload_image(
        &mut self,
        node_id: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        self.runtime
            .node_generation(node_id)
            .map_err(runtime_error)?;
        self.executor
            .upload_rgba(node_id, rgba, width, height)
            .map_err(|error| error.to_string())?;
        self.runtime.mark_dirty(node_id).map_err(runtime_error)
    }

    pub fn remove_texture(&mut self, node_id: &str) {
        self.executor.remove_texture(node_id);
    }

    pub fn read_output(&self, node_id: &str) -> Result<Vec<u8>, String> {
        let (rgba, width, height) = self.read_output_rgba(node_id)?;
        Ok(Self::output_payload(&rgba, width, height))
    }

    pub fn preview_source(&self, node_id: &str) -> Result<GpuOutputHandle, String> {
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

    pub fn output_payload(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
        let mut payload = Vec::with_capacity(8 + rgba.len());
        payload.extend_from_slice(&width.to_le_bytes());
        payload.extend_from_slice(&height.to_le_bytes());
        payload.extend_from_slice(rgba);
        payload
    }

    pub fn attach_video(
        &mut self,
        node_id: &str,
        config: NativeVideoConfig,
    ) -> Result<NativeVideoInfo, String> {
        self.runtime
            .node_generation(node_id)
            .map_err(runtime_error)?;
        let source = NativeVideoSource::open(config)?;
        let info = source.info().clone();
        self.videos.insert(node_id.to_owned(), source);
        self.sync_video_nodes()?;
        self.runtime.mark_dirty(node_id).map_err(runtime_error)?;
        Ok(info)
    }

    pub fn detach_video(&mut self, node_id: &str) -> Result<(), String> {
        self.videos.remove(node_id);
        self.executor.remove_texture(node_id);
        self.sync_video_nodes()
    }

    fn sync_video_nodes(&mut self) -> Result<(), String> {
        let node_ids = self.videos.keys().cloned().collect::<Vec<_>>();
        self.runtime
            .set_video_nodes(&node_ids)
            .map_err(runtime_error)
    }

    pub fn upload_video_frames(&mut self) -> Result<(), String> {
        let (videos, executor) = (&mut self.videos, &mut self.executor);
        let mut dirty = Vec::new();
        for (node_id, source) in videos {
            let uploaded = source.upload_latest(&mut |frame, width, height| match frame {
                NativeVideoFrame::Rgba(rgba) => executor
                    .upload_rgba(node_id, rgba, width, height)
                    .map_err(|error| error.to_string()),
                #[cfg(all(windows, feature = "native-video-d3d12"))]
                NativeVideoFrame::D3d12(frame) => executor
                    .upload_d3d12_yuv(node_id, frame)
                    .map_err(|error| error.to_string()),
                #[cfg(all(windows, not(feature = "native-video-d3d12")))]
                #[allow(unreachable_patterns)]
                _ => Err("D3D12 video frames require the native-video-d3d12 feature".to_owned()),
            })?;
            if uploaded {
                dirty.push(node_id.clone());
            }
        }
        for node_id in dirty {
            self.runtime.mark_dirty(&node_id).map_err(runtime_error)?;
        }
        Ok(())
    }

    fn pause_videos(&mut self) {
        for source in self.videos.values_mut() {
            source.pause();
        }
    }

    pub fn start_playback(&mut self) -> Result<(), String> {
        self.resume_videos()?;
        let now_ns = self.clock_now_ns();
        self.runtime.play(now_ns).map_err(runtime_error)
    }

    fn clock_now_ns(&self) -> u64 {
        self.clock_origin
            .elapsed()
            .as_nanos()
            .min(u128::from(u64::MAX)) as u64
    }

    fn resume_videos(&mut self) -> Result<(), String> {
        for source in self.videos.values_mut() {
            source.resume()?;
        }
        Ok(())
    }

    pub fn load_onnx(
        &mut self,
        node_id: String,
        session: OnnxSession,
        config: NativeOnnxConfig,
    ) -> Result<open_quartz_execution::onnx::OnnxSessionInfo, String> {
        self.runtime
            .node_generation(&node_id)
            .map_err(runtime_error)?;
        let info = session.info().clone();
        self.onnx_resources.insert(
            node_id.clone(),
            NativeOnnxResource {
                session: Arc::new(Mutex::new(session)),
                config,
                backend: info.backend.clone(),
            },
        );
        self.runtime.mark_dirty(&node_id).map_err(runtime_error)?;
        Ok(info)
    }

    pub fn unload_onnx(&mut self, node_id: &str) {
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
            let current_generation = self.runtime.node_generation(&completion.node_id).ok();
            if completion.revision != self.runtime.revision()
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
            let completion_outputs = self
                .onnx_outputs
                .get(&completion.node_id)
                .into_iter()
                .flatten()
                .filter_map(|(key, data_type)| {
                    let payload = match data_type {
                        DataType::Sampler2d | DataType::SamplerCube => OutputPayload::Resource {
                            handle: u64::from(completion.generation),
                        },
                        DataType::Roi | DataType::Mesh | DataType::Json | DataType::Auto => {
                            OutputPayload::Json(
                                output.data.clone().unwrap_or(serde_json::Value::Null),
                            )
                        }
                        _ => return None,
                    };
                    Some((key.clone(), payload))
                })
                .collect();
            self.runtime
                .submit_completion(AsyncCompletionEnvelope {
                    node_id: completion.node_id.clone(),
                    graph_revision: completion.revision,
                    node_generation: completion.generation,
                    input_stamp: completion.input_stamp.clone(),
                    content_stamp: ContentStamp {
                        epoch: completion.input_stamp.epoch,
                        timeline_ns: completion.input_stamp.timeline_ns,
                        media_pts_ns: None,
                    },
                    outputs: completion_outputs,
                })
                .map_err(runtime_error)?;
            let expected_subscription = self.onnx_event_subscriptions.get(&completion.node_id);
            let delivered = self
                .runtime
                .drain_deliveries()
                .deliveries
                .iter()
                .any(|delivery| Some(&delivery.subscription_id) == expected_subscription);
            if delivered {
                self.output_events.push(NativeOutputEvent {
                    node_id: completion.node_id,
                    width: output.width,
                    height: output.height,
                    backend,
                    data: output.data,
                });
            }
        }
        Ok(())
    }

    fn schedule_onnx(
        &mut self,
        command: &ExecutionCommand,
        input_stamp: &FrameStamp,
    ) -> Result<(), String> {
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
        let revision = self.runtime.revision();
        let generation = self
            .runtime
            .node_generation(&node_id)
            .map_err(runtime_error)?;
        let input_stamp = input_stamp.clone();
        self.onnx_pending.insert(node_id.clone());
        let worker = std::thread::Builder::new()
            .name(format!("open-quartz-onnx-{node_id}"))
            .spawn(move || {
                let result = session
                    .lock()
                    .map_err(|_| "Native ONNX session lock is poisoned".to_owned())
                    .and_then(|mut session| {
                        open_quartz_execution::onnx::run_native_image_task(
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
                    input_stamp,
                    result,
                });
            })
            .map_err(|error| format!("Cannot start native ONNX task: {error}"))?;
        self.onnx_workers.push(worker);
        Ok(())
    }

    fn execute_runtime_commands(
        &mut self,
        commands: &[ExecutionCommand],
        input_stamp: &FrameStamp,
    ) -> Result<(), String> {
        let mut batch = Vec::new();
        for command in commands {
            if command.kind == "onnx" {
                if !batch.is_empty() {
                    self.runtime
                        .execute_gpu(&mut self.executor, &batch)
                        .map_err(runtime_error)?;
                    batch.clear();
                }
                self.schedule_onnx(command, input_stamp)?;
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
            self.runtime
                .execute_gpu(&mut self.executor, &batch)
                .map_err(runtime_error)?;
        }
        Ok(())
    }

    #[cfg(windows)]
    pub fn set_shared_texture_enabled(&mut self, enabled: bool) -> Result<bool, String> {
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
    pub fn take_shared_texture(&mut self) -> Option<SharedTextureFrame> {
        self.shared_presenter
            .as_mut()
            .and_then(SharedTexturePresenter::take_latest)
    }

    #[cfg(windows)]
    pub fn has_shared_texture_pending(&self) -> bool {
        self.shared_presenter
            .as_ref()
            .and_then(SharedTexturePresenter::latest)
            .is_some()
    }

    #[cfg(windows)]
    pub fn release_shared_texture(&mut self, lease_id: u64) -> Result<(), String> {
        self.shared_presenter
            .as_mut()
            .ok_or_else(|| "DXGI shared texture Presenter is unavailable".to_owned())?
            .release(lease_id)
    }

    pub fn take_output_events(&mut self) -> Vec<NativeOutputEvent> {
        std::mem::take(&mut self.output_events)
    }

    pub fn render_next(&mut self) -> Result<NativeFrameRendered, String> {
        let now_ns = self.clock_now_ns();
        self.render(now_ns)
    }
    pub fn backend(&self) -> Arc<GpuBackend> {
        self.executor.backend().clone()
    }

    pub fn has_graph(&self) -> bool {
        self.runtime.execution_plan().is_some()
    }

    pub fn has_output_texture(&self, node_id: &str) -> bool {
        self.executor.output_texture(node_id).is_some()
    }

    pub fn video_metrics(&self, node_id: &str) -> Result<NativeVideoMetrics, String> {
        self.videos
            .get(node_id)
            .map(NativeVideoSource::metrics)
            .ok_or_else(|| format!("Video node {node_id} is not attached"))
    }

    pub fn pause(&mut self) -> Result<(), String> {
        self.pause_videos();
        let now_ns = self.clock_now_ns();
        self.runtime.pause(now_ns).map_err(runtime_error)
    }

    pub fn resume(&mut self) -> Result<(), String> {
        self.resume_videos()?;
        let now_ns = self.clock_now_ns();
        self.runtime.resume(now_ns).map_err(runtime_error)
    }

    pub fn stop(&mut self) -> Result<(), String> {
        self.pause_videos();
        self.runtime.stop().map_err(runtime_error)
    }

    pub fn set_mouse(&mut self, mouse: [f32; 4]) {
        self.mouse = mouse;
    }

    pub fn drain_events(&mut self) -> Vec<open_quartz_execution::event::EngineEvent> {
        self.runtime.drain_events()
    }

    pub fn drain_events_json(&mut self) -> Result<String, String> {
        serde_json::to_string(&self.drain_events())
            .map_err(|error| format!("Cannot serialize runtime events: {error}"))
    }

    fn render(&mut self, now_ns: u64) -> Result<NativeFrameRendered, String> {
        let total_started = Instant::now();
        let (width, height) = self
            .runtime
            .execution_plan()
            .map(|plan| (plan.default_width.max(1), plan.default_height.max(1)))
            .unwrap_or((960, 540));
        let upload_started = Instant::now();
        self.upload_video_frames()?;
        let upload_video = upload_started.elapsed();
        let engine_started = Instant::now();
        self.drain_onnx_completions()?;
        let date = utc_date_uniform(SystemTime::now());
        let clock = self
            .runtime
            .advance(&RuntimeFrameInput {
                now_ns,
                date,
                mouse: self.mouse,
                resolution: [width as f32, height as f32, 1.0],
            })
            .map_err(runtime_error)?;
        let frame_stamp = FrameStamp {
            epoch: clock.epoch,
            frame: clock.frame,
            timeline_ns: clock.timeline_ns,
            deadline_ns: clock.next_deadline_ns,
        };
        let frame = clock.frame;
        let plan = self
            .runtime
            .execution_plan()
            .ok_or_else(|| "Native engine has no execution plan".to_owned())?
            .clone();
        let commands = self.runtime.drain_commands();
        let engine_plan = engine_started.elapsed();
        let execute_started = Instant::now();
        self.execute_runtime_commands(&commands, &frame_stamp)?;
        let execute_gpu = execute_started.elapsed();
        let output_node_id = self
            .output_node_id
            .clone()
            .or_else(|| plan.output_nodes.first().cloned())
            .ok_or_else(|| "Graph has no renderer or terminal texture output".to_owned())?;
        let Some(output) = self.executor.output_texture(&output_node_id) else {
            if !self.onnx_pending.is_empty()
                || self.videos.values().any(NativeVideoSource::is_active)
            {
                return Ok(NativeFrameRendered {
                    frame,
                    revision: self.runtime.revision(),
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
            if let Some(output_handle) = self.executor.output_handle(&output_node_id) {
                let presentation_output =
                    if output_handle.width.max(output_handle.height) > PRESENTATION_MAX_DIMENSION {
                        self.presentation_scaler
                            .scale(&output_handle, PRESENTATION_MAX_DIMENSION)?
                    } else {
                        output_handle
                    };
                if let Some(presenter) = self.shared_presenter.as_mut() {
                    let _ = presenter.submit(GpuPresentationFrame {
                        node_id: output_node_id.clone(),
                        frame,
                        timeline_ns: elapsed_ns(self.presentation_started_at),
                        output: presentation_output,
                    });
                    let _ = presenter.process_latest();
                }
            }
        }
        let rendered = NativeFrameRendered {
            frame,
            revision: self.runtime.revision(),
            output_node_id,
            width: output.width,
            height: output.height,
        };
        let (decoded_frames, uploaded_frames) =
            self.videos
                .values()
                .fold((0_u64, 0_u64), |(decoded, uploaded), source| {
                    let metrics = source.metrics();
                    (
                        decoded.saturating_add(metrics.decoded_frames),
                        uploaded.saturating_add(metrics.uploaded_frames),
                    )
                });
        self.perf.record(
            upload_video,
            engine_plan,
            execute_gpu,
            total_started.elapsed(),
            decoded_frames,
            uploaded_frames,
        );
        Ok(rendered)
    }
}

impl Drop for NativeHost {
    fn drop(&mut self) {
        for worker in self.onnx_workers.drain(..) {
            let _ = worker.join();
        }
    }
}
#[cfg(windows)]
fn elapsed_ns(origin: Instant) -> u64 {
    origin.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
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
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    return wgpu::Backends::METAL;
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "ios")))]
    return wgpu::Backends::VULKAN;
}

#[cfg(test)]
mod tests {
    use super::{civil_from_days, utc_date_uniform, NativeFrameRendered, NativeHost};
    use std::time::{Duration, UNIX_EPOCH};

    #[cfg(target_os = "macos")]
    #[test]
    fn native_host_uses_metal_on_macos() {
        assert_eq!(super::native_backends(), wgpu::Backends::METAL);
    }

    #[test]
    fn output_payload_prefixes_little_endian_dimensions() {
        let payload = NativeHost::output_payload(&[1, 2, 3, 4], 640, 360);
        assert_eq!(&payload[..4], &640_u32.to_le_bytes());
        assert_eq!(&payload[4..8], &360_u32.to_le_bytes());
        assert_eq!(&payload[8..], &[1, 2, 3, 4]);
    }

    #[test]
    fn frame_contract_keeps_camel_case_names() {
        let value = serde_json::to_value(NativeFrameRendered {
            frame: 3,
            revision: 2,
            output_node_id: "renderer".to_owned(),
            width: 640,
            height: 360,
        })
        .expect("frame should serialize");
        assert_eq!(value["outputNodeId"], "renderer");
        assert!(value.get("output_node_id").is_none());
    }

    #[test]
    fn converts_unix_time_to_wgsl_date_uniform() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(
            utc_date_uniform(UNIX_EPOCH + Duration::from_secs(86_400)),
            [1970.0, 1.0, 2.0, 0.0]
        );
    }
}
