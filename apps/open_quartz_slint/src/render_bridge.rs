use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use open_quartz_execution::native_video::{NativeVideoConfig, NativeVideoSourceKind};
use open_quartz_native_host::NativeHost;
use open_quartz_sdk::Graph;
use slint::wgpu_30::WGPUConfiguration;
use std::path::PathBuf;

use crate::onnx_runtime::{LiveOnnxState, ModelMessage, OnnxRuntime};
use std::collections::BTreeMap;
pub struct RenderUpdate {
    generation: u64,
    pub status: Option<String>,
    pub texture: Option<wgpu::Texture>,
    pub clear_image: bool,
    pub onnx: Option<BTreeMap<String, LiveOnnxState>>,
    pub previews: Option<BTreeMap<String, wgpu::Texture>>,
    pub node_error: Option<open_quartz_sdk::SdkError>,
}

impl RenderUpdate {
    fn new(generation: u64) -> Self {
        Self {
            generation,
            status: None,
            texture: None,
            clear_image: false,
            onnx: None,
            previews: None,
            node_error: None,
        }
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
    },
    LoadOnnx {
        generation: u64,
        node_id: String,
        path: String,
    },
    UnloadOnnx {
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
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("open-quartz-slint-shared-device"),
            ..Default::default()
        }))
        .map_err(|error| {
            format!("Cannot initialize native rendering: GPU device request failed: {error}")
        })?;
        let (host, info) = NativeHost::from_shared_wgpu(device.clone(), queue.clone(), &adapter)?;
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
    ) -> Result<(), String> {
        self.generation = self.generation.wrapping_add(1);
        self.accepting_frames = true;
        self.sender
            .send(Command::SetGraph {
                generation: self.generation,
                graph,
                reset_resources: std::mem::take(&mut self.reset_resources),
                images,
            })
            .map_err(|_| "Native render thread is no longer available".to_owned())
    }

    pub fn begin_project(&mut self) {
        self.reset_resources = true;
    }

    pub fn load_onnx(&self, node_id: String, path: String) -> Result<(), String> {
        self.sender
            .send(Command::LoadOnnx {
                generation: self.generation,
                node_id,
                path,
            })
            .map_err(|_| "Native render thread is no longer available".to_owned())
    }

    pub fn unload_onnx(&self, node_id: String) -> Result<(), String> {
        self.sender
            .send(Command::UnloadOnnx {
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
        let parsed = serde_json::from_str::<open_quartz_sdk::SdkError>(error).ok();
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

fn publish_onnx(updates: &Mutex<Option<RenderUpdate>>, generation: u64, onnx: &OnnxRuntime) {
    if let Ok(mut pending) = updates.lock() {
        let update = pending.get_or_insert_with(|| RenderUpdate::new(generation));
        if update.generation != generation {
            *update = RenderUpdate::new(generation);
        }
        update.onnx = Some(onnx.states.clone());
    }
}

fn render_worker(
    mut host: NativeHost,
    description: String,
    receiver: mpsc::Receiver<Command>,
    updates: Arc<Mutex<Option<RenderUpdate>>>,
    sender: mpsc::Sender<Command>,
) {
    let mut generation = 0;
    let mut running = false;
    let mut render_failed = false;
    let mut first_frame = true;
    let mut onnx = OnnxRuntime::default();
    let mut image_keys = BTreeMap::<String, u64>::new();
    let mut video_configs = BTreeMap::<String, NativeVideoConfig>::new();
    let mut current_graph: Option<String> = None;
    let mut stopped = false;
    let mut playback_requested = true;
    let mut preview_renderer: Option<String> = None;
    let mut preview_nodes = Vec::<String>::new();
    let mut preview_tick = Instant::now();
    loop {
        if onnx.reap_workers() {
            publish_onnx(&updates, generation, &onnx);
        }
        let command = if running || onnx.is_loading() {
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
            }
            Some(Command::SetGraph {
                generation: next_generation,
                graph,
                reset_resources,
                images,
            }) => {
                generation = next_generation;
                first_frame = true;
                running = false;
                render_failed = false;
                stopped = false;
                playback_requested = true;
                preview_nodes = graph.nodes().iter().filter(|node| {
                    matches!(node.node_type, open_quartz_sdk::NodeType::Shader | open_quartz_sdk::NodeType::Constant | open_quartz_sdk::NodeType::Renderer)
                        || node.data.outputs.iter().any(|port| port.data_type == open_quartz_sdk::DataType::Sampler2d)
                }).map(|node| node.id.clone()).collect();
                preview_tick = Instant::now() - Duration::from_millis(100);
                if reset_resources {
                    onnx.clear(&mut host);
                }
                let result: Result<bool, String> = (|| {
                    if host.has_graph() {
                        host.stop()?;
                    }
                    onnx.sync(&graph, &mut host, false)?;
                    let next_video_configs = graph_video_configs(&graph)?;
                    for node_id in video_configs
                        .iter()
                        .filter(|(id, config)| {
                            reset_resources || next_video_configs.get(*id) != Some(*config)
                        })
                        .map(|(id, _)| id.clone())
                        .collect::<Vec<_>>()
                    {
                        host.detach_video(&node_id)?;
                        video_configs.remove(&node_id);
                    }
                    let graph_json = serde_json::to_string(&graph)
                        .map_err(|error| format!("Cannot serialize the SDK graph: {error}"))?;
                    host.set_graph(&graph_json)?;
                    for (node_id, config) in next_video_configs {
                        if video_configs.get(&node_id) != Some(&config) {
                            host.attach_video(&node_id, config.clone())?;
                            video_configs.insert(node_id, config);
                        }
                    }
                    current_graph = Some(graph_json);
                    let stale_images = image_keys
                        .iter()
                        .filter(|(node_id, key)| {
                            reset_resources
                                || images.get(*node_id).is_none_or(|image| image.key != **key)
                        })
                        .map(|(node_id, _)| node_id.clone())
                        .collect::<Vec<_>>();
                    for node_id in stale_images {
                        host.remove_texture(&node_id);
                        image_keys.remove(&node_id);
                    }
                    for (node_id, image) in &images {
                        if image_keys.get(node_id) == Some(&image.key) {
                            continue;
                        }
                        host.upload_image(node_id, &image.rgba, image.width, image.height)?;
                        image_keys.insert(node_id.clone(), image.key);
                    }
                    // Frame events coalesce in the engine; clear graph/state history only on replacement.
                    let _ = host.drain_events();
                    if graph.nodes().is_empty() {
                        return Ok(false);
                    }
                    host.start_playback()?;
                    Ok(true)
                })();
                match result {
                    Ok(should_run) => {
                        running = should_run;
                        let status = if running {
                            format!("{description} · NativeHost graph ready; waiting for renderer output")
                        } else {
                            format!("{description} · NativeHost ready; empty graph has no renderer output")
                        };
                        publish_status(&updates, generation, status, true);
                    }
                    Err(error) => {
                        render_failed = true;
                        publish_failure(&updates, generation, "NativeHost graph initialization failed", &error);
                    }
                }
                publish_onnx(&updates, generation, &onnx);
            }
            Some(Command::LoadOnnx {
                generation: requested,
                node_id,
                path,
            }) if requested == generation => {
                host.unload_onnx(&node_id);
                if let Err(error) = onnx.load(node_id, path, sender.clone()) {
                    publish_status(
                        &updates,
                        generation,
                        format!("ONNX load refused: {error}"),
                        true,
                    );
                } else {
                    publish_status(
                        &updates,
                        generation,
                        "Loading ONNX model resource…".into(),
                        true,
                    );
                }
                publish_onnx(&updates, generation, &onnx);
            }
            Some(Command::UnloadOnnx {
                generation: requested,
                node_id,
            }) if requested == generation => {
                onnx.unload(&node_id, &mut host);
                publish_status(&updates, generation, "ONNX session unloaded.".into(), true);
                publish_onnx(&updates, generation, &onnx);
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
                    if let Some(node_id) = preview_renderer.as_deref() {
                        match host.preview_source(node_id) {
                            Ok(output) => {
                                if let Ok(mut pending) = updates.lock() {
                                    let update = pending
                                        .get_or_insert_with(|| RenderUpdate::new(generation));
                                    update.texture = Some((*output.texture).clone());
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
                    "stop" => host.stop(),
                    _ if running => Ok(()),
                    _ if stopped => (|| {
                        let graph_json = current_graph
                            .as_deref()
                            .ok_or("No graph is ready for playback")?;
                        host.set_graph(graph_json)?;
                        for (node_id, config) in &video_configs {
                            host.detach_video(node_id)?;
                            host.attach_video(node_id, config.clone())?;
                        }
                        host.start_playback()
                    })(),
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
                        }
                        first_frame = true;
                        publish_status(&updates, generation, format!("Playback: {action}"), false);
                    }
                    Err(error) => publish_status(&updates, generation, error, false),
                }
            }
            Some(Command::Playback { .. }) => {}
            Some(Command::Model(message)) => {
                let node_id = message.node_id().to_owned();
                if onnx.receive(message, &mut host) {
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
                                "ONNX model attached; waiting for inference output…".into(),
                                true,
                            );
                        }
                        Err(error) => {
                            running = false;
                            render_failed = true;
                            onnx.render_failed(&error);
                            publish_failure(&updates, generation, "NativeHost resume failed", &error);
                        }
                    }
                } else if let Some(state) = onnx.states.get(&node_id) {
                    if !state.loading && !state.attached {
                        publish_status(&updates, generation, state.status.clone(), true);
                    }
                }
                publish_onnx(&updates, generation, &onnx);
            }
            Some(Command::LoadOnnx { .. } | Command::UnloadOnnx { .. }) => {}
            None => {}
        }
        if !render_failed && preview_tick.elapsed() >= Duration::from_millis(100) {
            let previews = preview_nodes.iter().filter_map(|id| {
                host.preview_source(id).ok().map(|output| (id.clone(), (*output.texture).clone()))
            }).collect();
            if let Ok(mut pending) = updates.lock() {
                let update = pending.get_or_insert_with(|| RenderUpdate::new(generation));
                if update.generation != generation { *update = RenderUpdate::new(generation); }
                update.previews = Some(previews);
            }
            preview_tick = Instant::now();
        }
        if !running {
            continue;
        }
        let result: Result<Option<(wgpu::Texture, u32, u32)>, String> = (|| {
            let rendered = host.render_next()?;
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
            Ok(Some((
                (*output.texture).clone(),
                output.width,
                output.height,
            )))
        })();
        let events = host.take_output_events();
        if !events.is_empty() {
            for event in events {
                onnx.output(event);
            }
            publish_onnx(&updates, generation, &onnx);
        }
        match result {
            Ok(Some((texture, width, height))) => {
                if let Ok(mut pending) = updates.lock() {
                    let update = pending.get_or_insert_with(|| RenderUpdate::new(generation));
                    if update.generation != generation {
                        *update = RenderUpdate::new(generation);
                    }
                    update.texture = Some(texture);
                    if first_frame {
                        update.status = Some(format!("{description} · NativeHost rendering {width}×{height} · shared GPU texture"));
                        first_frame = false;
                    }
                }
            }
            Ok(None) => {}
            Err(error) => {
                let _ = host.pause();
                running = false;
                render_failed = true;
                onnx.render_failed(&error);
                publish_onnx(&updates, generation, &onnx);
                publish_failure(&updates, generation, "NativeHost rendering stopped", &error);
            }
        }
    }
    if host.has_graph() {
        let _ = host.stop();
    }
}

