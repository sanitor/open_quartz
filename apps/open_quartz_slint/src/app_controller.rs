use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use open_quartz_sdk::{
    Environment, GraphCommand, NodeFactoryRequest, NodeId, OpenQuartz, PortId, PortKey, Position,
    Project, SdkError,
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
    selected_node_id: Option<String>,
    graph_stress_count: Option<usize>,
    files: ProjectIo,
    busy: bool,
    timer: slint::Timer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProjectionStats {
    node_count: usize,
    edge_count: usize,
    duration_micros: u128,
}

impl AppController {
    pub fn new(
        window: &AppWindow,
        renderer: Option<RenderBridge>,
        runtime_status: String,
        project_path: Option<PathBuf>,
        graph_stress_count: Option<usize>,
    ) -> Result<Rc<RefCell<Self>>, String> {
        let sdk = OpenQuartz::new(Environment::headless());
        let project = if project_path.is_some() {
            sdk.create_project("Untitled")
        } else if let Some(count) = graph_stress_count {
            graph_stress_project(&sdk, count)?
        } else {
            sample_project(&sdk)?
        };
        let controller = Rc::new(RefCell::new(Self {
            window: window.as_weak(),
            project: Arc::new(project),
            renderer,
            selected_node_id: None,
            graph_stress_count,
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
        window.on_graph_node_selected(move |node_id| {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().select_node(node_id.as_str());
            }
        });
        let weak = Rc::downgrade(&controller);
        window.on_graph_node_moved(move |node_id, x, y| {
            if let Some(controller) = weak.upgrade() {
                controller
                    .borrow_mut()
                    .move_node(node_id.as_str(), f64::from(x), f64::from(y));
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
        let Some(projection) = self.publish_graph_projection() else {
            return;
        };
        let Some(window) = self.window.upgrade() else {
            return;
        };
        window.set_renderer_image(slint::Image::default());
        window.set_has_renderer_image(false);
        if let Some(requested_count) = self.graph_stress_count {
            window.set_runtime_status(
                format!("Graph stress mode · projection only · requested {requested_count} nodes")
                    .into(),
            );
            window.set_io_status(
                format!(
                    "Stress projection: {} nodes · {} edges · {} µs",
                    projection.node_count, projection.edge_count, projection.duration_micros
                )
                .into(),
            );
            return;
        }
        if let Some(renderer) = &mut self.renderer {
            match renderer.set_graph(self.project.graph_snapshot()) {
                Ok(()) => window.set_runtime_status("NativeHost is applying the SDK graph…".into()),
                Err(error) => window.set_runtime_status(error.into()),
            }
        }
    }

    fn publish_graph_projection(&mut self) -> Option<ProjectionStats> {
        let window = self.window.upgrade()?;
        let projection =
            GraphViewModel::from_project(&self.project, self.selected_node_id.as_deref());
        let stats = ProjectionStats {
            node_count: projection.node_count,
            edge_count: projection.edge_count,
            duration_micros: projection.projection_duration.as_micros(),
        };
        window.set_project_name(self.project.name().into());
        window.set_graph_nodes(projection.nodes);
        window.set_graph_edges(projection.edges);
        window.set_graph_summary(projection.summary.into());
        window.set_graph_content_width(projection.content_width);
        window.set_graph_content_height(projection.content_height);
        window.set_graph_origin_x(projection.origin_x);
        window.set_graph_origin_y(projection.origin_y);
        Some(stats)
    }

    fn select_node(&mut self, node_id: &str) {
        if self.busy {
            if let Some(window) = self.window.upgrade() {
                window
                    .set_io_status("Cannot select a node while project I/O is in progress.".into());
            }
            return;
        }
        if self.selected_node_id.as_deref() == Some(node_id) {
            return;
        }
        if self
            .project
            .graph()
            .nodes()
            .iter()
            .any(|node| node.id == node_id)
        {
            self.selected_node_id = Some(node_id.to_owned());
            self.publish_graph_projection();
        }
    }

    fn move_node(&mut self, node_id: &str, x: f64, y: f64) {
        if self.busy {
            if let Some(window) = self.window.upgrade() {
                window.set_io_status(
                    "Cannot move a node while project I/O is in progress; project was not changed."
                        .into(),
                );
            }
            return;
        }
        let result = commit_node_position(Arc::make_mut(&mut self.project), node_id, x, y);
        let Some(window) = self.window.upgrade() else {
            return;
        };
        match result {
            Ok(changed) => {
                self.selected_node_id = Some(node_id.to_owned());
                let projection = self.publish_graph_projection();
                let timing = projection
                    .map(|stats| format!(" · projection {} µs", stats.duration_micros))
                    .unwrap_or_default();
                let status = if changed {
                    format!("Moved {node_id} to ({x:.1}, {y:.1}){timing}")
                } else {
                    format!("Selected {node_id}; position was unchanged{timing}")
                };
                window.set_io_status(status.into());
            }
            Err(error) => {
                window.set_io_status(
                    format!(
                        "Cannot move node {node_id}; project was not changed: {}",
                        error.to_json()
                    )
                    .into(),
                );
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
                    self.selected_node_id = None;
                    self.graph_stress_count = None;
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

fn commit_node_position(
    project: &mut Project,
    node_id: &str,
    x: f64,
    y: f64,
) -> Result<bool, SdkError> {
    if project
        .graph()
        .nodes()
        .iter()
        .any(|node| node.id == node_id && node.position == (Position { x, y }))
    {
        return Ok(false);
    }
    let revision = project.graph_revision();
    project
        .apply_graph_command(
            GraphCommand::SetNodePosition {
                node_id: NodeId::new(node_id),
                position: Position { x, y },
            },
            revision,
        )
        .map(|_| true)
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

fn graph_stress_project(sdk: &OpenQuartz, count: usize) -> Result<Project, String> {
    let mut project = sdk.create_project(format!("Graph stress · {count} nodes"));
    let mut ports = Vec::with_capacity(count);
    for index in 0..count {
        let column = index % 10;
        let row = index / 10;
        let (node, _) = project
            .create_graph_node(
                NodeFactoryRequest::Math {
                    position: Some(Position {
                        x: column as f64 * 220.0,
                        y: row as f64 * 128.0,
                    }),
                    op: "add".to_owned(),
                },
                project.graph_revision(),
            )
            .map_err(|error| error.to_json())?;
        let output = node
            .data
            .outputs
            .first()
            .ok_or_else(|| format!("Stress node {} has no output port", node.id))?;
        if node.data.inputs.len() < 2 {
            return Err(format!("Stress node {} has fewer than two inputs", node.id));
        }
        ports.push((
            NodeId::new(&node.id),
            PortId::new(&output.id),
            node.data
                .inputs
                .iter()
                .map(|port| PortId::new(&port.id))
                .collect::<Vec<_>>(),
        ));
    }

    let mut connect = |source: usize, target: usize, target_input: usize| {
        project
            .apply_graph_command(
                GraphCommand::Connect {
                    source: PortKey::new(ports[source].0.clone(), ports[source].1.clone()),
                    target: PortKey::new(
                        ports[target].0.clone(),
                        ports[target].2[target_input].clone(),
                    ),
                },
                project.graph_revision(),
            )
            .map(|_| ())
            .map_err(|error| error.to_json())
    };
    for target in 1..count {
        connect(target - 1, target, 0)?;
    }
    let secondary_edges = (count / 2 + 1).min(count.saturating_sub(2));
    for offset in 0..secondary_edges {
        let target = offset + 2;
        connect(target - 2, target, 1)?;
    }
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

#[cfg(test)]
mod tests {
    use open_quartz_sdk::{Environment, OpenQuartz, Position, SdkErrorCode};
    use slint::Model;

    use super::{commit_node_position, graph_stress_project, sample_project, GraphViewModel};

    #[test]
    fn projection_contains_spatial_nodes_and_edge_endpoints() {
        let sdk = OpenQuartz::new(Environment::headless());
        let project = sample_project(&sdk).unwrap();
        let projection = GraphViewModel::from_project(&project, None);

        assert_eq!(projection.node_count, 2);
        assert_eq!(projection.edge_count, 1);
        assert_eq!(projection.nodes.row_count(), 2);
        assert_eq!(projection.edges.row_count(), 1);
        let edge = projection.edges.row_data(0).unwrap();
        assert_eq!(edge.source_index, 0);
        assert_eq!(edge.target_index, 1);
        assert_eq!((edge.source_x, edge.source_y), (184.0, 46.0));
        assert_eq!((edge.target_x, edge.target_y), (320.0, 46.0));
        assert!((0..projection.nodes.row_count()).all(|row| !projection
            .nodes
            .row_data(row)
            .unwrap()
            .selected));
    }

    #[test]
    fn projection_represents_exactly_one_selected_node() {
        let sdk = OpenQuartz::new(Environment::headless());
        let project = sample_project(&sdk).unwrap();
        let selected_id = project.graph().nodes()[1].id.clone();
        let projection = GraphViewModel::from_project(&project, Some(selected_id.as_str()));

        let selected = (0..projection.nodes.row_count())
            .filter_map(|row| projection.nodes.row_data(row))
            .filter(|node| node.selected)
            .collect::<Vec<_>>();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].node_id.as_str(), selected_id.as_str());
    }

    #[test]
    fn completed_move_is_one_command_and_survives_serialization_reload() {
        let sdk = OpenQuartz::new(Environment::headless());
        let mut project = sample_project(&sdk).unwrap();
        let node_id = project.graph().nodes()[0].id.clone();
        let revision = project.graph_revision();

        commit_node_position(&mut project, &node_id, 144.5, 233.25).unwrap();

        assert_eq!(project.graph_revision(), revision + 1);
        let serialized = serde_json::to_string_pretty(&project.to_file()).unwrap();
        let reloaded = sdk.open_project_json(&serialized).unwrap();
        let node = reloaded
            .graph()
            .nodes()
            .iter()
            .find(|node| node.id == node_id)
            .unwrap();
        assert_eq!(
            node.position,
            Position {
                x: 144.5,
                y: 233.25
            }
        );
        let projection = GraphViewModel::from_project(&reloaded, Some(node_id.as_str()));
        let edge = projection.edges.row_data(0).unwrap();
        assert_eq!((edge.source_x, edge.source_y), (328.5, 279.25));
        assert_eq!((edge.target_x, edge.target_y), (320.0, 46.0));
    }

    #[test]
    fn unchanged_position_commit_does_not_increment_revision() {
        let sdk = OpenQuartz::new(Environment::headless());
        let mut project = sample_project(&sdk).unwrap();
        let node = project.graph().nodes()[0].clone();
        let revision = project.graph_revision();

        let changed = commit_node_position(
            &mut project,
            node.id.as_str(),
            node.position.x,
            node.position.y,
        )
        .unwrap();

        assert!(!changed);
        assert_eq!(project.graph_revision(), revision);
    }

    #[test]
    fn projection_offsets_negative_nodes_inside_scrollable_content() {
        let sdk = OpenQuartz::new(Environment::headless());
        let mut project = sample_project(&sdk).unwrap();
        let node_id = project.graph().nodes()[0].id.clone();
        commit_node_position(&mut project, node_id.as_str(), -100.0, -50.0).unwrap();

        let projection = GraphViewModel::from_project(&project, None);
        let node = projection
            .nodes
            .row_data(0)
            .expect("sample project has a first node");

        assert_eq!(projection.origin_x, 124.0);
        assert_eq!(projection.origin_y, 74.0);
        assert_eq!(node.x + projection.origin_x, 24.0);
        assert_eq!(node.y + projection.origin_y, 24.0);
        assert!(node.x + projection.origin_x + node.width < projection.content_width);
        assert!(node.y + projection.origin_y + node.height < projection.content_height);
    }

    #[test]
    fn unknown_node_move_preserves_project_and_revision() {
        let sdk = OpenQuartz::new(Environment::headless());
        let mut project = sample_project(&sdk).unwrap();
        let before = project.to_file();
        let revision = project.graph_revision();

        let error = commit_node_position(&mut project, "missing-node", 10.0, 20.0).unwrap_err();

        assert_eq!(error.code, SdkErrorCode::UnknownNode);
        assert_eq!(project.graph_revision(), revision);
        assert_eq!(project.to_file(), before);
    }

    #[test]
    fn hundred_node_stress_fixture_is_deterministic_and_structurally_bounded() {
        let sdk = OpenQuartz::new(Environment::headless());
        let first = graph_stress_project(&sdk, 100).unwrap();
        let second = graph_stress_project(&sdk, 100).unwrap();

        assert_eq!(first.graph().nodes().len(), 100);
        assert_eq!(first.graph().edges().len(), 150);
        assert_eq!(first.graph_snapshot(), second.graph_snapshot());
        let serialized = serde_json::to_string(&first.to_file()).unwrap();
        let reloaded = sdk.open_project_json(&serialized).unwrap();
        assert_eq!(reloaded.graph_snapshot(), first.graph_snapshot());

        let projection = GraphViewModel::from_project(&first, None);
        eprintln!(
            "100-node graph projection took {} µs",
            projection.projection_duration.as_micros()
        );
        assert_eq!(projection.node_count, 100);
        assert_eq!(projection.edge_count, 150);
        assert_eq!(projection.nodes.row_count(), 100);
        assert_eq!(projection.edges.row_count(), 150);
        let last_node = projection.nodes.row_data(99).unwrap();
        assert_eq!((last_node.x, last_node.y), (1980.0, 1152.0));
        assert!(last_node.x + projection.origin_x + last_node.width < projection.content_width);
        assert!(last_node.y + projection.origin_y + last_node.height < projection.content_height);
        for row in 0..projection.edges.row_count() {
            let edge = projection.edges.row_data(row).unwrap();
            assert!((edge.source_index as usize) < projection.node_count);
            assert!((edge.target_index as usize) < projection.node_count);
        }
    }
}
