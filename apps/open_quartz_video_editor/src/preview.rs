use crate::document::{ClipTransform, MediaAsset};
use open_quartz::execution::{
    FrameAdmission, FrameDemand, FrameEvaluationInput, FrameOutcome, FramePipeline,
};
use open_quartz::media::native::{NativeVideoConfig, NativeVideoSourceKind};
use open_quartz::outputs::{FrameStamp, PlaybackCapabilities};
use open_quartz::rendering::{RenderDevice, StableFrame};
use slint::wgpu_30::WGPUConfiguration;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Clone)]
pub struct PreviewRequest {
    pub ticket: u64,
    pub revision: u64,
    pub frame: u64,
    pub source_ns: u64,
    pub width: u32,
    pub height: u32,
    pub asset: Option<MediaAsset>,
    pub transform: ClipTransform,
}
pub struct PreviewUpdate {
    pub ticket: u64,
    pub revision: u64,
    pub frame: u64,
    pub image: Result<StableFrame, String>,
    pub actual_pts_ns: Option<i64>,
    pub cpu_copy_bytes: u64,
}
pub struct PreviewBridge {
    sender: mpsc::SyncSender<()>,
    latest: Arc<Mutex<Option<PreviewRequest>>>,
    closed: Arc<AtomicBool>,
    updates: Arc<Mutex<Option<PreviewUpdate>>>,
    worker: Option<JoinHandle<()>>,
    pub backend: Arc<open_quartz::rendering::GpuBackend>,
}
impl PreviewBridge {
    pub fn new() -> Result<(Self, WGPUConfiguration, String), String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: if cfg!(target_os = "macos") {
                wgpu::Backends::METAL
            } else {
                wgpu::Backends::PRIMARY
            },
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;
        let features = adapter.features() & wgpu::Features::TEXTURE_FORMAT_16BIT_NORM;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("OpenQuartz Studio shared GPU"),
            required_features: features,
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;
        let (render, info) =
            RenderDevice::from_shared_wgpu(device.clone(), queue.clone(), &adapter)?;
        let backend = render.backend();
        let config = WGPUConfiguration::Manual {
            instance,
            adapter,
            device,
            queue,
        };
        let (sender, receiver) = mpsc::sync_channel(1);
        let latest = Arc::new(Mutex::new(None));
        let worker_latest = latest.clone();
        let closed = Arc::new(AtomicBool::new(false));
        let worker_closed = closed.clone();
        let updates = Arc::new(Mutex::new(None));
        let publish = updates.clone();
        let worker = thread::Builder::new()
            .name("oq-editor-preview".into())
            .spawn(move || run(render, receiver, publish, worker_latest, worker_closed))
            .map_err(|e| e.to_string())?;
        Ok((
            Self {
                sender,
                latest,
                closed,
                updates,
                worker: Some(worker),
                backend,
            },
            config,
            format!(
                "{} · {} · resident video only",
                info.adapter_name, info.backend
            ),
        ))
    }
    pub fn request(&self, request: PreviewRequest) -> Result<(), String> {
        if self.closed.load(Ordering::Acquire) {
            return Err("Preview worker is closed".into());
        }
        *self
            .latest
            .lock()
            .map_err(|_| "Preview intent lock is poisoned")? = Some(request);
        match self.sender.try_send(()) {
            Ok(()) | Err(mpsc::TrySendError::Full(())) => Ok(()),
            Err(mpsc::TrySendError::Disconnected(())) => {
                Err("Preview worker is unavailable".into())
            }
        }
    }
    pub fn take_update(&self) -> Option<PreviewUpdate> {
        self.updates.lock().ok()?.take()
    }
    pub fn shutdown(&mut self) {
        self.closed.store(true, Ordering::Release);
        let _ = self.sender.try_send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for PreviewBridge {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn graph(request: &PreviewRequest) -> Result<open_quartz::Graph, String> {
    let (nodes, edges) = if let Some(asset) = &request.asset {
        let t = request.transform;
        let fit = ((request.width as f64 / asset.width as f64)
            .min(request.height as f64 / asset.height as f64))
            * f64::from(t.scale);
        let fit_x = asset.width as f64 * fit / request.width as f64;
        let fit_y = asset.height as f64 * fit / request.height as f64;
        let code=format!("@fragment fn main(@location(0) uv:vec2f)->@location(0) vec4f {{ let p=(uv-vec2f(0.5)-vec2f({:.8},{:.8}))/vec2f({:.8},{:.8})+vec2f(0.5); if(any(p<vec2f(0.0))||any(p>vec2f(1.0))){{return vec4f(0.0,0.0,0.0,1.0);}} let c=textureSample(image,imageSampler,p);return vec4f(clamp(c.rgb*{:.8},vec3f(0.0),vec3f(1.0)),1.0);}}",t.x,t.y,fit_x,fit_y,2.0f64.powf(f64::from(t.exposure)));
        (
            vec![
                serde_json::json!({"id":"video","type":"input","position":{"x":0,"y":0},"data":{"type":"input","label":"Media","shaderCode":"","inputMode":"video","inputDataType":"sampler2D","inputs":[],"outputs":[{"id":"out","label":"image","dataType":"sampler2D","direction":"output"}],"uniforms":{}}}),
                serde_json::json!({"id":"output","type":"shader","position":{"x":0,"y":0},"data":{"type":"shader","label":"Program","shaderCode":code,"inputs":[{"id":"in","label":"image","dataType":"sampler2D","direction":"input"}],"outputs":[{"id":"out","label":"fragColor","dataType":"vec4","direction":"output"}],"uniforms":{},"autoSize":false,"width":request.width,"height":request.height}}),
            ],
            vec![
                serde_json::json!({"id":"media-program","source":"video","sourceHandle":"out","target":"output","targetHandle":"in"}),
            ],
        )
    } else {
        (
            vec![
                serde_json::json!({"id":"output","type":"shader","position":{"x":0,"y":0},"data":{"type":"shader","label":"Gap","shaderCode":"@fragment fn main()->@location(0) vec4f{return vec4f(0.0,0.0,0.0,1.0);}","inputs":[],"outputs":[{"id":"out","label":"fragColor","dataType":"vec4","direction":"output"}],"uniforms":{},"autoSize":false,"width":request.width,"height":request.height}}),
            ],
            vec![],
        )
    };
    serde_json::from_value(serde_json::json!({"nodes":nodes,"edges":edges}))
        .map_err(|e| e.to_string())
}
fn run(
    render: RenderDevice,
    receiver: mpsc::Receiver<()>,
    updates: Arc<Mutex<Option<PreviewUpdate>>>,
    latest: Arc<Mutex<Option<PreviewRequest>>>,
    closed: Arc<AtomicBool>,
) {
    let mut pipeline =
        FramePipeline::from_render_device(render, PlaybackCapabilities { data_paths: vec![] });
    let owner = thread::current();
    let _ = pipeline.set_waker(Arc::new(move || owner.unpark()));
    let mut current: Option<PreviewRequest> = None;
    let mut pending_image: Option<StableFrame> = None;
    let mut configured: Option<(Option<String>, u32, u32, ClipTransform)> = None;
    let mut epoch = 1u64;
    let mut sequence = 0u64;
    let mut last_source = 0u64;
    let mut admitted = false;
    loop {
        if closed.load(Ordering::Acquire) {
            break;
        }
        if matches!(
            receiver.recv_timeout(Duration::from_millis(3)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ) {
            break;
        }
        let next = latest.lock().ok().and_then(|mut request| request.take());
        if let Some(request) = next {
            pipeline.cancel_pending();
            while pipeline.poll_completion().is_some() {}
            pending_image = None;
            admitted = false;
            let key = (
                request
                    .asset
                    .as_ref()
                    .map(|asset| format!("{}:{}", asset.path, asset.fingerprint)),
                request.width,
                request.height,
                request.transform,
            );
            let prepare = (|| {
                let limit = pipeline.backend().device.limits().max_texture_dimension_2d;
                if request.width == 0
                    || request.height == 0
                    || request.width > limit
                    || request.height > limit
                {
                    return Err("Sequence dimensions exceed this GPU's texture limits".into());
                }
                if request.asset.as_ref().is_some_and(|asset| !asset.online) {
                    return Err("Media offline — use Relink to restore the original file".into());
                }
                if configured.as_ref() != Some(&key) {
                    let graph = graph(&request)?;
                    let mut prepared = pipeline.prepare_graph(&graph)?;
                    if let Some(asset) = &request.asset {
                        prepared.stage_video(
                            "video",
                            NativeVideoConfig {
                                kind: NativeVideoSourceKind::File,
                                source: asset.path.clone(),
                                looping: false,
                                playback_rate: 1.0,
                            },
                            false,
                        )?;
                    }
                    pipeline.commit_graph(prepared)?;
                    configured = Some(key);
                    epoch = epoch.saturating_add(1);
                } else if request.source_ns < last_source {
                    epoch = epoch.saturating_add(1);
                }
                Ok::<_, String>(())
            })();
            if let Err(error) = prepare {
                publish(&updates, request, Err(error), 0);
                current = None;
            } else {
                last_source = request.source_ns;
                current = Some(request);
            }
        }
        let Some(request) = current.as_ref() else {
            continue;
        };
        if !admitted {
            sequence = sequence.saturating_add(1);
            let demand = FrameDemand {
                request_id: request.ticket,
                graph_revision: pipeline.revision(),
                output_node_id: "output".into(),
                sources: Vec::new(),
                input: FrameEvaluationInput {
                    stamp: FrameStamp {
                        epoch,
                        frame: sequence,
                        timeline_ns: request.source_ns,
                        deadline_ns: 0,
                    },
                    previous_timeline_ns: request.source_ns,
                    date: [0.0; 4],
                    mouse: [0.0; 4],
                    resolution: [request.width as f32, request.height as f32, 1.0],
                },
            };
            match pipeline.submit(demand) {
                FrameAdmission::Accepted => admitted = true,
                FrameAdmission::WouldBlock(_) => {}
                FrameAdmission::Rejected { error, .. } => {
                    publish(&updates, current.take().unwrap(), Err(error), 0);
                    continue;
                }
            }
        }
        pipeline.progress();
        let _ = pipeline.backend().device.poll(wgpu::PollType::Poll);
        if let Some(completion) = pipeline.poll_completion() {
            match completion.outcome {
                FrameOutcome::Submitted(frame) => pending_image = Some(frame),
                FrameOutcome::Failed(error) => {
                    publish(&updates, current.take().unwrap(), Err(error), 0);
                    continue;
                }
                FrameOutcome::Cancelled => {
                    current = None;
                    continue;
                }
            }
        }
        if pending_image
            .as_ref()
            .is_some_and(|image| image.ready.is_ready())
        {
            let bytes = pipeline
                .video_metrics("video")
                .map(|metrics| metrics.cpu_copy_bytes)
                .unwrap_or(0);
            publish(
                &updates,
                current.take().unwrap(),
                Ok(pending_image.take().unwrap()),
                bytes,
            );
        }
    }
    let _ = pipeline.dispose();
}
fn publish(
    updates: &Mutex<Option<PreviewUpdate>>,
    request: PreviewRequest,
    image: Result<StableFrame, String>,
    cpu_copy_bytes: u64,
) {
    let actual_pts_ns = image.as_ref().ok().and_then(|frame| {
        frame
            .identity
            .content_stamp
            .as_ref()
            .and_then(|stamp| stamp.media_pts_ns)
    });
    if let Ok(mut slot) = updates.lock() {
        *slot = Some(PreviewUpdate {
            ticket: request.ticket,
            revision: request.revision,
            frame: request.frame,
            image,
            actual_pts_ns,
            cpu_copy_bytes,
        });
    }
}

#[cfg(test)]
#[path = "preview_tests.rs"]
mod preview_tests;
