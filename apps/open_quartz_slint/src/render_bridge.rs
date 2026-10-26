use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use open_quartz_native_host::NativeHost;
use open_quartz_sdk::Graph;
use slint::wgpu_30::WGPUConfiguration;

pub struct RenderUpdate {
    generation: u64,
    pub status: Option<String>,
    pub texture: Option<wgpu::Texture>,
    pub clear_image: bool,
}

impl RenderUpdate {
    fn new(generation: u64) -> Self {
        Self {
            generation,
            status: None,
            texture: None,
            clear_image: false,
        }
    }
}

enum Command {
    SetGraph { generation: u64, graph: Graph },
    Pause,
    Shutdown,
}

pub struct RenderBridge {
    sender: mpsc::Sender<Command>,
    updates: Arc<Mutex<Option<RenderUpdate>>>,
    generation: u64,
    accepting_frames: bool,
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
        let worker = thread::Builder::new()
            .name("open-quartz-render".to_owned())
            .spawn(move || render_worker(host, description, receiver, worker_updates))
            .map_err(|error| format!("Cannot start native render thread: {error}"))?;
        Ok((
            Self {
                sender,
                updates,
                generation: 0,
                accepting_frames: true,
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

fn render_worker(
    mut host: NativeHost,
    description: String,
    receiver: mpsc::Receiver<Command>,
    updates: Arc<Mutex<Option<RenderUpdate>>>,
) {
    let mut generation = 0;
    let mut running = false;
    let mut first_frame = true;
    loop {
        let command = if running {
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
            }) => {
                generation = next_generation;
                first_frame = true;
                running = false;
                let result: Result<bool, String> = (|| {
                    if host.has_graph() {
                        host.stop()?;
                    }
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
            }
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
