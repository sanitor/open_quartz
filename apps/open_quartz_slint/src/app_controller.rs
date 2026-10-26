use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use open_quartz_sdk::{
    Environment, GraphCommand, NodeFactoryRequest, NodeId, OpenQuartz, PortId, PortKey, Position,
    Project,
};
use slint::ComponentHandle;

use crate::graph_view_model::GraphViewModel;
use crate::render_bridge::RenderBridge;
use crate::services::file_service;
use crate::AppWindow;

pub struct AppController {
    window: slint::Weak<AppWindow>,
    project: Arc<Project>,
    renderer: Option<RenderBridge>,
    files: ProjectIo,
    busy: bool,
    timer: slint::Timer,
}

impl AppController {
    pub fn new(
        window: &AppWindow,
        renderer: Option<RenderBridge>,
        runtime_status: String,
        project_path: Option<PathBuf>,
    ) -> Result<Rc<RefCell<Self>>, String> {
        let sdk = OpenQuartz::new(Environment::headless());
        let project = if project_path.is_some() {
            sdk.create_project("Untitled")
        } else {
            sample_project(&sdk)?
        };
        let controller = Rc::new(RefCell::new(Self {
            window: window.as_weak(),
            project: Arc::new(project),
            renderer,
            files: ProjectIo::new()?,
            busy: false,
            timer: slint::Timer::default(),
        }));
        window.set_runtime_status(runtime_status.into());
        window.set_io_status(
            "Enter an explicit project path to Open or Save. Save overwrites that path.".into(),
        );
        controller.borrow_mut().publish_project();

        let weak = Rc::downgrade(&controller);
        window.on_open_project(move |path| {
            if let Some(controller) = weak.upgrade() {
                controller
                    .borrow_mut()
                    .open_path(PathBuf::from(path.as_str()));
            }
        });
        let weak = Rc::downgrade(&controller);
        window.on_save_project(move |path| {
            if let Some(controller) = weak.upgrade() {
                controller
                    .borrow_mut()
                    .save_path(PathBuf::from(path.as_str()));
            }
        });
        let weak = Rc::downgrade(&controller);
        controller.borrow().timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(16),
            move || {
                if let Some(controller) = weak.upgrade() {
                    controller.borrow_mut().poll();
                }
            },
        );
        let weak = Rc::downgrade(&controller);
        window.window().on_close_requested(move || {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().shutdown();
            }
            if let Err(error) = slint::quit_event_loop() {
                eprintln!("Cannot quit Slint event loop: {error}");
            }
            slint::CloseRequestResponse::HideWindow
        });
        if let Some(path) = project_path {
            window.set_project_path(path.to_string_lossy().as_ref().into());
            controller.borrow_mut().open_path(path);
        }
        Ok(controller)
    }

    pub fn shutdown(&mut self) {
        self.timer.stop();
        if let Some(window) = self.window.upgrade() {
            window.set_renderer_image(slint::Image::default());
            window.set_has_renderer_image(false);
        }
        self.renderer.take();
    }

    fn publish_project(&mut self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let projection = GraphViewModel::from_project(&self.project);
        window.set_project_name(self.project.name().into());
        window.set_graph_nodes(projection.nodes);
        window.set_graph_summary(projection.summary.into());
        window.set_renderer_image(slint::Image::default());
        window.set_has_renderer_image(false);
        if let Some(renderer) = &mut self.renderer {
            match renderer.set_graph(self.project.graph_snapshot()) {
                Ok(()) => window.set_runtime_status("NativeHost is applying the SDK graph…".into()),
                Err(error) => window.set_runtime_status(error.into()),
            }
        }
    }

    fn open_path(&mut self, path: PathBuf) {
        self.request_file(path, false);
    }

    fn save_path(&mut self, path: PathBuf) {
        self.request_file(path, true);
    }

    fn request_file(&mut self, path: PathBuf, save: bool) {
        if self.busy {
            return;
        }
        let Some(window) = self.window.upgrade() else {
            return;
        };
        if path.as_os_str().is_empty() {
            window.set_io_status("A project path is required. Enter a path above.".into());
            return;
        }
        let status = format!(
            "{} {}…",
            if save { "Saving" } else { "Opening" },
            path.display()
        );
        let request = if save {
            FileRequest::Save {
                path,
                project: self.project.clone(),
            }
        } else {
            FileRequest::Open(path)
        };
        if self.files.sender.send(request).is_err() {
            window.set_io_status("Project file worker is no longer available.".into());
            return;
        }
        self.busy = true;
        window.set_busy(true);
        window.set_io_status(status.into());
    }

    fn poll(&mut self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        while let Ok(result) = self.files.receiver.try_recv() {
            self.busy = false;
            window.set_busy(false);
            match result {
                FileResult::Opened { path, project } => {
                    self.project = Arc::new(project);
                    window.set_project_path(path.to_string_lossy().as_ref().into());
                    window.set_io_status(
                        format!("Opened {} through the Rust SDK.", path.display()).into(),
                    );
                    self.publish_project();
                }
                FileResult::Saved(path) => {
                    window.set_project_path(path.to_string_lossy().as_ref().into());
                    window.set_io_status(
                        format!("Saved {} through the Rust SDK.", path.display()).into(),
                    );
                }
                FileResult::Failed(error) => window.set_io_status(error.into()),
            }
        }
        let update = self.renderer.as_ref().and_then(RenderBridge::take_update);
        if let Some(update) = update {
            if let Some(status) = update.status {
                window.set_runtime_status(status.into());
            }
            if update.clear_image {
                window.set_renderer_image(slint::Image::default());
                window.set_has_renderer_image(false);
            }
            if let Some(texture) = update.texture {
                match slint::Image::try_from(texture) {
                    Ok(image) => {
                        window.set_renderer_image(image);
                        window.set_has_renderer_image(true);
                    }
                    Err(error) => {
                        window.set_renderer_image(slint::Image::default());
                        window.set_has_renderer_image(false);
                        window.set_runtime_status(format!("Native renderer stopped: shared GPU texture import failed: {error:?}").into());
                        if let Some(renderer) = &mut self.renderer {
                            renderer.pause_after_import_error();
                        }
                    }
                }
            }
        }
    }
}

