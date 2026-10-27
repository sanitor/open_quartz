use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use open_quartz_sdk::{
    Environment, GraphCommand, NodeFactoryRequest, NodeId, NodeType, OpenQuartz, PortId, PortKey,
    Position, Project, SdkError,
};
use slint::ComponentHandle;

use crate::graph_clipboard::GraphClipboard;
use crate::graph_view_model::GraphViewModel;
use crate::render_bridge::RenderBridge;
use crate::services::file_service;
use crate::AppWindow;

#[derive(Clone, Debug)]
struct InspectorDraft {
    node_id: String,
    label: String,
    shader_code: String,
    auto_size: bool,
    output_width: i32,
    output_height: i32,
    validation_message: String,
}

impl InspectorDraft {
    fn from_node(node: &open_quartz_sdk::ProjectNode) -> Self {
        Self {
            node_id: node.id.clone(),
            label: node.data.label.clone(),
            shader_code: node.data.shader_code.clone(),
            auto_size: node.data.auto_size != Some(false),
            output_width: node.data.width.unwrap_or(512).clamp(1, 8192) as i32,
            output_height: node.data.height.unwrap_or(512).clamp(1, 8192) as i32,
            validation_message: node
                .data
                .extra
                .get("parseError")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        }
    }
}

#[derive(Clone, Copy)]
enum InspectorChange {
    Label,
    Shader,
    OutputSize,
}

impl InspectorChange {
    fn status(self) -> &'static str {
        match self {
            Self::Label => "Node name applied through the SDK.",
            Self::Shader => "Shader source applied through the SDK.",
            Self::OutputSize => "Output size applied through the SDK.",
        }
    }
}

