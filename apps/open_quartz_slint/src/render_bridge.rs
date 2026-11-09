#[cfg(test)]
#[path = "render_bridge_replacement_tests.rs"]
mod replacement_tests;

#[cfg(test)]
#[path = "inference_render_tests.rs"]
mod inference_tests;

use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use open_quartz::media::native::{NativeVideoConfig, NativeVideoSourceKind};
use open_quartz::{Graph, NativePlayer};
use open_quartz::rendering::GpuOutputHandle;
use slint::wgpu_30::WGPUConfiguration;
use std::path::PathBuf;

use crate::inference_runtime::{host_resource_plan, LiveInferenceState, ModelMessage, InferenceRuntime};
use std::collections::BTreeMap;
pub struct RenderUpdate {
    generation: u64,
    pub status: Option<String>,
    pub texture: Option<GpuOutputHandle>,
    pub clear_image: bool,
    pub inference: Option<BTreeMap<String, LiveInferenceState>>,
    pub inference_metrics: Option<open_quartz::player::InferenceMetrics>,
    pub previews: Option<BTreeMap<String, GpuOutputHandle>>,
    pub node_error: Option<open_quartz::SdkError>,
    pub metrics: Option<PlaybackMetrics>,
    #[cfg(test)]
    rendered_revision: Option<u32>,
}

impl RenderUpdate {
    fn new(generation: u64) -> Self {
        Self {
            generation,
            status: None,
            texture: None,
            clear_image: false,
            inference: None,
            inference_metrics: None,
            previews: None,
            node_error: None,
            metrics: None,
            #[cfg(test)]
            rendered_revision: None,
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct PlaybackMetrics {
    pub timeline_ns: u64,
    pub fps: f64,
}

struct RenderMeter {
    metrics: PlaybackMetrics,
    window_started: Instant,
    published: Instant,
    frames: u64,
}

impl RenderMeter {
    fn new(now: Instant) -> Self {
        Self { metrics: PlaybackMetrics::default(), window_started: now, published: now, frames: 0 }
    }

    fn frame(&mut self, now: Instant, timeline_ns: u64, has_output: bool) -> Option<PlaybackMetrics> {
        self.metrics.timeline_ns = timeline_ns;
        self.frames += u64::from(has_output);
        let window = now.saturating_duration_since(self.window_started);
        if window >= Duration::from_millis(500) {
            self.metrics.fps = self.frames as f64 / window.as_secs_f64();
            self.frames = 0;
            self.window_started = now;
        }
        if now.saturating_duration_since(self.published) >= Duration::from_millis(100) {
            self.published = now;
            Some(self.metrics)
        } else { None }
    }

    fn suspend(&mut self, now: Instant, reset_time: bool) -> PlaybackMetrics {
        if reset_time { self.metrics.timeline_ns = 0; }
        self.metrics.fps = 0.0;
        self.frames = 0;
        self.window_started = now;
        self.published = now;
        self.metrics
    }
}

fn publish_metrics(updates: &Mutex<Option<RenderUpdate>>, generation: u64, metrics: PlaybackMetrics) {
    if let Ok(mut pending) = updates.lock() {
        let update = pending.get_or_insert_with(|| RenderUpdate::new(generation));
        if update.generation != generation { *update = RenderUpdate::new(generation); }
        update.metrics = Some(metrics);
    }
}

#[derive(Clone)]
pub(crate) struct ImageUpload {
    pub key: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
}
pub(crate) enum Command {
    SetGraph {
        generation: u64,
        graph: Graph,
        reset_resources: bool,
        images: BTreeMap<String, ImageUpload>,
        play: bool,
    },
    LoadInference {
        generation: u64,
        node_id: String,
        path: String,
    },
    UnloadInference {
        generation: u64,
        node_id: String,
    },
    CapturePng {
        generation: u64,
        node_id: String,
        path: PathBuf,
        response: mpsc::Sender<Result<(), String>>,
    },
    Playback {
        generation: u64,
        action: String,
    },
    SelectPreview {
        generation: u64,
        node_id: Option<String>,
    },
    Model(ModelMessage),
    Pause,
    Shutdown,
}

pub struct RenderBridge {
    sender: mpsc::Sender<Command>,
    updates: Arc<Mutex<Option<RenderUpdate>>>,
    generation: u64,
    accepting_frames: bool,
    reset_resources: bool,
    worker: Option<JoinHandle<()>>,
}

impl RenderBridge {
    /// Prepare the host and the exact same GPU handles for Slint before creating a window.
    pub fn initialize() -> Result<(Self, WGPUConfiguration), String> {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = if cfg!(windows) {
            wgpu::Backends::DX12
        } else {
            wgpu::Backends::PRIMARY
        };
        let instance = wgpu::Instance::new(descriptor);
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            ..Default::default()
        }))
        .map_err(|error| {
            format!("Cannot initialize native rendering: GPU adapter request failed: {error}")
        })?;
        let mut required_features = wgpu::Features::empty();
        #[cfg(target_os = "macos")]
        if adapter.features().contains(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM) {
            required_features |= wgpu::Features::TEXTURE_FORMAT_16BIT_NORM;
        }
        #[cfg(windows)]
        {
            required_features |= adapter.features() & wgpu::Features::TEXTURE_FORMAT_NV12;
            let p010 = wgpu::Features::TEXTURE_FORMAT_P010 | wgpu::Features::TEXTURE_FORMAT_16BIT_NORM;
            if adapter.features().contains(p010) { required_features |= p010; }
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("open-quartz-slint-shared-device"),
            required_features,
            ..Default::default()
        }))
        .map_err(|error| {
            format!("Cannot initialize native rendering: GPU device request failed: {error}")
        })?;
        let (host, info) = NativePlayer::from_shared_wgpu(device.clone(), queue.clone(), &adapter)?;
        let description = format!("{} · {}", info.adapter_name, info.backend);
        let configuration = WGPUConfiguration::Manual {
            instance,
            adapter,
            device,
            queue,
        };
        let (sender, receiver) = mpsc::channel();
        let updates = Arc::new(Mutex::new(None));
        let worker_updates = updates.clone();
        let worker_sender = sender.clone();
        let worker = thread::Builder::new()
            .name("open-quartz-render".to_owned())
            .spawn(move || {
                render_worker(host, description, receiver, worker_updates, worker_sender)
            })
            .map_err(|error| format!("Cannot start native render thread: {error}"))?;
        Ok((
            Self {
                sender,
                updates,
                generation: 0,
                accepting_frames: true,
                reset_resources: true,
                worker: Some(worker),
            },
            configuration,
        ))
    }

    pub fn set_graph(
        &mut self,
        graph: Graph,
        images: BTreeMap<String, ImageUpload>,
        play: bool,
    ) -> Result<(), String> {
        self.generation = self.generation.wrapping_add(1);
        self.accepting_frames = true;
        self.sender
            .send(Command::SetGraph {
                generation: self.generation,
                graph,
                reset_resources: std::mem::take(&mut self.reset_resources),
                images,
                play,
            })
            .map_err(|_| "Native render thread is no longer available".to_owned())
    }

    pub fn begin_project(&mut self) {
        self.reset_resources = true;
    }

    pub fn load_inference(&self, node_id: String, path: String) -> Result<(), String> {
        self.sender
            .send(Command::LoadInference {
                generation: self.generation,
                node_id,
                path,
            })
            .map_err(|_| "Native render thread is no longer available".to_owned())
    }

    pub fn unload_inference(&self, node_id: String) -> Result<(), String> {
        self.sender
            .send(Command::UnloadInference {
                generation: self.generation,
                node_id,
            })
            .map_err(|_| "Native render thread is no longer available".to_owned())
    }
    pub fn capture_png(
        &self,
        node_id: String,
        path: PathBuf,
    ) -> Result<mpsc::Receiver<Result<(), String>>, String> {
        let (response, receiver) = mpsc::channel();
        self.sender
            .send(Command::CapturePng {
                generation: self.generation,
                node_id,
                path,
                response,
            })
            .map_err(|_| "Native render thread is no longer available".to_owned())?;
        Ok(receiver)
    }

    pub fn set_playback(&self, action: &str) -> Result<(), String> {
        if !matches!(action, "play" | "pause" | "stop") {
            return Err("Unknown playback action".into());
        }
        self.sender
            .send(Command::Playback {
                generation: self.generation,
                action: action.to_owned(),
            })
            .map_err(|_| "Native render thread is no longer available".to_owned())
    }

    pub fn select_preview(&self, node_id: Option<String>) -> Result<(), String> {
        self.sender
            .send(Command::SelectPreview {
                generation: self.generation,
                node_id,
            })
            .map_err(|_| "Native render thread is no longer available".to_owned())
    }

    pub fn take_update(&self) -> Option<RenderUpdate> {
        let update = self.updates.lock().ok()?.take()?;
        (self.accepting_frames && update.generation == self.generation).then_some(update)
    }

    pub fn pause_after_import_error(&mut self) {
        self.accepting_frames = false;
        let _ = self.sender.send(Command::Pause);
    }
}