fn sample_project(sdk: &OpenQuartz) -> Result<Project, String> {
    let mut project = sdk.create_project("Untitled · Gradient");
    let (shader, _) = project.create_graph_node(NodeFactoryRequest::Shader {
        position: Some(Position { x: 0.0, y: 0.0 }),
        code: "@fragment\nfn main(@location(0) uv: vec2f) -> @location(0) vec4f {\n    return vec4f(uv.x, uv.y, 0.35, 1.0);\n}\n".to_owned(),
        label: "Gradient".to_owned(),
        template_name: None,
        shader_template_id: None,
    }, project.graph_revision()).map_err(|error| error.to_json())?;
    let mut data = shader.data.clone();
    data.width = Some(640);
    data.height = Some(360);
    data.auto_size = Some(false);
    project
        .apply_graph_command(
            GraphCommand::UpdateNodeData {
                node_id: NodeId::new(&shader.id),
                data,
            },
            project.graph_revision(),
        )
        .map_err(|error| error.to_json())?;
    let (renderer, _) = project
        .create_graph_node(
            NodeFactoryRequest::Renderer {
                position: Some(Position { x: 320.0, y: 0.0 }),
            },
            project.graph_revision(),
        )
        .map_err(|error| error.to_json())?;
    let output = shader
        .data
        .outputs
        .first()
        .ok_or("SDK sample shader has no output port")?;
    let input = renderer
        .data
        .inputs
        .first()
        .ok_or("SDK sample renderer has no input port")?;
    project
        .apply_graph_command(
            GraphCommand::Connect {
                source: PortKey::new(NodeId::new(&shader.id), PortId::new(&output.id)),
                target: PortKey::new(NodeId::new(&renderer.id), PortId::new(&input.id)),
            },
            project.graph_revision(),
        )
        .map_err(|error| error.to_json())?;
    Ok(project)
}

enum FileRequest {
    Open(PathBuf),
    Save {
        path: PathBuf,
        project: Arc<Project>,
    },
    Shutdown,
}

enum FileResult {
    Opened { path: PathBuf, project: Project },
    Saved(PathBuf),
    Failed(String),
}

struct ProjectIo {
    sender: mpsc::Sender<FileRequest>,
    receiver: mpsc::Receiver<FileResult>,
    worker: Option<JoinHandle<()>>,
}

impl ProjectIo {
    fn new() -> Result<Self, String> {
        let (sender, requests) = mpsc::channel();
        let (results, receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("open-quartz-project-io".to_owned())
            .spawn(move || {
                let sdk = OpenQuartz::new(Environment::headless());
                while let Ok(request) = requests.recv() {
                    let result = match request {
                        FileRequest::Open(path) => {
                            let project = file_service::read_project(&path).and_then(|contents| {
                                sdk.open_project_json(&contents).map_err(|error| {
                                    format!("Cannot open {}: {}", path.display(), error.to_json())
                                })
                            });
                            match project {
                                Ok(project) => FileResult::Opened { path, project },
                                Err(error) => FileResult::Failed(error),
                            }
                        }
                        FileRequest::Save { path, project } => {
                            let contents = serde_json::to_string_pretty(&project.to_file())
                                .map_err(|error| {
                                    format!("Cannot serialize {}: {error}", path.display())
                                });
                            match contents.and_then(|mut contents| {
                                contents.push('\n');
                                file_service::write_project(&path, &contents)
                            }) {
                                Ok(()) => FileResult::Saved(path),
                                Err(error) => FileResult::Failed(error),
                            }
                        }
                        FileRequest::Shutdown => break,
                    };
                    if results.send(result).is_err() {
                        break;
                    }
                }
            })
            .map_err(|error| format!("Cannot start project file worker: {error}"))?;
        Ok(Self {
            sender,
            receiver,
            worker: Some(worker),
        })
    }
}

impl Drop for ProjectIo {
    fn drop(&mut self) {
        let _ = self.sender.send(FileRequest::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
