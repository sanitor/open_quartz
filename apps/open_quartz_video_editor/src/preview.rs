use crate::clip_pipeline::{ClipPipeline, OUTPUT_ID, SOURCE_ID};
use crate::document::{self, ClipTransform, MediaAsset, ProjectData, Rational};
use open_quartz::execution::{
    FrameAdmission, FrameDemand, FrameEvaluationInput, FrameOutcome, FramePipeline, FrameWindowConfig,
    PreparedNativeGraph,
};
use open_quartz::media::native::{NativeVideoConfig, NativeVideoSourceKind};
use open_quartz::outputs::{FrameStamp, PlaybackCapabilities};
use open_quartz::rendering::{RenderDevice, StableFrame, StableFrameBudget};
use slint::wgpu_30::WGPUConfiguration;
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct PreviewRequest {
    pub ticket: u64,
    pub revision: u64,
    pub frame: u64,
    // The graph clock uses media time for single-source preview and an
    // animation-relative sequence time when the composite evaluates keys.
    pub source_ns: u64,
    pub width: u32,
    pub height: u32,
    pub asset: Option<MediaAsset>,
    pub transform: ClipTransform,
    pub pipeline: ClipPipeline,
    pub layers: Vec<PreviewLayer>,
}
/// Visible layers are stored from the top timeline row to the bottom.
#[derive(Clone)]
pub struct PreviewLayer {
    pub clip_id: String,
    pub track_name: String,
    pub asset: MediaAsset,
    pub source_ns: u64,
    pub transform: ClipTransform,
    pub pipeline: ClipPipeline,
    opacity: Option<OpacityAnimation>,
}

#[derive(Clone)]
struct OpacityAnimation {
    keys: Vec<document::OpacityKey>,
    timeline_start_ns: u64,
    source_in_ns: u64,
}

pub fn sequence_request(
    project: &ProjectData, ticket: u64, revision: u64, frame: u64,
    mut pipeline_override: impl FnMut(&str) -> Option<ClipPipeline>,
    transform_override: Option<(&str, ClipTransform)>,
) -> Result<PreviewRequest, String> {
    let mut layers = Vec::new();
    for (track, clip) in project.active_clips(frame) {
        let asset = project.assets.iter().find(|asset| asset.id == clip.asset_id)
            .ok_or_else(|| format!("{}: missing clip media", track.name))?;
        let opacity = if clip.opacity_keys.is_empty() { None } else {
            Some(OpacityAnimation {
                keys: clip.opacity_keys.clone(),
                timeline_start_ns: project.fps.frames_to_ns(clip.timeline_start)?,
                source_in_ns: document::source_time_ns(asset, clip, clip.timeline_start, project.fps)?,
            })
        };
        layers.push(PreviewLayer {
            clip_id: clip.id.clone(), track_name: track.name.clone(), asset: asset.clone(),
            source_ns: document::source_time_ns(asset, clip, frame, project.fps)?,
            transform: transform_override.filter(|(id, _)| *id == clip.id)
                .map_or(clip.transform, |(_, value)| value),
            pipeline: pipeline_override(&clip.id).unwrap_or_else(|| clip.pipeline.clone()),
            opacity,
        });
    }
    let mut request = PreviewRequest {
        ticket, revision, frame, width: project.width, height: project.height,
        asset: None, source_ns: project.fps.frames_to_ns(frame)?, transform: ClipTransform::default(),
        pipeline: ClipPipeline::default(), layers: Vec::new(),
    };
    if let [layer] = layers.as_slice() {
        if project.tracks.len() == 1 && layer.opacity.is_none()
            && layer.transform.rotation == 0.0 && layer.transform.opacity == 1.0 {
            let layer = layers.pop().expect("one preview layer");
            request.asset = Some(layer.asset);
            request.source_ns = layer.source_ns;
            request.transform = layer.transform;
            request.pipeline = layer.pipeline;
            return Ok(request);
        }
    }
    request.layers = layers;
    request.source_ns -= request.animation_origin_ns();
    Ok(request)
}

