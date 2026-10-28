use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use open_quartz_native_host::NativeHost;
use open_quartz_sdk::Graph;
use slint::wgpu_30::WGPUConfiguration;

use crate::onnx_runtime::{LiveOnnxState, ModelMessage, OnnxRuntime};
use std::collections::BTreeMap;
pub struct RenderUpdate {
    generation: u64,
    pub status: Option<String>,
    pub texture: Option<wgpu::Texture>,
    pub clear_image: bool,
    pub onnx: Option<BTreeMap<String, LiveOnnxState>>,
}

impl RenderUpdate {
    fn new(generation: u64) -> Self {
        Self {
            generation,
            status: None,
            texture: None,
            clear_image: false,
            onnx: None,
        }
    }
}

pub(crate) enum Command {
    SetGraph {
        generation: u64,
        graph: Graph,
        reset_resources: bool,
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

    pub fn set_graph(&mut self, graph: Graph) -> Result<(), String> {
        self.generation = self.generation.wrapping_add(1);
        self.accepting_frames = true;
        self.sender
            .send(Command::SetGraph {
                generation: self.generation,
                graph,
                reset_resources: std::mem::take(&mut self.reset_resources),
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
    let mut first_frame = true;
    let mut onnx = OnnxRuntime::default();
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
                running = false;
            }
            Some(Command::SetGraph {
                generation: next_generation,
                graph,
                reset_resources,
            }) => {
                generation = next_generation;
                first_frame = true;
                running = false;
                if reset_resources {
                    onnx.clear(&mut host);
                }
                let result: Result<bool, String> = (|| {
                    if host.has_graph() {
                        host.stop()?;
                    }
                    onnx.sync(&graph, &mut host, false)?;
                    let graph_json = serde_json::to_string(&graph)
                        .map_err(|error| format!("Cannot serialize the SDK graph: {error}"))?;
                    host.set_graph(&graph_json)?;
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
                    Err(error) => publish_status(
                        &updates,
                        generation,
                        format!("NativeHost graph initialization failed: {error}"),
                        true,
                    ),
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
            Some(Command::Model(message)) => {
                let node_id = message.node_id().to_owned();
                if onnx.receive(message, &mut host) {
                    let result = if running { Ok(()) } else { host.resume() };
                    match result {
                        Ok(()) => {
                            running = true;
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
                            onnx.render_failed(&error);
                            publish_status(&updates, generation, error, true);
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
        if !running {
            continue;
        }
        let result: Result<Option<(wgpu::Texture, u32, u32)>, String> = (|| {
            let rendered = host.render_next()?;
            if rendered.width == 0 || rendered.height == 0 {
                return Ok(None);
            }
            let output = host.preview_source(&rendered.output_node_id)?;
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
                onnx.render_failed(&error);
                publish_onnx(&updates, generation, &onnx);
                publish_status(
                    &updates,
                    generation,
                    format!("NativeHost rendering stopped: {error}"),
                    true,
                );
            }
        }
    }
    if host.has_graph() {
        let _ = host.stop();
    }
}