pub struct AppController {
    window: slint::Weak<AppWindow>,
    project: Arc<Project>,
    renderer: Option<RenderBridge>,
    selected_node_ids: BTreeSet<String>,
    inspector_draft: Option<InspectorDraft>,
    clipboard: Option<GraphClipboard>,
    graph_stress_count: Option<usize>,
    selected_edge_id: Option<String>,
    pending_connection: Option<PendingConnection>,
    undo_edits: Vec<EditKind>,
    redo_edits: Vec<EditKind>,
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

struct PendingConnection {
    source: PortKey,
    edge_id: Option<String>,
}

#[derive(Clone, Copy)]
enum EditKind {
    Layout,
    Topology,
    Content,
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
            selected_node_ids: BTreeSet::new(),
            inspector_draft: None,
            clipboard: None,
            graph_stress_count,
            selected_edge_id: None,
            pending_connection: None,
            undo_edits: Vec::new(),
            redo_edits: Vec::new(),
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
        window.on_inspector_label_draft_changed(move |text| {
            if let Some(controller) = weak.upgrade() {
                let mut controller = controller.borrow_mut();
                if !controller.busy {
                    if let Some(draft) = controller.inspector_draft.as_mut() {
                        draft.label = text.to_string();
                    }
                    controller.publish_inspector_dirty();
                }
            }
        });
        let weak = Rc::downgrade(&controller);
        window.on_inspector_shader_draft_changed(move |text| {
            if let Some(controller) = weak.upgrade() {
                let mut controller = controller.borrow_mut();
                if !controller.busy {
                    if let Some(draft) = controller.inspector_draft.as_mut() {
                        draft.shader_code = text.to_string();
                    }
                    controller.publish_inspector_dirty();
                }
            }
        });
        let weak = Rc::downgrade(&controller);
        window.on_inspector_output_size_draft_changed(move |width, height, auto_size| {
            if let Some(controller) = weak.upgrade() {
                let mut controller = controller.borrow_mut();
                if !controller.busy {
                    if let Some(draft) = controller.inspector_draft.as_mut() {
                        draft.output_width = width;
                        draft.output_height = height;
                        draft.auto_size = auto_size;
                    }
                    controller.publish_inspector_dirty();
                }
            }
        });
        let weak = Rc::downgrade(&controller);
        window.on_inspector_apply_label(move || {
            if let Some(controller) = weak.upgrade() {
                controller
                    .borrow_mut()
                    .apply_inspector(InspectorChange::Label);
            }
        });
        let weak = Rc::downgrade(&controller);
        window.on_inspector_apply_shader(move || {
            if let Some(controller) = weak.upgrade() {
                controller
                    .borrow_mut()
                    .apply_inspector(InspectorChange::Shader);
            }
        });
        let weak = Rc::downgrade(&controller);
        window.on_inspector_apply_output_size(move || {
            if let Some(controller) = weak.upgrade() {
                controller
                    .borrow_mut()
                    .apply_inspector(InspectorChange::OutputSize);
            }
        });
        let weak = Rc::downgrade(&controller);
        window.on_inspector_discard_drafts(move || {
            if let Some(controller) = weak.upgrade() {
                let mut controller = controller.borrow_mut();
                if !controller.busy {
                    controller.inspector_draft = None;
                    controller.publish_inspector();
                    controller.set_edit_status("Inspector drafts discarded; project unchanged.");
                }
            }
        });
        let weak = Rc::downgrade(&controller);
        window.on_graph_node_selected(move |node_id, additive| {
            if let Some(controller) = weak.upgrade() {
                controller
                    .borrow_mut()
                    .select_node(node_id.as_str(), additive);
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
        window.on_graph_port_selected(move |node_id, port_id, output| {
            if let Some(controller) = weak.upgrade() {
                controller
                    .borrow_mut()
                    .select_port(node_id.as_str(), port_id.as_str(), output);
            }
        });
        let weak = Rc::downgrade(&controller);
        window.on_graph_edge_selected(move |edge_id| {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().select_edge(edge_id.as_str());
            }
        });
        let weak = Rc::downgrade(&controller);
        window.on_graph_reconnect_edge(move || {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().reconnect_edge();
            }
        });
        let weak = Rc::downgrade(&controller);
        window.on_graph_delete_edge(move || {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().delete_edge();
            }
        });
        let weak = Rc::downgrade(&controller);
        window.on_graph_cancel_connection(move || {
            if let Some(controller) = weak.upgrade() {
                let mut controller = controller.borrow_mut();
                if !controller.edits_blocked() {
                    controller.pending_connection = None;
                    controller.publish_edit_state();
                    controller.set_edit_status("Connection cancelled; project unchanged.");
                }
            }
        });
        let weak = Rc::downgrade(&controller);
        window.on_graph_undo(move || {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().restore_edit(false);
            }
        });
        let weak = Rc::downgrade(&controller);
        window.on_graph_redo(move || {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().restore_edit(true);
            }
        });
        let weak = Rc::downgrade(&controller);
        window.on_graph_copy(move || {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().copy_selection();
            }
        });
        let weak = Rc::downgrade(&controller);
        window.on_graph_paste(move || {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().paste_clipboard();
            }
        });
        let weak = Rc::downgrade(&controller);
        window.on_graph_select_all(move || {
            if let Some(controller) = weak.upgrade() {
                let mut controller = controller.borrow_mut();
                if !controller.edits_blocked() {
                    controller.selected_node_ids = controller
                        .project
                        .graph()
                        .nodes()
                        .iter()
                        .map(|node| node.id.clone())
                        .collect();
                    controller.selected_edge_id = None;
                    controller.publish_graph_projection();
                }
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
        let projection = GraphViewModel::from_project(&self.project, &self.selected_node_ids);
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
        self.publish_edit_state();
        self.publish_inspector();
        Some(stats)
    }

    fn select_node(&mut self, node_id: &str, additive: bool) {
        if self.edits_blocked() {
            return;
        }
        if self
            .project
            .graph()
            .nodes()
            .iter()
            .any(|node| node.id == node_id)
        {
            if additive {
                if !self.selected_node_ids.remove(node_id) {
                    self.selected_node_ids.insert(node_id.to_owned());
                }
            } else {
                self.selected_node_ids.clear();
                self.selected_node_ids.insert(node_id.to_owned());
            }
            self.selected_edge_id = None;
            self.publish_graph_projection();
        }
    }

    fn move_node(&mut self, node_id: &str, x: f64, y: f64) {
        if self.edits_blocked() {
            return;
        }
        let result = commit_node_position(Arc::make_mut(&mut self.project), node_id, x, y);
        let Some(window) = self.window.upgrade() else {
            return;
        };
        match result {
            Ok(changed) => {
                self.selected_node_ids.clear();
                self.selected_node_ids.insert(node_id.to_owned());
                self.selected_edge_id = None;
                if changed {
                    self.undo_edits.push(EditKind::Layout);
                    self.redo_edits.clear();
                }
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

    fn edits_blocked(&self) -> bool {
        if self.busy {
            self.set_edit_status(
                "Cannot edit while project I/O is in progress; project unchanged.",
            );
            return true;
        }
        if self.inspector_is_dirty() {
            self.set_edit_status(
                "Apply or discard inspector drafts before changing the graph or project.",
            );
            return true;
        }
        false
    }

    fn inspector_dirty_fields(&self) -> (bool, bool, bool) {
        let Some(draft) = &self.inspector_draft else {
            return (false, false, false);
        };
        let Some(node) = self
            .project
            .graph()
            .nodes()
            .iter()
            .find(|node| node.id == draft.node_id)
        else {
            return (false, false, false);
        };
        let output_dirty = matches!(node.node_type, NodeType::Shader | NodeType::Constant)
            && (draft.auto_size != (node.data.auto_size != Some(false))
                || (!draft.auto_size
                    && (draft.output_width as u32 != node.data.width.unwrap_or(512)
                        || draft.output_height as u32 != node.data.height.unwrap_or(512))));
        (
            draft.label != node.data.label,
            node.node_type == NodeType::Shader && draft.shader_code != node.data.shader_code,
            output_dirty,
        )
    }

    fn inspector_is_dirty(&self) -> bool {
        let (label, shader, output) = self.inspector_dirty_fields();
        label || shader || output
    }

    fn publish_inspector_dirty(&self) {
        if let Some(window) = self.window.upgrade() {
            let (label, shader, output) = self.inspector_dirty_fields();
            window.set_inspector_label_dirty(label);
            window.set_inspector_shader_dirty(shader);
            window.set_inspector_output_size_dirty(output);
            window.set_inspector_dirty(label || shader || output);
        }
    }

    fn publish_inspector(&mut self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let node = if self.selected_node_ids.len() == 1 {
            self.project
                .graph()
                .nodes()
                .iter()
                .find(|node| self.selected_node_ids.contains(&node.id))
        } else {
            None
        };
        let Some(node) = node else {
            self.inspector_draft = None;
            window.set_inspector_node_id("".into());
            window.set_inspector_selection_message(
                if self.selected_node_ids.is_empty() {
                    "Select a node to inspect its properties."
                } else {
                    "Select one node to edit its properties."
                }
                .into(),
            );
            self.publish_inspector_dirty();
            return;
        };
        if self
            .inspector_draft
            .as_ref()
            .map(|draft| draft.node_id.as_str())
            != Some(node.id.as_str())
        {
            self.inspector_draft = Some(InspectorDraft::from_node(node));
        }
        let draft = self
            .inspector_draft
            .as_ref()
            .expect("selected node has a draft");
        window.set_inspector_node_id(node.id.as_str().into());
        window.set_inspector_node_type(format!("{:?}", node.node_type).into());
        window.set_inspector_is_shader(node.node_type == NodeType::Shader);
        window.set_inspector_supports_output_size(matches!(
            node.node_type,
            NodeType::Shader | NodeType::Constant
        ));
        window.set_inspector_node_label(draft.label.as_str().into());
        window.set_inspector_shader_code(draft.shader_code.as_str().into());
        window.set_inspector_auto_size(draft.auto_size);
        window.set_inspector_output_width(draft.output_width);
        window.set_inspector_output_height(draft.output_height);
        window.set_inspector_validation_message(draft.validation_message.as_str().into());
        window.set_inspector_ports(slint::ModelRc::new(slint::VecModel::from(
            node.data
                .inputs
                .iter()
                .map(|port| (port, false))
                .chain(node.data.outputs.iter().map(|port| (port, true)))
                .map(|(port, output)| crate::InspectorPort {
                    label: port.label.as_str().into(),
                    data_type: format!("{:?}", port.data_type).into(),
                    output,
                })
                .collect::<Vec<_>>(),
        )));
        self.publish_inspector_dirty();
    }

    fn apply_inspector(&mut self, change: InspectorChange) {
        if self.busy {
            return;
        }
        let Some(draft) = &self.inspector_draft else {
            return;
        };
        let node_id = draft.node_id.clone();
        match commit_inspector(Arc::make_mut(&mut self.project), draft, change) {
            Ok(changed) => {
                let node = self
                    .project
                    .graph()
                    .nodes()
                    .iter()
                    .find(|node| node.id == node_id)
                    .expect("committed inspector node exists");
                let fresh = InspectorDraft::from_node(node);
                let draft = self
                    .inspector_draft
                    .as_mut()
                    .expect("committed draft exists");
                match change {
                    InspectorChange::Label => draft.label = fresh.label,
                    InspectorChange::Shader => {
                        draft.shader_code = fresh.shader_code;
                        draft.validation_message = fresh.validation_message;
                    }
                    InspectorChange::OutputSize => {
                        draft.auto_size = fresh.auto_size;
                        draft.output_width = fresh.output_width;
                        draft.output_height = fresh.output_height;
                    }
                }
                if changed {
                    self.undo_edits
                        .push(if matches!(change, InspectorChange::Label) {
                            EditKind::Layout
                        } else {
                            EditKind::Content
                        });
                    self.redo_edits.clear();
                    if matches!(change, InspectorChange::Label) {
                        self.publish_graph_projection();
                    } else {
                        self.pending_connection = None;
                        self.publish_project();
                    }
                } else {
                    self.publish_inspector();
                }
                self.set_edit_status(change.status());
            }
            Err(error) => {
                self.set_edit_status(&format!(
                    "Cannot apply inspector edit; project unchanged, draft retained: {}",
                    error.to_json()
                ));
            }
        }
    }

    fn set_edit_status(&self, status: &str) {
        if let Some(window) = self.window.upgrade() {
            window.set_io_status(status.into());
        }
    }

    fn publish_edit_state(&mut self) {
        self.selected_node_ids.retain(|id| {
            self.project
                .graph()
                .nodes()
                .iter()
                .any(|node| &node.id == id)
        });
        if let Some(window) = self.window.upgrade() {
            window
                .set_selected_edge_id(self.selected_edge_id.as_deref().unwrap_or_default().into());
            window.set_connection_active(self.pending_connection.is_some());
            window.set_can_copy(!self.selected_node_ids.is_empty());
            window.set_can_paste(self.clipboard.is_some());
            window.set_graph_selection_count(self.selected_node_ids.len() as i32);
            let source = self
                .pending_connection
                .as_ref()
                .map(|pending| &pending.source);
            window.set_pending_node_id(
                source
                    .map(|source| source.node_id().as_str())
                    .unwrap_or_default()
                    .into(),
            );
            window.set_pending_port_id(
                source
                    .map(|source| source.port_id().as_str())
                    .unwrap_or_default()
                    .into(),
            );
            window.set_can_undo(!self.undo_edits.is_empty());
            window.set_can_redo(!self.redo_edits.is_empty());
        }
    }

    fn select_port(&mut self, node_id: &str, port_id: &str, output: bool) {
        if self.edits_blocked() {
            return;
        }
        let key = PortKey::new(NodeId::new(node_id), PortId::new(port_id));
        if output {
            // A source click during reconnection changes the source without losing the old edge.
            let edge_id = self
                .pending_connection
                .take()
                .and_then(|pending| pending.edge_id);
            self.pending_connection = Some(PendingConnection {
                source: key,
                edge_id,
            });
            self.publish_edit_state();
            self.set_edit_status("Output selected. Click an input to connect, or Cancel.");
            return;
        }
        let Some(pending) = &self.pending_connection else {
            let edge_id = self
                .project
                .graph()
                .edges()
                .iter()
                .find(|edge| edge.target == node_id && edge.target_handle == port_id)
                .map(|edge| edge.id.clone());
            if let Some(edge_id) = edge_id {
                self.select_edge(&edge_id);
            } else {
                self.set_edit_status("Choose an output first, then an input. A connected input selects its existing edge.");
            }
            return;
        };
        let result = commit_connection(
            Arc::make_mut(&mut self.project),
            pending.source.clone(),
            key,
            pending.edge_id.as_deref(),
        );
        match result {
            Ok(changed) => {
                self.pending_connection = None;
                self.selected_edge_id = None;
                if changed {
                    self.finish_topology_edit("Connection applied through the SDK.");
                } else {
                    self.publish_edit_state();
                    self.set_edit_status("Connection unchanged; no revision created.");
                }
            }
            Err(error) => self.set_edit_status(&format!(
                "Cannot connect; project unchanged: {}",
                error.to_json()
            )),
        }
    }

    fn select_edge(&mut self, edge_id: &str) {
        if self.edits_blocked() {
            return;
        }
        if self
            .project
            .graph()
            .edges()
            .iter()
            .any(|edge| edge.id == edge_id)
        {
            self.selected_edge_id = Some(edge_id.to_owned());
            self.selected_node_ids.clear();
            self.pending_connection = None;
            self.publish_graph_projection();
            self.set_edit_status("Connection selected. Reconnect changes its endpoints atomically; Delete edge removes it.");
        }
    }

    fn reconnect_edge(&mut self) {
        if self.edits_blocked() {
            return;
        }
        let Some(edge) = self
            .project
            .graph()
            .edges()
            .iter()
            .find(|edge| Some(&edge.id) == self.selected_edge_id.as_ref())
        else {
            return;
        };
        self.pending_connection = Some(PendingConnection {
            source: PortKey::new(NodeId::new(&edge.source), PortId::new(&edge.source_handle)),
            edge_id: Some(edge.id.clone()),
        });
        self.publish_edit_state();
        self.set_edit_status("Reconnecting: choose a new input, or an output then input. The old edge remains until a valid commit.");
    }

    fn delete_edge(&mut self) {
        if self.edits_blocked() {
            return;
        }
        let Some(edge_id) = self.selected_edge_id.clone() else {
            return;
        };
        let revision = self.project.graph_revision();
        match Arc::make_mut(&mut self.project)
            .apply_graph_command(GraphCommand::Disconnect { edge_id }, revision)
        {
            Ok(_) => {
                self.selected_edge_id = None;
                self.pending_connection = None;
                self.finish_topology_edit("Connection deleted through the SDK.");
            }
            Err(error) => self.set_edit_status(&format!(
                "Cannot delete connection; project unchanged: {}",
                error.to_json()
            )),
        }
    }

    fn finish_topology_edit(&mut self, status: &str) {
        self.undo_edits.push(EditKind::Topology);
        self.redo_edits.clear();
        self.publish_project();
        self.set_edit_status(status);
    }

    fn restore_edit(&mut self, redo: bool) {
        if self.edits_blocked() {
            return;
        }
        let Some(kind) = (if redo {
            &self.redo_edits
        } else {
            &self.undo_edits
        })
        .last()
        .copied() else {
            return;
        };
        let revision = self.project.graph_revision();
        let project = Arc::make_mut(&mut self.project);
        let result = if redo {
            project.redo_graph(revision)
        } else {
            project.rollback_graph(revision)
        };
        match result {
            Ok(_) => {
                if redo {
                    self.redo_edits.pop();
                    self.undo_edits.push(kind);
                } else {
                    self.undo_edits.pop();
                    self.redo_edits.push(kind);
                }
                self.selected_edge_id = None;
                self.pending_connection = None;
                self.inspector_draft = None;
                match kind {
                    EditKind::Layout => {
                        self.publish_graph_projection();
                    }
                    EditKind::Topology | EditKind::Content => self.publish_project(),
                }
                self.set_edit_status(if redo {
                    "Graph edit redone."
                } else {
                    "Graph edit undone."
                });
            }
            Err(error) => {
                self.set_edit_status(&format!("Cannot restore graph edit: {}", error.to_json()))
            }
        }
    }

    fn copy_selection(&mut self) {
        if self.edits_blocked() {
            return;
        }
        if let Some(clipboard) = GraphClipboard::capture(&self.project, &self.selected_node_ids) {
            let count = clipboard.node_count();
            self.clipboard = Some(clipboard);
            self.publish_edit_state();
            self.set_edit_status(&format!(
                "Copied {count} nodes and their internal connections to the graph clipboard."
            ));
        } else {
            self.set_edit_status("Select nodes to copy; the previous graph clipboard was kept.");
        }
    }

    fn paste_clipboard(&mut self) {
        if self.edits_blocked() {
            return;
        }
        let Some(clipboard) = &mut self.clipboard else {
            self.set_edit_status("The graph clipboard is empty. Select nodes and Copy first.");
            return;
        };
        match clipboard.paste(Arc::make_mut(&mut self.project)) {
            Ok(ids) => {
                let count = ids.len();
                self.selected_node_ids = ids;
                self.selected_edge_id = None;
                self.pending_connection = None;
                self.finish_topology_edit(&format!(
                    "Pasted {count} nodes as one graph edit. Undo removes the pasted subgraph."
                ));
            }
            Err(error) => self.set_edit_status(&format!(
                "Cannot paste; project unchanged: {}",
                error.to_json()
            )),
        }
    }

    fn open_path(&mut self, path: PathBuf) {
        self.request_file(path, false);
    }

    fn save_path(&mut self, path: PathBuf) {
        self.request_file(path, true);
    }

    fn request_file(&mut self, path: PathBuf, save: bool) {
        if self.edits_blocked() {
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
                    self.selected_node_ids.clear();
                    self.inspector_draft = None;
                    self.graph_stress_count = None;
                    self.selected_edge_id = None;
                    self.pending_connection = None;
                    self.undo_edits.clear();
                    self.redo_edits.clear();
                    window.set_project_path(path.to_string_lossy().as_ref().into());
                    window.set_io_status(
                        format!("Opened {} through the Rust SDK.", path.display()).into(),
                    );
                    self.publish_project();
                    window.invoke_reset_graph_view();
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

fn commit_inspector(
    project: &mut Project,
    draft: &InspectorDraft,
    change: InspectorChange,
) -> Result<bool, SdkError> {
    let node = project
        .graph()
        .nodes()
        .iter()
        .find(|node| node.id == draft.node_id)
        .ok_or_else(|| {
            SdkError::new(
                open_quartz_sdk::SdkErrorCode::UnknownNode,
                "Inspector node no longer exists",
            )
        })?;
    let command = match change {
        InspectorChange::Label => {
            if node.data.label == draft.label {
                return Ok(false);
            }
            let mut data = node.data.clone();
            data.label = draft.label.clone();
            GraphCommand::UpdateNodeData {
                node_id: NodeId::new(&node.id),
                data,
            }
        }
        InspectorChange::Shader => {
            if node.node_type != NodeType::Shader {
                return Err(SdkError::new(
                    open_quartz_sdk::SdkErrorCode::InvalidState,
                    "Only Shader nodes have editable WGSL source",
                ));
            }
            if node.data.shader_code == draft.shader_code {
                return Ok(false);
            }
            GraphCommand::UpdateShaderCode {
                node_id: NodeId::new(&node.id),
                shader_code: draft.shader_code.clone(),
            }
        }
        InspectorChange::OutputSize => {
            if !matches!(node.node_type, NodeType::Shader | NodeType::Constant) {
                return Err(SdkError::new(
                    open_quartz_sdk::SdkErrorCode::InvalidState,
                    "This node has no output size",
                ));
            }
            if !draft.auto_size
                && (!(1..=8192).contains(&draft.output_width)
                    || !(1..=8192).contains(&draft.output_height))
            {
                return Err(SdkError::new(
                    open_quartz_sdk::SdkErrorCode::InvalidGraph,
                    "Output dimensions must be between 1 and 8192",
                ));
            }
            let same = draft.auto_size == (node.data.auto_size != Some(false))
                && (draft.auto_size
                    || (draft.output_width as u32 == node.data.width.unwrap_or(512)
                        && draft.output_height as u32 == node.data.height.unwrap_or(512)));
            if same {
                return Ok(false);
            }
            let mut data = node.data.clone();
            data.auto_size = Some(draft.auto_size);
            data.width = (!draft.auto_size).then_some(draft.output_width as u32);
            data.height = (!draft.auto_size).then_some(draft.output_height as u32);
            GraphCommand::UpdateNodeData {
                node_id: NodeId::new(&node.id),
                data,
            }
        }
    };
    project
        .apply_graph_command(command, project.graph_revision())
        .map(|_| true)
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

fn commit_connection(
    project: &mut Project,
    source: PortKey,
    target: PortKey,
    reconnect_edge_id: Option<&str>,
) -> Result<bool, SdkError> {
    let unchanged = project.graph().edges().iter().any(|edge| {
        edge.source == source.node_id().as_str()
            && edge.source_handle == source.port_id().as_str()
            && edge.target == target.node_id().as_str()
            && edge.target_handle == target.port_id().as_str()
            && reconnect_edge_id.is_none_or(|id| id == edge.id)
    });
    if unchanged {
        return Ok(false);
    }
    let revision = project.graph_revision();
    if let Some(edge_id) = reconnect_edge_id {
        let mut graph = project.graph_snapshot();
        graph.edit(&mut |edit| {
            edit.disconnect(edge_id)?;
            edit.connect(source.clone(), target.clone())
        })?;
        project.replace_graph(graph, revision)?;
    } else {
        project.apply_graph_command(GraphCommand::Connect { source, target }, revision)?;
    }
    Ok(true)
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
    use open_quartz_sdk::{
        Environment, NodeFactoryRequest, NodeId, OpenQuartz, PortId, PortKey, Position,
        SdkErrorCode,
    };
    use slint::Model;

    use super::{
        commit_connection, commit_inspector, commit_node_position, graph_stress_project,
        sample_project, GraphViewModel, InspectorChange, InspectorDraft,
    };

    #[test]
    fn inspector_fields_commit_independently_and_undo_as_single_edits() {
        let sdk = OpenQuartz::new(Environment::headless());
        let mut project = sample_project(&sdk).unwrap();
        let original = project.graph_snapshot();
        let mut draft = InspectorDraft::from_node(&project.graph().nodes()[0]);
        let revision = project.graph_revision();
        assert!(!commit_inspector(&mut project, &draft, InspectorChange::Label).unwrap());
        assert_eq!(project.graph_revision(), revision);
        draft.label = "Edited gradient".into();
        draft.shader_code = draft.shader_code.replace("0.35", "0.75");
        draft.output_width = 128;
        draft.output_height = 64;
        commit_inspector(&mut project, &draft, InspectorChange::Label).unwrap();
        assert_eq!(project.graph().nodes()[0].data.label, draft.label);
        assert_eq!(
            project.graph().nodes()[0].data.shader_code,
            original.nodes()[0].data.shader_code
        );
        commit_inspector(&mut project, &draft, InspectorChange::OutputSize).unwrap();
        assert_eq!(project.graph().nodes()[0].data.width, Some(128));
        assert_eq!(project.graph().nodes()[0].data.height, Some(64));
        commit_inspector(&mut project, &draft, InspectorChange::Shader).unwrap();
        assert_eq!(project.graph().edges(), original.edges());
        assert_eq!(
            project.graph().nodes()[0].data.outputs,
            original.nodes()[0].data.outputs
        );
        let reloaded = sdk
            .open_project_json(&serde_json::to_string(&project.to_file()).unwrap())
            .unwrap();
        assert_eq!(reloaded.graph(), project.graph());
        project.rollback_graph(project.graph_revision()).unwrap();
        assert_eq!(
            project.graph().nodes()[0].data.shader_code,
            original.nodes()[0].data.shader_code
        );
        assert_eq!(project.graph().nodes()[0].data.width, Some(128));
        project.redo_graph(project.graph_revision()).unwrap();
        assert_eq!(
            project.graph().nodes()[0].data.shader_code,
            draft.shader_code
        );
        draft.auto_size = true;
        commit_inspector(&mut project, &draft, InspectorChange::OutputSize).unwrap();
        assert_eq!(project.graph().nodes()[0].data.auto_size, Some(true));
        assert_eq!(project.graph().nodes()[0].data.width, None);
        assert_eq!(project.graph().nodes()[0].data.height, None);
    }

    #[test]
    fn inspector_rejects_connected_port_removal_without_losing_draft_or_project() {
        let sdk = OpenQuartz::new(Environment::headless());
        let mut project = sample_project(&sdk).unwrap();
        let original = project.graph_snapshot();
        let revision = project.graph_revision();
        let mut draft = InspectorDraft::from_node(&project.graph().nodes()[0]);
        draft.shader_code = "not wgsl".into();
        assert_eq!(
            commit_inspector(&mut project, &draft, InspectorChange::Shader)
                .unwrap_err()
                .code,
            SdkErrorCode::InvalidGraph
        );
        assert_eq!(project.graph(), &original);
        assert_eq!(project.graph_revision(), revision);
        draft.output_width = 0;
        assert_eq!(
            commit_inspector(&mut project, &draft, InspectorChange::OutputSize)
                .unwrap_err()
                .code,
            SdkErrorCode::InvalidGraph
        );
        assert_eq!(project.graph(), &original);
        assert_eq!(project.graph_revision(), revision);
    }

    #[test]
    fn inspector_preserves_sdk_parse_error_until_source_is_repaired() {
        let sdk = OpenQuartz::new(Environment::headless());
        let mut project = sample_project(&sdk).unwrap();
        project
            .apply_graph_command(
                open_quartz_sdk::GraphCommand::Disconnect {
                    edge_id: project.graph().edges()[0].id.clone(),
                },
                project.graph_revision(),
            )
            .unwrap();
        let mut draft = InspectorDraft::from_node(&project.graph().nodes()[0]);
        let source = draft.shader_code.clone();
        draft.shader_code = "not wgsl".into();
        commit_inspector(&mut project, &draft, InspectorChange::Shader).unwrap();
        assert_eq!(project.graph().nodes()[0].data.shader_code, "not wgsl");
        assert!(project.graph().nodes()[0]
            .data
            .extra
            .get("parseError")
            .and_then(serde_json::Value::as_str)
            .is_some());
        let json = serde_json::to_string(&project.to_file()).unwrap();
        let reloaded = sdk.open_project_json(&json).unwrap();
        assert_eq!(
            reloaded.graph().nodes()[0].data.extra,
            project.graph().nodes()[0].data.extra
        );
        draft.shader_code = source;
        commit_inspector(&mut project, &draft, InspectorChange::Shader).unwrap();
        assert!(!project.graph().nodes()[0]
            .data
            .extra
            .contains_key("parseError"));
    }

    #[test]
    fn projected_edges_terminate_at_their_actual_ports() {
        let sdk = OpenQuartz::new(Environment::headless());
        let project = graph_stress_project(&sdk, 3).unwrap();
        let projection = GraphViewModel::from_project(&project, &Default::default());
        for (index, edge) in project.graph().edges().iter().enumerate() {
            let projected = projection.edges.row_data(index).unwrap();
            assert_eq!(projected.edge_id.as_str(), edge.id);
            let source = projection
                .nodes
                .row_data(projected.source_index as usize)
                .unwrap();
            let target = projection
                .nodes
                .row_data(projected.target_index as usize)
                .unwrap();
            let source_port = source
                .ports
                .iter()
                .find(|port| port.output && port.port_id.as_str() == edge.source_handle)
                .unwrap();
            let target_port = target
                .ports
                .iter()
                .find(|port| !port.output && port.port_id.as_str() == edge.target_handle)
                .unwrap();
            assert_eq!(
                (projected.source_x, projected.source_y),
                (source.x + source.width, source.y + source_port.y)
            );
            assert_eq!(
                (projected.target_x, projected.target_y),
                (target.x, target.y + target_port.y)
            );
            assert!(target_port.y < target.height);
        }
        let target = projection.nodes.row_data(2).unwrap();
        let inputs = target
            .ports
            .iter()
            .filter(|port| !port.output)
            .collect::<Vec<_>>();
        assert!(inputs[1].y > inputs[0].y);
    }

    #[test]
    fn projection_represents_selected_subgraph_without_selecting_other_nodes() {
        let sdk = OpenQuartz::new(Environment::headless());
        let project = graph_stress_project(&sdk, 3).unwrap();
        let ids = project.graph().nodes()[..2]
            .iter()
            .map(|node| node.id.clone())
            .collect();
        let projection = GraphViewModel::from_project(&project, &ids);
        let selected = (0..projection.nodes.row_count())
            .filter_map(|row| projection.nodes.row_data(row))
            .filter(|node| node.selected)
            .map(|node| node.node_id.to_string())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(ids, selected);
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

        let projection = GraphViewModel::from_project(&project, &Default::default());
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

        let projection = GraphViewModel::from_project(&first, &Default::default());
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

    fn port_key(
        project: &open_quartz_sdk::Project,
        node: usize,
        output: bool,
        port: usize,
    ) -> PortKey {
        let node = &project.graph().nodes()[node];
        let ports = if output {
            &node.data.outputs
        } else {
            &node.data.inputs
        };
        PortKey::new(NodeId::new(&node.id), PortId::new(&ports[port].id))
    }

    #[test]
    fn occupied_input_replacement_is_one_undoable_persisted_edit() {
        let sdk = OpenQuartz::new(Environment::headless());
        let mut project = graph_stress_project(&sdk, 3).unwrap();
        let before = project.graph_snapshot();
        let source = port_key(&project, 0, true, 0);
        let target = port_key(&project, 2, false, 0);
        let revision = project.graph_revision();
        assert!(commit_connection(&mut project, source.clone(), target.clone(), None).unwrap());
        assert_eq!(project.graph_revision(), revision + 1);
        assert_eq!(project.graph().edges().len(), before.edges.len());
        let after = project.graph_snapshot();
        assert!(after
            .edges
            .iter()
            .any(|edge| edge.source == source.node_id().as_str()
                && edge.target == target.node_id().as_str()
                && edge.target_handle == target.port_id().as_str()));
        let serialized = serde_json::to_string(&project.to_file()).unwrap();
        assert_eq!(
            sdk.open_project_json(&serialized).unwrap().graph_snapshot(),
            after
        );
        assert!(!commit_connection(&mut project, source, target, None).unwrap());
        assert_eq!(project.graph_revision(), revision + 1);
        project.rollback_graph(project.graph_revision()).unwrap();
        assert_eq!(project.graph_snapshot(), before);
        project.redo_graph(project.graph_revision()).unwrap();
        assert_eq!(project.graph_snapshot(), after);
    }

    #[test]
    fn reconnect_moves_edge_and_replaced_input_as_one_transaction() {
        let sdk = OpenQuartz::new(Environment::headless());
        let mut project = graph_stress_project(&sdk, 3).unwrap();
        let before = project.graph_snapshot();
        let old = before.edges[0].clone();
        let source = port_key(&project, 0, true, 0);
        let target = port_key(&project, 2, false, 0);
        let revision = project.graph_revision();
        commit_connection(&mut project, source, target, Some(&old.id)).unwrap();
        assert_eq!(project.graph_revision(), revision + 1);
        assert!(!project.graph().edges().iter().any(|edge| edge.id == old.id));
        assert_eq!(project.graph().edges().len(), before.edges.len() - 1);
        let after = project.graph_snapshot();
        project.rollback_graph(project.graph_revision()).unwrap();
        assert_eq!(project.graph_snapshot(), before);

        // This cycle is discovered only after disconnect/connect in the draft.
        let source = port_key(&project, 2, true, 0);
        let target = port_key(&project, 0, false, 0);
        let revision = project.graph_revision();
        assert!(commit_connection(&mut project, source, target, Some(&old.id)).is_err());
        assert_eq!(project.graph_revision(), revision);
        assert_eq!(project.graph_snapshot(), before);
        // Failed reconnection must not clear the SDK redo stack.
        project.redo_graph(project.graph_revision()).unwrap();
        assert_eq!(project.graph_snapshot(), after);
    }

    #[test]
    fn incompatible_reconnect_preserves_old_edge_and_history() {
        let sdk = OpenQuartz::new(Environment::headless());
        let mut project = sample_project(&sdk).unwrap();
        let initial = project.graph_snapshot();
        project
            .create_graph_node(
                NodeFactoryRequest::Math {
                    position: None,
                    op: "add".into(),
                },
                project.graph_revision(),
            )
            .unwrap();
        let before = project.graph_snapshot();
        let source = port_key(&project, 2, true, 0);
        let target = port_key(&project, 1, false, 0);
        let revision = project.graph_revision();
        let error = commit_connection(&mut project, source, target, Some(&initial.edges[0].id))
            .unwrap_err();
        assert_eq!(error.code, SdkErrorCode::InvalidGraph);
        assert_eq!(project.graph_revision(), revision);
        assert_eq!(project.graph_snapshot(), before);
        project.rollback_graph(project.graph_revision()).unwrap();
        assert_eq!(project.graph_snapshot(), initial);
    }
}