fn graph_video_configs(graph: &Graph) -> Result<BTreeMap<String, NativeVideoConfig>, String> {
    use open_quartz_host_api::{
        plan_host_resource_intents, HostGraphSnapshot, HostResourceIntent,
        HostResourceIntentRequest, HostResourceTarget, VideoSourceKind,
    };
    let snapshot: HostGraphSnapshot =
        serde_json::from_value(serde_json::to_value(graph).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    let plan = plan_host_resource_intents(HostResourceIntentRequest {
        host: HostResourceTarget::Native,
        previous_graph: None,
        graph: snapshot,
    })?;
    Ok(plan
        .intents
        .into_iter()
        .filter_map(|intent| match intent {
            HostResourceIntent::AttachVideo {
                node_id,
                kind,
                source,
                looping,
                playback_rate,
                ..
            } => Some((
                node_id,
                NativeVideoConfig {
                    kind: match kind {
                        VideoSourceKind::File => NativeVideoSourceKind::File,
                        VideoSourceKind::Camera => NativeVideoSourceKind::Camera,
                    },
                    source,
                    looping,
                    playback_rate,
                },
            )),
            _ => None,
        })
        .collect())
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
    fn failure_retains_node_identity_and_clears_failed_generation_previews() {
        let updates = Mutex::new(Some(RenderUpdate::new(1)));
        let error = open_quartz_sdk::SdkError::new(
            open_quartz_sdk::SdkErrorCode::InvalidResource, "Invalid WGSL type",
        ).for_node("shader_7");
        publish_failure(&updates, 2, "Rendering stopped", &error.to_json());
        let update = updates.lock().unwrap();
        let update = update.as_ref().unwrap();
        assert_eq!(update.generation, 2);
        assert_eq!(update.node_error.as_ref(), Some(&error));
        assert!(update.clear_image && update.texture.is_none());
        assert!(update.previews.as_ref().unwrap().is_empty());
    }
}