fn sequence_region(project: &ProjectData, ticket: u64, revision: u64, frame: u64, duration: u64) -> Result<(PreviewRequest, u64), String> {
    let end = project.tracks.iter().filter(|track| track.visible).flat_map(|track| {
        let active_end = track.clip_at(frame).and_then(|clip| clip.timeline_end().ok());
        let next = track.clips.partition_point(|clip| clip.timeline_start <= frame);
        active_end.into_iter().chain(track.clips.get(next).map(|clip| clip.timeline_start))
    }).min().unwrap_or(duration);
    Ok((sequence_request(project, ticket, revision, frame, |_| None, None)?, end))
}
fn sequence_source_times(project: &ProjectData, request: &PreviewRequest, frame: u64) -> Result<(u64, BTreeMap<String, u64>), String> {
    let mut times = BTreeMap::new();
    for (_, clip) in project.active_clips(frame) {
        let asset = project.assets.iter().find(|asset| asset.id == clip.asset_id).ok_or("Clip media is missing")?;
        let source_ns = document::source_time_ns(asset, clip, frame, project.fps)?;
        if request.asset.is_some() {
            times.insert(SOURCE_ID.into(), source_ns);
            return Ok((source_ns, times));
        }
        times.insert(composition::source_id(&clip.id), source_ns);
    }
    let timeline_ns = if times.is_empty() { 0 } else {
        project.fps.frames_to_ns(frame)?.checked_sub(request.animation_origin_ns())
            .ok_or("Frame precedes its animation origin")?
    };
    Ok((timeline_ns, times))
}

impl PreviewRequest {
    fn animation_origin_ns(&self) -> u64 {
        self.layers.iter().filter_map(|layer| layer.opacity.as_ref())
            .map(|animation| animation.timeline_start_ns).max().unwrap_or(0)
    }

    fn sources(&self) -> impl Iterator<Item = (String, &MediaAsset, u64)> {
        self.asset.iter().map(|asset| (SOURCE_ID.into(), asset, self.source_ns))
            .chain(self.layers.iter().map(|layer| {
                (composition::source_id(&layer.clip_id), &layer.asset, layer.source_ns)
            }))
    }
}

pub struct PreviewUpdate {
    pub ticket: u64,
    pub revision: u64,
    pub epoch: u64,
    pub frame: u64,
    pub image: Result<StableFrame, String>,
    pub actual_pts_ns: Option<i64>,
    pub layer_pts_ns: Vec<(String, Option<i64>)>,
    pub cpu_copy_bytes: u64,
}