impl Drop for RenderBridge {
    fn drop(&mut self) {
        let _ = self.sender.send(Command::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn publish_status(
    updates: &Mutex<Option<RenderUpdate>>,
    generation: u64,
    status: String,
    clear_image: bool,
) {
    if let Ok(mut pending) = updates.lock() {
        let update = pending.get_or_insert_with(|| RenderUpdate::new(generation));
        if update.generation != generation {
            *update = RenderUpdate::new(generation);
        }
        update.status = Some(status);
        if clear_image {
            update.texture = None;
            update.clear_image = true;
        }
    }
}

fn publish_failure(
    updates: &Mutex<Option<RenderUpdate>>,
    generation: u64,
    context: &str,
    error: &str,
) {
    if let Ok(mut pending) = updates.lock() {
        let update = pending.get_or_insert_with(|| RenderUpdate::new(generation));
        if update.generation != generation {
            *update = RenderUpdate::new(generation);
        }
        let parsed = serde_json::from_str::<open_quartz::SdkError>(error).ok();
        update.node_error = parsed.clone();
        update.status = Some(format!(
            "{context}: {}",
            parsed.as_ref().map_or(error, |error| error.message.as_str())
        ));
        update.texture = None;
        update.clear_image = true;
        update.previews = Some(BTreeMap::new());
    }
}

fn publish_graph_rejection(updates: &Mutex<Option<RenderUpdate>>, generation: u64, error: &str) {
    if let Ok(mut pending) = updates.lock() {
        let update = pending.get_or_insert_with(|| RenderUpdate::new(generation));
        if update.generation != generation {
            *update = RenderUpdate::new(generation);
        }
        let parsed = serde_json::from_str::<open_quartz::SdkError>(error).ok();
        update.node_error = parsed.clone();
        update.status = Some(format!(
            "Graph replacement rejected; previous graph retained: {}",
            parsed.as_ref().map_or(error, |error| error.message.as_str())
        ));
    }
}

fn publish_inference(updates: &Mutex<Option<RenderUpdate>>, generation: u64, inference: &InferenceRuntime) {
    if let Ok(mut pending) = updates.lock() {
        let update = pending.get_or_insert_with(|| RenderUpdate::new(generation));
        if update.generation != generation {
            *update = RenderUpdate::new(generation);
        }
        update.inference = Some(inference.states.clone());
    }
}

fn publish_inference_metrics(
    updates: &Mutex<Option<RenderUpdate>>,
    generation: u64,
    metrics: open_quartz::player::InferenceMetrics,
) {
    if let Ok(mut pending) = updates.lock() {
        let update = pending.get_or_insert_with(|| RenderUpdate::new(generation));
        if update.generation != generation {
            *update = RenderUpdate::new(generation);
        }
        update.inference_metrics = Some(metrics);
    }
}

fn render_worker(
    mut host: NativePlayer,
    description: String,
    receiver: mpsc::Receiver<Command>,
    updates: Arc<Mutex<Option<RenderUpdate>>>,
    sender: mpsc::Sender<Command>,
) {
    let mut generation = 0;
    let mut running = false;
    let mut render_failed = false;
    let mut first_frame = true;
    let mut inference = InferenceRuntime::default();
    let mut image_keys = BTreeMap::<String, u64>::new();
    let mut video_configs = BTreeMap::<String, NativeVideoConfig>::new();
    let mut video_pipelines = BTreeMap::<String, String>::new();
    let mut current_graph: Option<Graph> = None;
    let mut graph_has_rendered = false;
    let mut source_preview_nodes = std::collections::BTreeSet::<String>::new();
    let mut stopped = true;
    let mut ready_to_start = false;
    let mut playback_requested = false;
    let mut preview_renderer: Option<String> = None;
    let mut preview_nodes = Vec::<String>::new();
    let mut preview_tick = Instant::now();
    let mut pending_video_previews = BTreeMap::<String, Instant>::new();
    let mut meter = RenderMeter::new(Instant::now());
    loop {
        if inference.reap_workers() {
            publish_inference(&updates, generation, &inference);
        }
        let command = if running || inference.is_loading() || !pending_video_previews.is_empty() {
            match receiver.recv_timeout(Duration::from_millis(16)) {
                Ok(command) => Some(command),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match receiver.recv() {
                Ok(command) => Some(command),
                Err(_) => break,
            }
        };
        match command {
            Some(Command::Shutdown) => break,
            Some(Command::Pause) => {
                let _ = host.pause();
                playback_requested = false;
                running = false;
                pending_video_previews.clear();
                publish_metrics(&updates, generation, meter.suspend(Instant::now(), false));
            }
            Some(Command::SetGraph {
                generation: next_generation,
                graph,
                reset_resources,
                images,
                play,
            }) => {
                let preparation: Result<_, String> = (|| {
                    let resource_plan = host_resource_plan(&graph)?;
                    let mut prepared = host.prepare_graph(&graph)?;
                    let next_video_configs = graph_video_configs(&resource_plan.intents);
                    let mut next_video_pipelines = video_pipelines.clone();
                    let mut next_video_previews = BTreeMap::new();
                    for (node_id, config) in &video_configs {
                        if reset_resources || next_video_configs.get(node_id) != Some(config) {
                            prepared.stage_detach_video(node_id);
                            next_video_pipelines.remove(node_id);
                        }
                    }
                    for (node_id, config) in &next_video_configs {
                        if reset_resources || video_configs.get(node_id) != Some(config)
                            || (!play && !host.has_output_texture(node_id))
                        {
                            let info = prepared.stage_video(node_id, config.clone(), !play)?;
                            if !play {
                                next_video_previews.insert(node_id.clone(), Instant::now());
                            }
                            let path = match &info.fallback_reason {
                                Some(reason) => format!("{}: {} (software fallback: {reason})", node_id, info.decoder),
                                None => format!("{}: {}", node_id, info.decoder),
                            };
                            next_video_pipelines.insert(node_id.clone(), path);
                        }
                    }
                    for (node_id, key) in &image_keys {
                        if reset_resources || images.get(node_id).is_none_or(|image| image.key != *key) {
                            prepared.stage_remove_texture(node_id);
                        }
                    }
                    for (node_id, image) in &images {
                        if reset_resources || image_keys.get(node_id) != Some(&image.key) {
                            prepared.stage_image(node_id, &image.rgba, image.width, image.height)?;
                        }
                    }
                    host.commit_graph(prepared)?;
                    Ok((resource_plan, next_video_configs, next_video_pipelines, next_video_previews))
                })();
                let (resource_plan, next_video_configs, next_video_pipelines, next_video_previews) = match preparation {
                    Ok(prepared) => prepared,
                    Err(error) => {
                        // UI generations identify requests; the accepted core graph revision stays intact.
                        generation = next_generation;
                        publish_graph_rejection(&updates, generation, &error);
                        continue;
                    }
                };
                generation = next_generation;
                graph_has_rendered = false;
                source_preview_nodes = graph.nodes().iter().filter(|node| {
                    matches!(node.node_type, open_quartz::NodeType::Input | open_quartz::NodeType::Inference)
                }).map(|node| node.id.clone()).collect();
                first_frame = true;
                running = false;
                render_failed = false;
                stopped = false;
                ready_to_start = !play;
                playback_requested = play;
                publish_metrics(&updates, generation, meter.suspend(Instant::now(), true));
                pending_video_previews = next_video_previews;
                preview_nodes = graph.nodes().iter().filter(|node| {
                    matches!(node.node_type, open_quartz::NodeType::Shader | open_quartz::NodeType::Constant | open_quartz::NodeType::Renderer)
                        || node.data.outputs.iter().any(|port| port.data_type == open_quartz::DataType::Sampler2d)
                }).map(|node| node.id.clone()).collect();
                preview_tick = Instant::now() - Duration::from_millis(100);
                video_configs = next_video_configs;
                video_pipelines = next_video_pipelines;
                image_keys = images.iter().map(|(node_id, image)| (node_id.clone(), image.key)).collect();
                for path in video_pipelines.values() {
                    eprintln!("[video] {path}");
                }
                if reset_resources {
                    inference.clear(&mut host);
                }
                inference.sync(&resource_plan.intents, &mut host, false);
                let result: Result<bool, String> = (|| {
                    // Frame events coalesce in the engine; clear graph/state history only on replacement.
                    let _ = host.drain_events();
                    if graph.nodes().is_empty() || !playback_requested {
                        return Ok(false);
                    }
                    host.start_playback()?;
                    Ok(true)
                })();
                match result {
                    Ok(should_run) => {
                        running = should_run;
                        publish_metrics(&updates, generation, meter.suspend(Instant::now(), true));
                        let status = if running {
                            format!("{description} · Graph ready; waiting for renderer output")
                        } else if graph.nodes().is_empty() {
                            format!("{description} · Ready; empty graph has no renderer output")
                        } else {
                            format!("{description} · Graph ready; press Play to render")
                        };
                        let status = if video_pipelines.is_empty() { status } else {
                            format!("{status} · {}", video_pipelines.values().cloned().collect::<Vec<_>>().join("; "))
                        };
                        publish_status(&updates, generation, status, true);
                        current_graph = Some(graph);
                    }
                    Err(error) => {
                        render_failed = true;
                        pending_video_previews.clear();
                        publish_failure(&updates, generation, "Graph initialization failed", &error);
                    }
                }
                publish_inference(&updates, generation, &inference);
            }
            Some(Command::LoadInference {
                generation: requested,
                node_id,
                path,
            }) if requested == generation => {
                host.unload_inference(&node_id);
                if let Err(error) = inference.load(node_id, path, sender.clone()) {
                    publish_status(
                        &updates,
                        generation,
                        format!("Inference load refused: {error}"),
                        true,
                    );
                } else {
                    publish_status(
                        &updates,
                        generation,
                        "Loading Inference model resource…".into(),
                        true,
                    );
                }
                publish_inference(&updates, generation, &inference);
            }
            Some(Command::UnloadInference {
                generation: requested,
                node_id,
            }) if requested == generation => {
                inference.unload(&node_id, &mut host);
                publish_status(&updates, generation, "Inference session unloaded.".into(), true);
                publish_inference(&updates, generation, &inference);
            }
            Some(Command::CapturePng {
                generation: requested,
                node_id,
                path,
                response,
            }) => {
                let result = if requested == generation {
                    host.read_output(&node_id)
                        .and_then(|payload| save_output_png(&path, &payload))
                } else {
                    Err("Capture belongs to a replaced project".into())
                };
                let _ = response.send(result);
            }
            Some(Command::SelectPreview {
                generation: requested,
                node_id,
            }) if requested == generation => {
                preview_renderer = node_id;
                first_frame = true;
                publish_status(
                    &updates,
                    generation,
                    "Waiting for selected node preview…".into(),
                    true,
                );
                if !running {
                    if let Some(node_id) = preview_renderer.as_deref()
                        .filter(|id| graph_has_rendered || source_preview_nodes.contains(*id))
                    {
                        match host.preview_source(node_id) {
                            Ok(output) => {
                                if let Ok(mut pending) = updates.lock() {
                                    let update = pending
                                        .get_or_insert_with(|| RenderUpdate::new(generation));
                                    update.texture = Some(output);
                                    update.clear_image = false;
                                }
                            }
                            Err(error) => publish_status(&updates, generation, error, true),
                        }
                    }
                }
            }
            Some(Command::SelectPreview { .. }) => {}
            Some(Command::Playback {
                generation: requested,
                action,
            }) if requested == generation => {
                let result = match action.as_str() {
                    "pause" => {
                        if running {
                            host.pause()
                        } else {
                            Ok(())
                        }
                    }
                    "stop" => (|| {
                        host.stop()?;
                        // Stop may race the initial decode. Keep any real frame
                        // already decoded; otherwise request just that preview again.
                        host.upload_video_frames()?;
                        for (node_id, started) in &mut pending_video_previews {
                            if !host.has_output_texture(node_id) {
                                if let Some(config) = video_configs.get(node_id) {
                                    host.attach_video_preview(node_id, config.clone())?;
                                    *started = Instant::now();
                                }
                            }
                        }
                        Ok(())
                    })(),
                    _ if running => Ok(()),
                    _ if stopped => (|| {
                        let graph = current_graph
                            .as_ref()
                            .ok_or("No graph is ready for playback")?;
                        host.set_graph_snapshot(graph)?;
                        for (node_id, config) in &video_configs {
                            host.detach_video(node_id)?;
                            host.attach_video(node_id, config.clone())?;
                        }
                        host.start_playback()
                    })(),
                    _ if ready_to_start => host.start_playback(),
                    _ => host.resume(),
                };
                match result {
                    Ok(()) => {
                        running = action == "play";
                        playback_requested = action == "play";
                        if action == "stop" {
                            stopped = true;
                        }
                        if action == "play" {
                            stopped = false;
                            ready_to_start = false;
                        }
                        first_frame = true;
                        publish_status(&updates, generation, format!("Playback: {action}"), false);
                        publish_metrics(&updates, generation, meter.suspend(Instant::now(), action == "stop"));
                    }
                    Err(error) => publish_status(&updates, generation, error, false),
                }
            }
            Some(Command::Playback { .. }) => {}
            Some(Command::Model(message)) => {
                let node_id = message.node_id().to_owned();
                if inference.receive(message, &mut host) {
                    let result = if running || !playback_requested {
                        Ok(())
                    } else {
                        host.resume()
                    };
                    match result {
                        Ok(()) => {
                            running = playback_requested;
                            first_frame = true;
                            publish_status(
                                &updates,
                                generation,
                                "Inference model attached; waiting for inference output…".into(),
                                true,
                            );
                        }
                        Err(error) => {
                            running = false;
                            render_failed = true;
                            inference.render_failed(&error);
                            publish_failure(&updates, generation, "Resume failed", &error);
                        }
                    }
                } else if let Some(state) = inference.states.get(&node_id) {
                    if !state.loading && !state.attached {
                        publish_status(&updates, generation, state.status.clone(), true);
                    }
                }
                publish_inference(&updates, generation, &inference);
            }
            Some(Command::LoadInference { .. } | Command::UnloadInference { .. }) => {}
            None => {}
        }
        publish_inference_metrics(&updates, generation, host.inference_metrics());
        // A source preview uploads decoded pixels only. It must never call
        // render_next/start_playback or consume the graph's first frame work.
        if !running && !render_failed && !pending_video_previews.is_empty() {
            let result = host.upload_video_frames().and_then(|()| {
                let mut ready = false;
                pending_video_previews.retain(|id, _| {
                    let pending = !host.has_output_texture(id);
                    ready |= !pending;
                    pending
                });
                if ready {
                    preview_tick = Instant::now() - Duration::from_millis(100);
                }
                if let Some((id, _)) = pending_video_previews.iter()
                    .find(|(_, started)| started.elapsed() >= Duration::from_secs(10))
                {
                    return Err(format!("Video node {id} did not decode a preview frame within 10 seconds"));
                }
                Ok(())
            });
            if let Err(error) = result {
                let _ = host.pause();
                pending_video_previews.clear();
                publish_status(&updates, generation, format!("Video preview failed: {error}"), false);
            }
        } else if running {
            pending_video_previews.clear();
        }
        if !render_failed && preview_tick.elapsed() >= Duration::from_millis(100) {
            let previews = preview_nodes.iter()
                .filter(|id| graph_has_rendered || source_preview_nodes.contains(*id))
                .filter_map(|id| {
                host.preview_source(id).ok().map(|output| (id.clone(), output))
            }).collect();
            if let Ok(mut pending) = updates.lock() {
                let update = pending.get_or_insert_with(|| RenderUpdate::new(generation));
                if update.generation != generation { *update = RenderUpdate::new(generation); }
                update.previews = Some(previews);
                if !running {
                    if let Some(output) = preview_renderer.as_deref()
                        .filter(|id| graph_has_rendered || source_preview_nodes.contains(*id))
                        .and_then(|id| host.preview_source(id).ok())
                    {
                        update.texture = Some(output);
                        update.clear_image = false;
                    }
                }
            }
            preview_tick = Instant::now();
        }
        if !running {
            continue;
        }
        #[cfg(test)]
        let mut rendered_revision = None;
        let result: Result<Option<GpuOutputHandle>, String> = (|| {
            let rendered = host.render_next()?;
            // Prepared shader targets are allocations, not completed previews.
            // A pending inference frame can succeed without producing an output.
            graph_has_rendered |= rendered.width > 0 && rendered.height > 0;
            #[cfg(test)]
            { rendered_revision = Some(rendered.revision); }
            if let Some(metrics) = meter.frame(Instant::now(), rendered.timeline_ns, rendered.width > 0 && rendered.height > 0) {
                publish_metrics(&updates, generation, metrics);
            }
            if rendered.width == 0 || rendered.height == 0 {
                return Ok(None);
            }
            let output = match host.preview_source(
                preview_renderer
                    .as_deref()
                    .unwrap_or(&rendered.output_node_id),
            ) {
                Ok(output) => output,
                // An unexecuted/disconnected inspector node may not have a texture.
                // Choosing it must not pause a healthy graph's Renderer output.
                Err(_) if preview_renderer.is_some() => return Ok(None),
                Err(error) => return Err(error),
            };
            Ok(Some(output))
        })();
        publish_inference_metrics(&updates, generation, host.inference_metrics());
        let events = host.take_inference_events();
        if !events.is_empty() {
            for event in events {
                inference.output(event);
            }
            publish_inference(&updates, generation, &inference);
        }
        match result {
            Ok(Some(output)) => {
                if let Ok(mut pending) = updates.lock() {
                    let update = pending.get_or_insert_with(|| RenderUpdate::new(generation));
                    if update.generation != generation {
                        *update = RenderUpdate::new(generation);
                    }
                    let (width, height) = (output.width, output.height);
                    update.texture = Some(output);
                    #[cfg(test)]
                    { update.rendered_revision = rendered_revision; }
                    if first_frame {
                        update.status = Some(format!("{description} · Rendering {width}×{height} · shared GPU texture"));
                        first_frame = false;
                    }
                }
            }
            Ok(None) => {}
            Err(error) => {
                let _ = host.pause();
                running = false;
                render_failed = true;
                publish_metrics(&updates, generation, meter.suspend(Instant::now(), false));
                inference.render_failed(&error);
                publish_inference(&updates, generation, &inference);
                publish_failure(&updates, generation, "Rendering stopped", &error);
            }
        }
    }
    if host.has_graph() {
        let _ = host.stop();
    }
}

fn graph_video_configs(
    intents: &[open_quartz::resources::plan::HostResourceIntent],
) -> BTreeMap<String, NativeVideoConfig> {
    use open_quartz::resources::plan::{HostResourceIntent, VideoSourceKind};
    intents
        .iter()
        .filter_map(|intent| match intent {
            HostResourceIntent::AttachVideo {
                node_id,
                kind,
                source,
                looping,
                playback_rate,
                ..
            } => Some((
                node_id.clone(),
                NativeVideoConfig {
                    kind: match kind {
                        VideoSourceKind::File => NativeVideoSourceKind::File,
                        VideoSourceKind::Camera => NativeVideoSourceKind::Camera,
                    },
                    source: source.clone(),
                    looping: *looping,
                    playback_rate: *playback_rate,
                },
            )),
            _ => None,
        })
        .collect()
}

fn save_output_png(path: &std::path::Path, payload: &[u8]) -> Result<(), String> {
    if payload.len() < 8 {
        return Err("Native output payload has no dimensions".into());
    }
    let width = u32::from_le_bytes(payload[0..4].try_into().unwrap());
    let height = u32::from_le_bytes(payload[4..8].try_into().unwrap());
    image::save_buffer_with_format(
        path,
        &payload[8..],
        width,
        height,
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .map_err(|error| format!("Cannot save {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_rate_excludes_missing_outputs_and_pause_time() {
        let now = Instant::now();
        let mut meter = RenderMeter::new(now);
        for index in 1..=10 {
            meter.frame(now + Duration::from_millis(index * 50), index * 50_000_000, index % 2 == 0);
        }
        assert_eq!(meter.metrics.fps, 10.0);
        assert_eq!(meter.metrics.timeline_ns, 500_000_000);
        let paused = meter.suspend(now + Duration::from_secs(1), false);
        assert_eq!(paused.timeline_ns, 500_000_000);
        assert_eq!(paused.fps, 0.0);
        meter.suspend(now + Duration::from_secs(10), false);
        meter.frame(now + Duration::from_millis(10_500), 1_000_000_000, true);
        assert_eq!(meter.metrics.fps, 2.0);
        let stopped = meter.suspend(now + Duration::from_secs(11), true);
        assert_eq!(stopped.timeline_ns, 0);
        assert_eq!(stopped.fps, 0.0);
    }

    #[test]
    fn failure_retains_node_identity_and_clears_failed_generation_previews() {
        let updates = Mutex::new(Some(RenderUpdate::new(1)));
        let error = open_quartz::SdkError::new(
            open_quartz::SdkErrorCode::InvalidResource, "Invalid WGSL type",
        ).for_node("shader_7");
        publish_failure(&updates, 2, "Rendering stopped", &error.to_json());
        let update = updates.lock().unwrap();
        let update = update.as_ref().unwrap();
        assert_eq!(update.generation, 2);
        assert_eq!(update.node_error.as_ref(), Some(&error));
        assert!(update.clear_image && update.texture.is_none());
        assert!(update.previews.as_ref().unwrap().is_empty());
    }

    #[test]
    fn preparing_a_graph_waits_for_explicit_play_and_preserves_first_frame_work() {
        use open_quartz::{Environment, GraphCommand, NodeFactoryRequest, NodeId, OpenQuartz, PortId, PortKey};
        let sdk = OpenQuartz::new(Environment::headless());
        let mut project = sdk.create_project("Explicit playback");
        let (shader, _) = project.create_graph_node(NodeFactoryRequest::Shader {
            position: None,
            code: "@fragment fn main() -> @location(0) vec4f { return vec4f(1.0); }".into(),
            label: "Solid".into(), template_name: None, shader_template_id: None,
        }, project.graph_revision()).unwrap();
        let (renderer, _) = project.create_graph_node(NodeFactoryRequest::Renderer { position: None }, project.graph_revision()).unwrap();
        project.apply_graph_command(GraphCommand::Connect {
            source: PortKey::new(NodeId::new(&shader.id), PortId::new(&shader.data.outputs[0].id)),
            target: PortKey::new(NodeId::new(&renderer.id), PortId::new(&renderer.data.inputs[0].id)),
        }, project.graph_revision()).unwrap();
        let (source_image, _) = project.create_graph_node(NodeFactoryRequest::Input {
            position: None, data_type: open_quartz::DataType::Sampler2d,
            input_mode: Some(open_quartz::InputMode::Image),
        }, project.graph_revision()).unwrap();
        let images = [(source_image.id.clone(), ImageUpload {
            key: 1, width: 2, height: 2, rgba: Arc::new([17, 34, 51, 255].repeat(4)),
        })].into_iter().collect();
        let (mut bridge, _configuration) = RenderBridge::initialize().unwrap();
        let wait_for = |bridge: &RenderBridge, predicate: &dyn Fn(&RenderUpdate) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(update) = bridge.take_update() {
                    assert!(update.node_error.is_none(), "{:?}", update.node_error);
                    if predicate(&update) { return update; }
                }
                assert!(Instant::now() < deadline, "render worker did not reach expected state");
                thread::sleep(Duration::from_millis(5));
            }
        };
        bridge.set_graph(project.graph_snapshot(), images, false).unwrap();
        let update = wait_for(&bridge, &|update| update.status.as_ref().is_some_and(|status| status.contains("press Play")));
        assert!(update.texture.is_none());
        // Force the periodic preview publication after selecting a prepared shader.
        thread::sleep(Duration::from_millis(110));
        bridge.select_preview(Some(shader.id.clone())).unwrap();
        let update = wait_for(&bridge, &|update| update.previews.is_some());
        let previews = update.previews.unwrap();
        assert!(previews.contains_key(&source_image.id), "uploaded source must remain available before Play");
        assert!(!previews.contains_key(&shader.id), "an allocated shader target is not a rendered preview");
        assert!(!previews.contains_key(&renderer.id));
        assert!(update.texture.is_none());
        bridge.select_preview(Some(source_image.id.clone())).unwrap();
        wait_for(&bridge, &|update| update.texture.is_some());
        let capture = std::env::temp_dir().join(format!("open-quartz-explicit-play-{}.png", std::process::id()));
        bridge.capture_png(source_image.id.clone(), capture.clone()).unwrap()
            .recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        let source_pixels = image::open(&capture).unwrap().to_rgba8();
        assert_eq!(source_pixels.dimensions(), (2, 2));
        assert!(source_pixels.pixels().all(|pixel| pixel.0 == [17, 34, 51, 255]));
        bridge.select_preview(Some(shader.id.clone())).unwrap();
        let update = wait_for(&bridge, &|update| update.status.as_ref().is_some_and(|status| status.contains("Waiting for selected node")));
        assert!(update.texture.is_none() && update.clear_image);
        bridge.set_playback("play").unwrap();
        wait_for(&bridge, &|update| update.texture.is_some());
        bridge.set_playback("pause").unwrap();
        wait_for(&bridge, &|update| update.status.as_deref() == Some("Playback: pause"));
        bridge.select_preview(Some(shader.id.clone())).unwrap();
        wait_for(&bridge, &|update| update.texture.is_some());
        bridge.capture_png(shader.id.clone(), capture.clone()).unwrap()
            .recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        let rendered_pixels = image::open(&capture).unwrap().to_rgba8();
        assert!(rendered_pixels.pixels().all(|pixel| pixel.0 == [255, 255, 255, 255]));
        std::fs::remove_file(capture).unwrap();
        bridge.set_playback("stop").unwrap();
        bridge.set_graph(project.graph_snapshot(), BTreeMap::new(), false).unwrap();
        let update = wait_for(&bridge, &|update| update.status.as_ref().is_some_and(|status| status.contains("press Play")));
        assert!(update.texture.is_none());
        assert!(update.clear_image);
    }
    fn standalone_video_graph(path: &std::path::Path) -> Graph {
        let sdk = open_quartz::OpenQuartz::new(open_quartz::Environment::headless());
        let mut project = sdk.create_project("Standalone video");
        let (video, _) = project.create_graph_node(open_quartz::NodeFactoryRequest::Input {
            position: None, data_type: open_quartz::DataType::Sampler2d,
            input_mode: Some(open_quartz::InputMode::Video),
        }, project.graph_revision()).unwrap();
        let mut graph = project.graph_snapshot();
        let node = graph.nodes.iter_mut().find(|node| node.id == video.id).unwrap();
        node.data.video_source_type = Some(open_quartz::VideoSourceType::File);
        node.data.video_file_path = Some(path.to_string_lossy().into_owned());
        graph
    }

    #[test]
    fn video_without_renderer_keeps_preview_and_advances_on_play() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let path = std::env::temp_dir().join(format!("oq-standalone-{}.mp4",
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
        let ffmpeg = open_quartz::media::native::find_ffmpeg().unwrap();
        assert!(std::process::Command::new(ffmpeg).args([
            "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i",
            "testsrc2=size=64x64:rate=10:duration=2", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-y",
        ]).arg(&path).status().unwrap().success());
        let graph = standalone_video_graph(&path);
        let video = graph.nodes[0].id.clone();
        let (mut bridge, _) = RenderBridge::initialize().unwrap();
        bridge.set_graph(graph, BTreeMap::new(), false).unwrap();
        let wait_for = |playing: bool| {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Some(update) = bridge.take_update() {
                    assert!(update.node_error.is_none(), "{:?}", update.node_error);
                    assert!(!update.status.as_ref().is_some_and(|status| status.contains("failed") || status.contains("stopped:")), "{:?}", update.status);
                    if (playing && update.texture.is_some()) || (!playing && update.previews.as_ref().is_some_and(|previews| previews.contains_key(&video))) { return; }
                }
                assert!(Instant::now() < deadline, "standalone video produced no output");
                thread::sleep(Duration::from_millis(5));
            }
        };
        wait_for(false);
        let capture = path.with_extension("png");
        bridge.capture_png(video.clone(), capture.clone()).unwrap().recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        let first = image::open(&capture).unwrap().to_rgba8();
        bridge.set_playback("play").unwrap();
        wait_for(true);
        thread::sleep(Duration::from_millis(600));
        bridge.capture_png(video, capture.clone()).unwrap().recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        let later = image::open(&capture).unwrap().to_rgba8();
        assert_eq!(later.dimensions(), (64, 64));
        assert_ne!(first, later, "Play kept the stopped thumbnail instead of advancing video");
        drop(bridge);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(capture).unwrap();
    }

    #[test]
    fn choosing_video_previews_decoded_pixels_while_stopped_or_paused() {
        use open_quartz::{DataType, Environment, GraphCommand, InputMode,
            NodeFactoryRequest, NodeId, OpenQuartz, PortId, PortKey, VideoSourceType};
        use std::process::Command as ProcessCommand;
        use std::time::{SystemTime, UNIX_EPOCH};

        let directory = std::env::temp_dir().join(format!("open-quartz-video-preview-{}",
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&directory).unwrap();
        let ffmpeg = open_quartz::media::native::find_ffmpeg().unwrap();
        let red = directory.join("red.mp4");
        let blue = directory.join("blue.mp4");
        for (path, color) in [(&red, "red"), (&blue, "blue")] {
            assert!(ProcessCommand::new(&ffmpeg).args([
                "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i",
                &format!("color=c={color}:s=64x64:r=10"), "-frames:v", "8",
                "-c:v", "libx264", "-pix_fmt", "yuv420p", "-y",
            ]).arg(path).status().unwrap().success());
        }
        let sdk = OpenQuartz::new(Environment::headless());
        let mut project = sdk.create_project("Stopped video preview");
        let (video, _) = project.create_graph_node(NodeFactoryRequest::Input {
            position: None, data_type: DataType::Sampler2d, input_mode: Some(InputMode::Video),
        }, project.graph_revision()).unwrap();
        let (renderer, _) = project.create_graph_node(NodeFactoryRequest::Renderer {
            position: None,
        }, project.graph_revision()).unwrap();
        project.apply_graph_command(GraphCommand::Connect {
            source: PortKey::new(NodeId::new(&video.id), PortId::new(&video.data.outputs[0].id)),
            target: PortKey::new(NodeId::new(&renderer.id), PortId::new(&renderer.data.inputs[0].id)),
        }, project.graph_revision()).unwrap();
        let (mut bridge, _configuration) = RenderBridge::initialize().unwrap();
        let wait_for = |bridge: &RenderBridge, playing: bool| {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Some(update) = bridge.take_update() {
                    assert!(update.node_error.is_none(), "{:?}", update.node_error);
                    if playing {
                        if update.texture.is_some() { break; }
                    } else {
                        assert!(update.texture.is_none(), "preview started graph playback");
                        if update.previews.as_ref().is_some_and(|previews| previews.contains_key(&video.id)) {
                            break;
                        }
                    }
                }
                assert!(Instant::now() < deadline, "decoded preview did not arrive");
                thread::sleep(Duration::from_millis(5));
            }
        };
        let capture = directory.join("preview.png");
        // Replace equal-sized files in the same node, then choose again while paused.
        for (index, (path, channel)) in [(&red, 0), (&blue, 2), (&red, 0)].into_iter().enumerate() {
            let mut graph = project.graph_snapshot();
            let node = graph.nodes.iter_mut().find(|node| node.id == video.id).unwrap();
            node.data.video_source_type = Some(VideoSourceType::File);
            node.data.video_file_path = Some(path.to_string_lossy().into_owned());
            bridge.set_graph(graph.clone(), BTreeMap::new(), false).unwrap();
            if index == 0 {
                // An edit with unchanged video config can arrive before decoding finishes.
                bridge.set_graph(graph, BTreeMap::new(), false).unwrap();
                bridge.set_playback("stop").unwrap();
            } else if index == 2 {
                bridge.set_playback("pause").unwrap();
            }
            wait_for(&bridge, false);
            // The source stays at its initial frame even after the decoder has had time to run.
            thread::sleep(Duration::from_millis(150));
            bridge.capture_png(video.id.clone(), capture.clone()).unwrap()
                .recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
            let pixels = image::open(&capture).unwrap().to_rgba8();
            assert_eq!(pixels.dimensions(), (64, 64));
            assert!(pixels.pixels().all(|pixel| pixel[channel] > 240
                && pixel[(channel + 1) % 3] < 10 && pixel[(channel + 2) % 3] < 10));
            if let Some(update) = bridge.take_update() {
                assert!(update.texture.is_none(), "choosing a file resumed playback");
            }
            if index == 1 {
                bridge.set_playback("play").unwrap();
                wait_for(&bridge, true);
                bridge.set_playback("pause").unwrap();
            }
        }
        let mut invalid = project.graph_snapshot();
        let node = invalid.nodes.iter_mut().find(|node| node.id == video.id).unwrap();
        node.data.video_source_type = Some(VideoSourceType::File);
        node.data.video_file_path = Some(directory.join("missing.mp4").to_string_lossy().into_owned());
        let mut removed = project.graph_snapshot();
        removed.nodes.retain(|node| node.id != video.id);
        removed.edges.clear();
        bridge.set_graph(invalid, BTreeMap::new(), false).unwrap();
        bridge.capture_png(video.id.clone(), capture.clone()).unwrap()
            .recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        let update = bridge.take_update().unwrap();
        assert!(update.status.as_deref().unwrap().contains("Graph replacement rejected"));
        assert!(!update.clear_image);
        let retained = image::open(&capture).unwrap().to_rgba8();
        assert!(retained.pixels().all(|pixel| pixel[0] > 240 && pixel[1] < 10 && pixel[2] < 10));
        bridge.set_graph(removed, BTreeMap::new(), false).unwrap();
        // A capture of a nonexistent output is a command barrier without producing pixels.
        assert!(bridge.capture_png("missing-output".into(), capture.clone()).unwrap()
            .recv_timeout(Duration::from_secs(5)).unwrap().is_err());
        let update = bridge.take_update().unwrap();
        assert!(!update.status.as_deref().unwrap().contains("failed"));
        assert!(!update.previews.as_ref().is_some_and(|previews| previews.contains_key(&video.id)));
        thread::sleep(Duration::from_millis(250));
        assert!(bridge.take_update().is_none(), "worker kept polling a removed video");
        drop(bridge);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