pub struct PlaybackRequest {
    pub ticket: u64,
    pub revision: u64,
    pub start_frame: u64,
    pub project: ProjectData,
    #[cfg(target_os = "macos")]
    pub audio_clock: Option<Arc<crate::audio_output::AudioClock>>,
}
enum PreviewCommand {
    Exact(PreviewRequest),
    Play(PlaybackRequest),
}
#[derive(Clone, Default, serde::Serialize)]
pub struct PreviewProgress {
    pub ticket: u64,
    pub revision: u64,
    pub epoch: u64,
    pub frame: u64,
    pub preparing: bool,
    pub playing: bool,
    pub width: u32,
    pub height: u32,
    pub displayed: u64,
    pub displayed_frame: Option<u64>,
    pub actual_pts_ns: Option<i64>,
    pub displayed_width: u32,
    pub displayed_height: u32,
    pub byte_limit: u64,
    pub stable_byte_limit: u64,
    pub window_high_water: usize,
    pub ready: usize,
    pub ready_peak: usize,
    pub source_opens: u64,
    pub submitted: u64,
    pub presented: u64,
    pub coalesced: u64,
    pub late: u64,
    pub dropped: u64,
    pub queue: usize,
    pub queue_peak: usize,
    pub surfaces: usize,
    pub bytes: u64,
    pub surfaces_peak: usize,
    pub bytes_peak: u64,
    pub lifetime_peaks: bool,
    pub window_current_capacity: usize,
    pub stable_surfaces: usize,
    pub stable_bytes: u64,
    pub pipeline_creations: u64,
    pub target_allocations: u64,
    pub decoded: u64,
    pub uploaded: u64,
    pub cpu_copy_bytes: u64,
}
struct PipelineValidation {
    pipeline: ClipPipeline,
    width: u32,
    height: u32,
    result: mpsc::Sender<Result<(), String>>,
}
pub struct PreviewBridge {
    validation_sender: mpsc::SyncSender<PipelineValidation>,
    latest: Arc<Mutex<Option<PreviewCommand>>>,
    closed: Arc<AtomicBool>,
    updates: Arc<Mutex<Option<PreviewUpdate>>>,
    progress: Arc<Mutex<PreviewProgress>>,
    delivered: Arc<AtomicU64>,
    coalesced: Arc<AtomicU64>,
    worker: Option<JoinHandle<()>>,
    #[cfg(target_os = "macos")]
    render_info: open_quartz::rendering::RenderDeviceInfo,
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
        // One shared coordinator also charges native imports, stable outputs and
        // thumbnails. Extent-specific sublimits below never enlarge this cap.
        backend.set_frame_budget(open_quartz::resources::FrameBudget::new(64, 2 * 1024 * 1024 * 1024)?)?;
        let (validation_sender, validation_receiver) = mpsc::sync_channel(8);
        let latest = Arc::new(Mutex::new(None));
        let worker_latest = latest.clone();
        let closed = Arc::new(AtomicBool::new(false));
        let worker_closed = closed.clone();
        let updates = Arc::new(Mutex::new(None));
        let publish = updates.clone();
        let progress = Arc::new(Mutex::new(PreviewProgress::default()));
        let worker_progress = progress.clone();
        let delivered = Arc::new(AtomicU64::new(0));
        let worker_delivered = delivered.clone();
        let coalesced = Arc::new(AtomicU64::new(0));
        let worker_coalesced = coalesced.clone();
        let worker = thread::Builder::new()
            .name("oq-editor-preview".into())
            .spawn(move || run(render, validation_receiver, publish, worker_latest, worker_closed,
                worker_progress, worker_delivered, worker_coalesced))
            .map_err(|e| e.to_string())?;
        Ok((
            Self {
                validation_sender,
                latest,
                closed,
                updates,
                progress,
                delivered,
                coalesced,
                worker: Some(worker),
                #[cfg(target_os = "macos")]
                render_info: info.clone(),
                backend,
            },
            config,
            format!(
                "{} · {} · resident video only",
                info.adapter_name, info.backend
            ),
        ))
    }
    #[cfg(target_os = "macos")]
    pub fn export_device(&self) -> Result<RenderDevice, String> {
        RenderDevice::offscreen_session(&self.backend, self.render_info.clone())
    }
    pub fn request(&self, request: PreviewRequest) -> Result<(), String> {
        self.command(PreviewCommand::Exact(request))
    }
    pub fn play(&self, request: PlaybackRequest) -> Result<(), String> {
        self.command(PreviewCommand::Play(request))
    }
    fn command(&self, command: PreviewCommand) -> Result<(), String> {
        if self.closed.load(Ordering::Acquire) {
            return Err("Preview worker is closed".into());
        }
        let mut latest = self.latest.lock().map_err(|_| "Preview intent lock is poisoned")?;
        if latest.replace(command).is_some() {
            self.coalesced.fetch_add(1, Ordering::Relaxed);
        }
        drop(latest);
        if let Some(worker) = &self.worker { worker.thread().unpark(); }
        Ok(())
    }
    /// Preflight on the shared GPU owner without replacing the active graph or source.
    pub fn validate_pipeline(
        &self,
        pipeline: ClipPipeline,
        width: u32,
        height: u32,
    ) -> Result<mpsc::Receiver<Result<(), String>>, String> {
        if self.closed.load(Ordering::Acquire) {
            return Err("Preview worker is closed".into());
        }
        let (result, receiver) = mpsc::channel();
        self.validation_sender
            .try_send(PipelineValidation { pipeline, width, height, result })
            .map_err(|error| format!("Preview validation unavailable: {error}"))?;
        if let Some(worker) = &self.worker { worker.thread().unpark(); }
        Ok(receiver)
    }
    pub fn take_update(&self) -> Option<PreviewUpdate> {
        self.updates.lock().ok()?.take()
    }
    pub fn progress(&self) -> PreviewProgress {
        let mut progress = self.progress.lock().map(|value| value.clone()).unwrap_or_default();
        progress.displayed = self.delivered.load(Ordering::Relaxed);
        progress
    }
    pub fn acknowledge_presented(&self) {
        self.delivered.fetch_add(1, Ordering::Relaxed);
    }
    pub fn shutdown(&mut self) {
        self.closed.store(true, Ordering::Release);
        if let Some(worker) = &self.worker { worker.thread().unpark(); }
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

fn pipeline_graph(
    pipeline: &ClipPipeline,
    width: u32,
    height: u32,
) -> Result<open_quartz::Graph, String> {
    let mut graph = pipeline.graph().clone();
    let output = graph.nodes.iter_mut().find(|node| node.id == OUTPUT_ID)
        .ok_or("Clip pipeline has no output node")?;
    output.data.auto_size = Some(false);
    output.data.width = Some(width);
    output.data.height = Some(height);
    Ok(graph)
}

fn graph(request: &PreviewRequest) -> Result<open_quartz::Graph, String> {
    if !request.layers.is_empty() { return composition::graph(request); }
    let mut graph = pipeline_graph(&request.pipeline, request.width, request.height)?;
    let code = if let Some(asset) = &request.asset {
        let t = request.transform;
        if graph.edges.iter().any(|edge| edge.target == OUTPUT_ID && edge.source == SOURCE_ID) {
            // Keep the original transform's f64 constants and WGSL byte-for-byte for
            // the default source -> output graph, including contain and exposure.
            let fit = ((request.width as f64 / asset.width as f64)
                .min(request.height as f64 / asset.height as f64))
                * f64::from(t.scale);
            let fit_x = asset.width as f64 * fit / request.width as f64;
            let fit_y = asset.height as f64 * fit / request.height as f64;
            format!("@fragment fn main(@location(0) uv:vec2f)->@location(0) vec4f {{ let p=(uv-vec2f(0.5)-vec2f({:.8},{:.8}))/vec2f({:.8},{:.8})+vec2f(0.5); if(any(p<vec2f(0.0))||any(p>vec2f(1.0))){{return vec4f(0.0,0.0,0.0,1.0);}} let c=textureSample(image,imageSampler,p);return vec4f(clamp(c.rgb*{:.8},vec3f(0.0),vec3f(1.0)),1.0);}}",t.x,t.y,fit_x,fit_y,2.0f64.powf(f64::from(t.exposure)))
        } else {
            // Auto-size propagates actual source dimensions through the core's
            // size_source targets; explicitly sized effects may change the aspect.
            format!("@fragment fn main(@location(0) uv:vec2f)->@location(0) vec4f {{ let size=vec2f(textureDimensions(image)); let canvas=vec2f({}.0,{}.0); let fit=min(canvas.x/size.x,canvas.y/size.y)*{:.8}; let p=(uv-vec2f(0.5)-vec2f({:.8},{:.8}))/(size*fit/canvas)+vec2f(0.5); if(any(p<vec2f(0.0))||any(p>vec2f(1.0))){{return vec4f(0.0,0.0,0.0,1.0);}} let c=textureSample(image,imageSampler,p);return vec4f(clamp(c.rgb*{:.8},vec3f(0.0),vec3f(1.0)),1.0);}}",request.width,request.height,t.scale,t.x,t.y,2.0f64.powf(f64::from(t.exposure)))
        }
    } else {
        graph.nodes.retain(|node| node.id == OUTPUT_ID);
        graph.edges.clear();
        graph.nodes[0].data.inputs.clear();
        "@fragment fn main()->@location(0) vec4f{return vec4f(0.0,0.0,0.0,1.0);}".into()
    };
    graph.nodes.iter_mut().find(|node| node.id == OUTPUT_ID).unwrap().data.shader_code = code;
    // Presentation metadata is not executable state and must not recompile a
    // running graph or reopen its source when the user moves or renames a node.
    for node in &mut graph.nodes {
        node.position.x = 0.0;
        node.position.y = 0.0;
        node.data.label.clear();
    }
    Ok(graph)
}

#[derive(PartialEq)]
struct PreviewConfiguration {
    sources: BTreeMap<String, (String, String)>,
    graph: open_quartz::Graph,
}

fn validate_dimensions(pipeline: &FramePipeline, width: u32, height: u32) -> Result<(), String> {
    let limit = pipeline.backend().device.limits().max_texture_dimension_2d;
    if width == 0 || height == 0 || width > limit || height > limit {
        return Err("Sequence dimensions exceed this GPU's texture limits".into());
    }
    Ok(())
}

fn configure(
    pipeline: &mut FramePipeline,
    configured: &mut Option<PreviewConfiguration>,
    request: &PreviewRequest,
) -> Result<(bool, u64), String> {
    let key = playback::configuration(request)?;
    if configured.as_ref() == Some(&key) {
        return Ok((false, 0));
    }
    let (prepared, opened) = playback::prepare_region(pipeline, configured, request, &key, false)?;
    pipeline.commit_graph(prepared)?;
    *configured = Some(key);
    Ok((true, opened))
}
#[path = "sequence_composition.rs"]
mod composition;

#[path = "preview_playback.rs"]
mod playback;
use playback::run;

#[cfg(target_os = "macos")]
#[path = "sequence_export.rs"]
pub(crate) mod export;

#[cfg(test)]
#[path = "preview_tests.rs"]
mod preview_tests;
