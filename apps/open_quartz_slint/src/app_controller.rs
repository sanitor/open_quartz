use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use open_quartz_sdk::{
    DataType, Environment, GraphCommand, NodeFactoryRequest, NodeId, NodeType, OpenQuartz, PortId,
    PortKey, Position, Project, SdkError,
};
use slint::{ComponentHandle, Model};

use crate::graph_clipboard::GraphClipboard;
use crate::graph_view_model::GraphViewModel;
use crate::inspector_forms::{FormKind, NodeFormsDraft};
use crate::render_bridge::RenderBridge;
use crate::services::file_service;
use crate::AppWindow;

#[derive(Clone, Debug)]
struct UniformDraft {
    name: String,
    data_type: DataType,
    literal: String,
    baseline_literal: String,
}

#[derive(Clone, Debug)]
struct InspectorDraft {
    node_id: String,
    label: String,
    shader_code: String,
    auto_size: bool,
    output_width: i32,
    output_height: i32,
    validation_message: String,
    uniforms: Vec<UniformDraft>,
    forms: NodeFormsDraft,
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
            uniforms: uniform_drafts(node),
            forms: NodeFormsDraft::from_data(&node.data),
        }
    }

    fn refresh_uniforms(&mut self, node: &open_quartz_sdk::ProjectNode, retain_drafts: bool) {
        let mut uniforms = uniform_drafts(node);
        if retain_drafts {
            for uniform in &mut uniforms {
                if let Some(previous) = self.uniforms.iter().find(|previous| {
                    previous.name == uniform.name && previous.data_type == uniform.data_type
                }) {
                    uniform.literal.clone_from(&previous.literal);
                }
            }
        }
        self.uniforms = uniforms;
    }
}

#[derive(Clone, Copy)]
enum InspectorChange {
    Label,
    Shader,
    OutputSize,
    Uniforms,
    Form(FormKind),
}

impl InspectorChange {
    fn status(self) -> &'static str {
        match self {
            Self::Label => "Node name applied through the SDK.",
            Self::Shader => "Shader source applied through the SDK.",
            Self::OutputSize => "Output size applied through the SDK.",
            Self::Uniforms => "Uniform values applied through the SDK.",
            Self::Form(FormKind::Sampling) => "Sampling applied through the SDK.",
            Self::Form(FormKind::Framebuffer) => "Framebuffer configuration saved through the SDK; raw resource loading is not available.",
            Self::Form(FormKind::Video) => "Video configuration saved through the SDK; video/camera opening is not available.",
        }
    }
}

pub struct AppController {
    window: slint::Weak<AppWindow>,
    project: Arc<Project>,
    renderer: Option<RenderBridge>,
    onnx_states: BTreeMap<String, crate::onnx_runtime::LiveOnnxState>,
    selected_node_ids: BTreeSet<String>,
    inspector_draft: Option<InspectorDraft>,
    inspector_uniforms: Rc<slint::VecModel<crate::InspectorUniform>>,
    inspector_forms: Rc<slint::VecModel<crate::InspectorFormGroup>>,
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
            onnx_states: BTreeMap::new(),
            selected_node_ids: BTreeSet::new(),
            inspector_draft: None,
            inspector_uniforms: Rc::new(slint::VecModel::from(
                Vec::<crate::InspectorUniform>::new(),
            )),
            inspector_forms: Rc::new(slint::VecModel::default()),
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
        window.set_inspector_uniforms(slint::ModelRc::from(
            controller.borrow().inspector_uniforms.clone(),
        ));
        window.set_inspector_forms(slint::ModelRc::from(
            controller.borrow().inspector_forms.clone(),
        ));
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
        window.on_inspector_uniform_draft_changed(move |name, literal| {
            if let Some(controller) = weak.upgrade() {
                controller
                    .borrow_mut()
                    .change_uniform_draft(name.as_str(), literal.as_str());
            }
        });
        let weak = Rc::downgrade(&controller);
        window.on_inspector_apply_uniforms(move || {
            if let Some(controller) = weak.upgrade() {
                controller
                    .borrow_mut()
                    .apply_inspector(InspectorChange::Uniforms);
            }
        });
        let weak = Rc::downgrade(&controller);
        window.on_inspector_form_draft_changed(move |key, value| {
            if let Some(controller) = weak.upgrade() {
                controller
                    .borrow_mut()
                    .change_form_draft(key.as_str(), value.as_str());
            }
        });
        let weak = Rc::downgrade(&controller);
        window.on_inspector_apply_form(move |key| {
            if let (Some(controller), Some(kind)) =
                (weak.upgrade(), FormKind::from_key(key.as_str()))
            {
                controller
                    .borrow_mut()
                    .apply_inspector(InspectorChange::Form(kind));
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
        window.on_inspector_load_onnx(move |path| {
            if let Some(controller) = weak.upgrade() {
                controller
                    .borrow_mut()
                    .change_onnx_session(Some(path.to_string()));
            }
        });
        let weak = Rc::downgrade(&controller);
        window.on_inspector_unload_onnx(move || {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().change_onnx_session(None);
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
        self.onnx_states.clear();
        self.publish_onnx_runtime();
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

    fn inspector_dirty_fields(&self) -> (bool, bool, bool, bool) {
        let Some(draft) = &self.inspector_draft else {
            return (false, false, false, false);
        };
        let Some(node) = self
            .project
            .graph()
            .nodes()
            .iter()
            .find(|node| node.id == draft.node_id)
        else {
            return (false, false, false, false);
        };
        let output_dirty = matches!(node.node_type, NodeType::Shader | NodeType::Constant)
            && (draft.auto_size != (node.data.auto_size != Some(false))
                || (!draft.auto_size
                    && (draft.output_width as u32 != node.data.width.unwrap_or(512)
                        || draft.output_height as u32 != node.data.height.unwrap_or(512))));
        let uniform_dirty = uniform_drafts_dirty(&self.project, node, draft);
        (
            draft.label != node.data.label,
            node.node_type == NodeType::Shader && draft.shader_code != node.data.shader_code,
            output_dirty,
            uniform_dirty,
        )
    }

    fn inspector_is_dirty(&self) -> bool {
        let (label, shader, output, uniform) = self.inspector_dirty_fields();
        label || shader || output || uniform || self.forms_dirty()
    }

    fn publish_inspector_dirty(&self) {
        if let Some(window) = self.window.upgrade() {
            let (label, shader, output, uniform) = self.inspector_dirty_fields();
            window.set_inspector_label_dirty(label);
            window.set_inspector_shader_dirty(shader);
            window.set_inspector_output_size_dirty(output);
            window.set_inspector_uniform_dirty(uniform);
            window.set_inspector_dirty(label || shader || output || uniform || self.forms_dirty());
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
            window.set_inspector_onnx_config(slint::ModelRc::default());
            window.set_inspector_onnx_saved(slint::ModelRc::default());
            window.set_inspector_onnx_live(slint::ModelRc::default());
            window.set_inspector_onnx_path("".into());
            window.set_inspector_selection_message(
                if self.selected_node_ids.is_empty() {
                    "Select a node to inspect its properties."
                } else {
                    "Select one node to edit its properties."
                }
                .into(),
            );
            self.publish_uniforms();
            self.publish_forms();
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
            window.set_inspector_onnx_path("".into());
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
        let (configuration, saved) = crate::inspector_onnx::inspect(node, self.project.graph());
        window.set_inspector_onnx_config(slint::ModelRc::new(slint::VecModel::from(configuration)));
        window.set_inspector_onnx_saved(slint::ModelRc::new(slint::VecModel::from(saved)));
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
        self.publish_uniforms();
        self.publish_forms();
        self.publish_inspector_dirty();
        self.publish_onnx_runtime();
    }

    fn publish_onnx_runtime(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let id = window.get_inspector_node_id();
        let state = self.onnx_states.get(id.as_str());
        window.set_inspector_onnx_available(
            self.renderer.is_some() && self.graph_stress_count.is_none(),
        );
        window.set_inspector_onnx_loading(state.is_some_and(|state| state.loading));
        window.set_inspector_onnx_attached(state.is_some_and(|state| state.attached));
        window.set_inspector_onnx_status(
            state
                .map(|state| state.status.as_str())
                .unwrap_or(if self.renderer.is_some() {
                    "Not loaded · no session attached"
                } else {
                    "Native renderer unavailable"
                })
                .into(),
        );
        let rows = state
            .map(|state| {
                [
                    ("Live backend", &state.backend),
                    ("Session inputs", &state.inputs),
                    ("Session outputs", &state.outputs),
                    ("Latest inference result", &state.result),
                ]
                .into_iter()
                .filter(|(_, value)| !value.is_empty())
                .map(|(label, value)| crate::InspectorOnnxRow {
                    label: label.into(),
                    value: value.as_str().into(),
                })
                .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        window.set_inspector_onnx_live(slint::ModelRc::new(slint::VecModel::from(rows)));
    }

    fn change_onnx_session(&mut self, path: Option<String>) {
        if self.edits_blocked() || self.selected_node_ids.len() != 1 {
            return;
        }
        let Some(node) = self.project.graph().nodes().iter().find(|node| {
            node.node_type == NodeType::Onnx && self.selected_node_ids.contains(&node.id)
        }) else {
            return;
        };
        let id = node.id.clone();
        let Some(renderer) = &self.renderer else {
            return;
        };
        let result = match path {
            Some(path) => renderer.load_onnx(id, path),
            None => renderer.unload_onnx(id),
        };
        if let Err(error) = result {
            self.set_edit_status(&error);
        }
    }

    fn forms_dirty(&self) -> bool {
        self.inspector_draft.as_ref().is_some_and(|draft| {
            self.project
                .graph()
                .nodes()
                .iter()
                .find(|node| node.id == draft.node_id)
                .is_some_and(|node| {
                    [FormKind::Sampling, FormKind::Framebuffer, FormKind::Video]
                        .into_iter()
                        .any(|kind| draft.forms.dirty(&node.data, kind))
                })
        })
    }

    fn change_form_draft(&mut self, key: &str, value: &str) {
        if self.busy {
            return;
        }
        if let Some(draft) = self.inspector_draft.as_mut() {
            if let Some(node) = self
                .project
                .graph()
                .nodes()
                .iter()
                .find(|node| node.id == draft.node_id)
            {
                let mut next = draft.forms.clone();
                if next
                    .change(key, value)
                    .is_some_and(|kind| kind.supports(&node.data))
                {
                    draft.forms = next;
                }
            }
        }
        self.publish_forms();
        self.publish_inspector_dirty();
    }

    fn publish_forms(&self) {
        let rows = self
            .inspector_draft
            .as_ref()
            .and_then(|draft| {
                self.project
                    .graph()
                    .nodes()
                    .iter()
                    .find(|node| node.id == draft.node_id)
                    .map(|node| draft.forms.groups(&node.data))
            })
            .unwrap_or_default();
        if rows.len() != self.inspector_forms.row_count()
            || rows.iter().enumerate().any(|(i, row)| {
                self.inspector_forms
                    .row_data(i)
                    .is_none_or(|old| old.key != row.key)
            })
        {
            self.inspector_forms.set_vec(rows);
            return;
        }
        for (index, mut row) in rows.into_iter().enumerate() {
            let old = self.inspector_forms.row_data(index).expect("existing form");
            if let Some(fields) = old
                .fields
                .as_any()
                .downcast_ref::<slint::VecModel<crate::InspectorFormField>>()
            {
                let same_schema = fields.row_count() == row.fields.row_count()
                    && (0..fields.row_count()).all(|i| {
                        fields.row_data(i).map(|f| f.key) == row.fields.row_data(i).map(|f| f.key)
                    });
                if same_schema {
                    for i in 0..fields.row_count() {
                        let new = row.fields.row_data(i).expect("new field");
                        let current = fields.row_data(i).expect("existing field");
                        if new.value != current.value {
                            fields.set_row_data(i, new);
                        }
                    }
                    row.fields = old.fields.clone();
                }
            }
            if row.fields != old.fields || row.dirty != old.dirty || row.error != old.error {
                self.inspector_forms.set_row_data(index, row);
            }
        }
    }

    fn change_uniform_draft(&mut self, name: &str, literal: &str) {
        if self.busy {
            return;
        }
        let Some(draft) = self.inspector_draft.as_mut() else {
            return;
        };
        let Some(node) = self
            .project
            .graph()
            .nodes()
            .iter()
            .find(|node| node.id == draft.node_id)
        else {
            return;
        };
        if node.node_type != NodeType::Shader {
            return;
        }
        let Some(port) = node.data.inputs.iter().find(|port| port.label == name) else {
            return;
        };
        if uniform_connected(&self.project, node, port) {
            return;
        }
        if let Some(uniform) = draft
            .uniforms
            .iter_mut()
            .find(|uniform| uniform.name == name && uniform.data_type == port.data_type)
        {
            uniform.literal = literal.to_owned();
        }
        self.publish_uniforms();
        self.publish_inspector_dirty();
    }

    fn publish_uniforms(&self) {
        let rows = self
            .inspector_draft
            .as_ref()
            .map(|draft| uniform_rows(&self.project, draft))
            .unwrap_or_default();
        let valid = rows.iter().all(|row| row.connected || row.valid);
        let same_schema = self.inspector_uniforms.row_count() == rows.len()
            && rows.iter().enumerate().all(|(index, row)| {
                self.inspector_uniforms
                    .row_data(index)
                    .is_some_and(|old| old.name == row.name && old.data_type == row.data_type)
            });
        if same_schema {
            for (index, row) in rows.into_iter().enumerate() {
                if self.inspector_uniforms.row_data(index).as_ref() != Some(&row) {
                    self.inspector_uniforms.set_row_data(index, row);
                }
            }
        } else {
            self.inspector_uniforms.set_vec(rows);
        }
        if let Some(window) = self.window.upgrade() {
            window.set_inspector_uniforms_valid(valid);
        }
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
                        draft.refresh_uniforms(node, true);
                    }
                    InspectorChange::OutputSize => {
                        draft.auto_size = fresh.auto_size;
                        draft.output_width = fresh.output_width;
                        draft.output_height = fresh.output_height;
                    }
                    InspectorChange::Uniforms => draft.refresh_uniforms(node, false),
                    InspectorChange::Form(kind) => draft.forms.restore_group(&node.data, kind),
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
                    self.onnx_states.clear();
                    if let Some(renderer) = &mut self.renderer {
                        renderer.begin_project();
                    }
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
            if let Some(states) = update.onnx {
                self.onnx_states = states;
                self.publish_onnx_runtime();
            }
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
        InspectorChange::Uniforms => {
            if node.node_type != NodeType::Shader {
                return Err(SdkError::new(
                    open_quartz_sdk::SdkErrorCode::InvalidState,
                    "Only Shader nodes have editable uniforms",
                ));
            }
            let mut data = node.data.clone();
            let mut changed = false;
            for port in node.data.inputs.iter().filter(|port| {
                uniform_shape(port.data_type).is_some() && !uniform_connected(project, node, port)
            }) {
                let Some(uniform) = draft.uniforms.iter().find(|uniform| {
                    uniform.name == port.label && uniform.data_type == port.data_type
                }) else {
                    continue;
                };
                let value =
                    parse_uniform_literal(port.data_type, &uniform.literal).map_err(|message| {
                        SdkError::new(
                            open_quartz_sdk::SdkErrorCode::InvalidGraph,
                            format!("{}: {message}", port.label),
                        )
                    })?;
                if value != effective_uniform_value(node, port) {
                    data.uniforms.insert(port.label.clone(), value);
                    changed = true;
                }
            }
            if !changed {
                return Ok(false);
            }
            GraphCommand::UpdateNodeData {
                node_id: NodeId::new(&node.id),
                data,
            }
        }
        InspectorChange::Form(kind) => {
            let data = draft
                .forms
                .updated_data(&node.data, kind)
                .map_err(|message| {
                    SdkError::new(open_quartz_sdk::SdkErrorCode::InvalidGraph, message)
                })?;
            if data == node.data {
                return Ok(false);
            }
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

fn uniform_shape(data_type: DataType) -> Option<usize> {
    Some(match data_type {
        DataType::Float | DataType::Int | DataType::Uint | DataType::Bool => 1,
        DataType::Vec2 | DataType::Ivec2 | DataType::Uvec2 => 2,
        DataType::Vec3 | DataType::Ivec3 | DataType::Uvec3 => 3,
        DataType::Vec4 | DataType::Ivec4 | DataType::Uvec4 | DataType::Mat2 => 4,
        DataType::Mat3 => 9,
        DataType::Mat4 => 16,
        _ => return None,
    })
}

fn default_uniform_value(data_type: DataType) -> serde_json::Value {
    let zero = match data_type {
        DataType::Bool => serde_json::Value::Bool(false),
        DataType::Int | DataType::Ivec2 | DataType::Ivec3 | DataType::Ivec4 => {
            serde_json::json!(0_i32)
        }
        DataType::Uint | DataType::Uvec2 | DataType::Uvec3 | DataType::Uvec4 => {
            serde_json::json!(0_u32)
        }
        _ => serde_json::json!(0.0),
    };
    match uniform_shape(data_type) {
        Some(arity) if arity > 1 => serde_json::Value::Array(vec![zero; arity]),
        _ => zero,
    }
}

fn format_uniform_literal(value: &serde_json::Value) -> String {
    fn scalar(value: &serde_json::Value) -> String {
        value
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| value.to_string())
    }
    match value.as_array() {
        Some(values) => values.iter().map(scalar).collect::<Vec<_>>().join(", "),
        None => scalar(value),
    }
}

fn uniform_drafts(node: &open_quartz_sdk::ProjectNode) -> Vec<UniformDraft> {
    if node.node_type != NodeType::Shader {
        return Vec::new();
    }
    node.data
        .inputs
        .iter()
        .filter(|port| uniform_shape(port.data_type).is_some())
        .map(|port| {
            let literal = format_uniform_literal(&effective_uniform_value(node, port));
            UniformDraft {
                name: port.label.clone(),
                data_type: port.data_type,
                baseline_literal: literal.clone(),
                literal,
            }
        })
        .collect()
}

fn effective_uniform_value(
    node: &open_quartz_sdk::ProjectNode,
    port: &open_quartz_sdk::Port,
) -> serde_json::Value {
    let value = node
        .data
        .uniforms
        .get(&port.label)
        .cloned()
        .unwrap_or_else(|| default_uniform_value(port.data_type));
    parse_uniform_literal(port.data_type, &format_uniform_literal(&value)).unwrap_or(value)
}

fn uniform_connected(
    project: &Project,
    node: &open_quartz_sdk::ProjectNode,
    port: &open_quartz_sdk::Port,
) -> bool {
    project
        .graph()
        .edges()
        .iter()
        .any(|edge| edge.target == node.id && edge.target_handle == port.id)
}

fn uniform_drafts_dirty(
    project: &Project,
    node: &open_quartz_sdk::ProjectNode,
    draft: &InspectorDraft,
) -> bool {
    node.node_type == NodeType::Shader
        && node.data.inputs.iter().any(|port| {
            uniform_shape(port.data_type).is_some()
                && !uniform_connected(project, node, port)
                && draft
                    .uniforms
                    .iter()
                    .find(|uniform| {
                        uniform.name == port.label && uniform.data_type == port.data_type
                    })
                    .is_some_and(|uniform| {
                        uniform.literal != uniform.baseline_literal
                            && parse_uniform_literal(port.data_type, &uniform.literal)
                                .as_ref()
                                .ok()
                                != Some(&effective_uniform_value(node, port))
                    })
        })
}

fn uniform_rows(project: &Project, draft: &InspectorDraft) -> Vec<crate::InspectorUniform> {
    let Some(node) = project
        .graph()
        .nodes()
        .iter()
        .find(|node| node.id == draft.node_id && node.node_type == NodeType::Shader)
    else {
        return Vec::new();
    };
    node.data
        .inputs
        .iter()
        .filter(|port| uniform_shape(port.data_type).is_some())
        .map(|port| {
            let literal = draft
                .uniforms
                .iter()
                .find(|uniform| uniform.name == port.label && uniform.data_type == port.data_type)
                .map(|uniform| uniform.literal.clone())
                .unwrap_or_else(|| format_uniform_literal(&effective_uniform_value(node, port)));
            let connected = uniform_connected(project, node, port);
            let error = if connected {
                String::new()
            } else {
                parse_uniform_literal(port.data_type, &literal)
                    .err()
                    .unwrap_or_default()
            };
            crate::InspectorUniform {
                name: port.label.as_str().into(),
                data_type: format!("{:?}", port.data_type).into(),
                value: literal.into(),
                connected,
                valid: error.is_empty(),
                error: error.into(),
            }
        })
        .collect()
}

fn parse_uniform_literal(data_type: DataType, literal: &str) -> Result<serde_json::Value, String> {
    let arity = uniform_shape(data_type).ok_or_else(|| "Unsupported uniform type".to_owned())?;
    let parts = literal.split(',').map(str::trim).collect::<Vec<_>>();
    if parts.len() != arity {
        return Err(format!(
            "Expected {arity} component(s), got {}",
            parts.len()
        ));
    }
    let values = parts
        .into_iter()
        .map(|part| match data_type {
            DataType::Bool => match part {
                "true" | "1" => Ok(serde_json::Value::Bool(true)),
                "false" | "0" => Ok(serde_json::Value::Bool(false)),
                _ => Err("Expected true, false, 1, or 0".to_owned()),
            },
            DataType::Int | DataType::Ivec2 | DataType::Ivec3 | DataType::Ivec4 => part
                .parse::<i32>()
                .map(|value| serde_json::json!(value))
                .map_err(|_| "Expected a signed 32-bit integer".to_owned()),
            DataType::Uint | DataType::Uvec2 | DataType::Uvec3 | DataType::Uvec4 => {
                if part.starts_with('-') {
                    return Err("Expected an unsigned 32-bit integer".to_owned());
                }
                part.parse::<u32>()
                    .map(|value| serde_json::json!(value))
                    .map_err(|_| "Expected an unsigned 32-bit integer".to_owned())
            }
            _ => part
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite() && (*value as f32).is_finite())
                .map(|value| serde_json::json!(value))
                .ok_or_else(|| "Expected a finite 32-bit float".to_owned()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if arity == 1 {
        Ok(values.into_iter().next().expect("scalar has one component"))
    } else {
        Ok(serde_json::Value::Array(values))
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
        DataType, Environment, GraphCommand, NodeFactoryRequest, NodeId, NodeType, OpenQuartz,
        Port, PortDirection, PortId, PortKey, Position, Project, SdkErrorCode,
    };
    use slint::Model;

    use super::{
        commit_connection, commit_inspector, commit_node_position, graph_stress_project,
        sample_project, GraphViewModel, InspectorChange, InspectorDraft,
    };

    #[test]
    fn node_forms_commit_independently_with_single_history_and_save_reload() {
        use crate::inspector_forms::FormKind;
        use open_quartz_sdk::{InputMode, TextureFilter};
        for (kind, changes) in [
            (FormKind::Sampling, vec![("filter", "1"), ("wrap", "2")]),
            (
                FormKind::Framebuffer,
                vec![
                    ("fb-format", "3"),
                    ("fb-width", "32"),
                    ("fb-height", "16"),
                    ("fb-stride", "256"),
                ],
            ),
            (
                FormKind::Video,
                vec![
                    ("video-source", "1"),
                    ("video-device", "camera-1"),
                    ("video-loop", "false"),
                    ("video-rate", "2.5"),
                ],
            ),
        ] {
            let mut project = uniform_project(&[]);
            if kind != FormKind::Sampling {
                let mut graph = project.graph_snapshot();
                let node = &mut graph.nodes[0];
                node.node_type = NodeType::Input;
                node.data.node_type = NodeType::Input;
                node.data.input_data_type = Some(DataType::Sampler2d);
                node.data.input_mode = Some(if kind == FormKind::Video {
                    InputMode::Video
                } else {
                    InputMode::Framebuffer
                });
                node.data.video_file_path = Some("/retained.mp4".into());
                node.data.raw_data_url = Some("retained-data".into());
                project
                    .replace_graph(graph, project.graph_revision())
                    .unwrap();
            }
            let mut controller = controller_for_uniforms(project);
            let before = controller.project.graph_snapshot();
            let revision = controller.project.graph_revision();
            controller.inspector_draft.as_mut().unwrap().label = "unapplied name".into();
            for (key, value) in changes {
                controller.change_form_draft(key, value);
            }
            assert!(controller.inspector_is_dirty());
            controller.apply_inspector(InspectorChange::Form(kind));
            let after = controller.project.graph_snapshot();
            assert_eq!(controller.project.graph_revision(), revision + 1);
            assert_eq!(controller.undo_edits.len(), 1);
            assert!(matches!(controller.undo_edits[0], super::EditKind::Content));
            assert_eq!(
                controller.inspector_draft.as_ref().unwrap().label,
                "unapplied name"
            );
            assert_eq!(after.nodes[0].data.label, before.nodes[0].data.label);
            assert_eq!(after.nodes[0].data.extra, before.nodes[0].data.extra);
            assert_eq!(
                after.nodes[0].data.raw_data_url,
                before.nodes[0].data.raw_data_url
            );
            assert_eq!(
                after.nodes[0].data.video_file_path,
                before.nodes[0].data.video_file_path
            );
            match kind {
                FormKind::Sampling => {
                    assert_eq!(after.nodes[0].data.tex_filter, Some(TextureFilter::Nearest))
                }
                FormKind::Framebuffer => assert_eq!(
                    (
                        after.nodes[0].data.fb_width,
                        after.nodes[0].data.fb_height,
                        after.nodes[0].data.fb_stride
                    ),
                    (Some(32), Some(16), Some(256))
                ),
                FormKind::Video => assert_eq!(after.nodes[0].data.video_playback_rate, Some(2.5)),
            }
            controller.apply_inspector(InspectorChange::Form(kind));
            assert_eq!(controller.project.graph_revision(), revision + 1);
            let sdk = OpenQuartz::new(Environment::headless());
            assert_eq!(
                sdk.open_project_json(
                    &serde_json::to_string(&controller.project.to_file()).unwrap()
                )
                .unwrap()
                .graph_snapshot(),
                after
            );
            let project = std::sync::Arc::make_mut(&mut controller.project);
            project.rollback_graph(project.graph_revision()).unwrap();
            assert_eq!(project.graph_snapshot(), before);
            project.redo_graph(project.graph_revision()).unwrap();
            assert_eq!(project.graph_snapshot(), after);
        }
    }

    #[test]
    fn invalid_form_and_busy_edits_preserve_project_and_drafts() {
        let mut controller = controller_for_uniforms(uniform_project(&[]));
        let before = controller.project.graph_snapshot();
        let revision = controller.project.graph_revision();
        controller.change_form_draft("filter", "99");
        assert!(controller.edits_blocked());
        controller.apply_inspector(InspectorChange::Form(
            crate::inspector_forms::FormKind::Sampling,
        ));
        assert_eq!(controller.project.graph_snapshot(), before);
        assert_eq!(controller.project.graph_revision(), revision);
        assert!(controller.undo_edits.is_empty());
        controller.busy = true;
        controller.change_form_draft("filter", "1");
        assert_eq!(
            controller.inspector_draft.as_ref().unwrap().forms.filter,
            "99"
        );
        controller.busy = false;
        controller.change_form_draft("filter", "0");
        assert!(!controller.inspector_is_dirty());
        controller.change_form_draft("video-rate", "2");
        assert!(!controller.inspector_is_dirty());
    }

    #[test]
    fn uniform_draft_survives_label_and_output_apply_then_undo_redo() {
        let project = uniform_project(&[("gain", DataType::Float)]);
        let mut controller = controller_for_uniforms(project);
        controller.change_uniform_draft("gain", "2.5");
        {
            let draft = controller.inspector_draft.as_mut().unwrap();
            draft.label = "Committed name".into();
            draft.output_width = 128;
            draft.output_height = 64;
        }
        controller.apply_inspector(InspectorChange::Label);
        assert_eq!(
            controller.inspector_draft.as_ref().unwrap().uniforms[0].literal,
            "2.5"
        );
        controller.apply_inspector(InspectorChange::OutputSize);
        assert_eq!(
            controller.inspector_draft.as_ref().unwrap().uniforms[0].literal,
            "2.5"
        );
        assert!(!controller.project.graph().nodes()[0]
            .data
            .uniforms
            .contains_key("gain"));
        let before = controller.project.graph_snapshot();
        controller.apply_inspector(InspectorChange::Uniforms);
        let after = controller.project.graph_snapshot();
        assert_eq!(after.nodes[0].data.label, "Committed name");
        assert_eq!(after.nodes[0].data.width, Some(128));
        assert_eq!(after.nodes[0].data.height, Some(64));
        assert!(!controller.inspector_is_dirty());
        controller.restore_edit(false);
        assert_eq!(controller.project.graph_snapshot(), before);
        assert_eq!(controller.redo_edits.len(), 1);
        assert!(matches!(controller.redo_edits[0], super::EditKind::Content));
        controller.inspector_draft = Some(InspectorDraft::from_node(
            &controller.project.graph().nodes()[0],
        ));
        let revision = controller.project.graph_revision();
        controller.apply_inspector(InspectorChange::Uniforms);
        assert_eq!(controller.project.graph_revision(), revision);
        assert_eq!(controller.redo_edits.len(), 1);
        controller.restore_edit(true);
        assert_eq!(controller.project.graph_snapshot(), after);
    }

    fn uniform_project(types: &[(&str, DataType)]) -> Project {
        let sdk = OpenQuartz::new(Environment::headless());
        let mut project = sdk.create_project("Uniforms");
        let (node, _) = project
            .create_graph_node(
                NodeFactoryRequest::Shader {
                    position: None,
                    code: "@fragment fn main() -> @location(0) vec4f { return vec4f(1.0); }".into(),
                    label: "Uniforms".into(),
                    template_name: None,
                    shader_template_id: None,
                },
                project.graph_revision(),
            )
            .unwrap();
        let mut data = node.data.clone();
        data.inputs = types
            .iter()
            .map(|(name, data_type)| Port {
                id: format!("{}_{}", node.id, name),
                label: (*name).into(),
                data_type: *data_type,
                direction: PortDirection::Input,
                default_value: None,
                description: None,
            })
            .collect();
        data.width = Some(96);
        data.height = Some(48);
        data.auto_size = Some(false);
        data.extra
            .insert("retained".into(), serde_json::json!({"nested": [1, 2]}));
        data.uniforms
            .insert("stale_key".into(), serde_json::json!([7, 8]));
        project
            .apply_graph_command(
                GraphCommand::UpdateNodeData {
                    node_id: NodeId::new(&node.id),
                    data,
                },
                project.graph_revision(),
            )
            .unwrap();
        project
    }

    fn set_uniform(draft: &mut InspectorDraft, name: &str, literal: &str) {
        draft
            .uniforms
            .iter_mut()
            .find(|uniform| uniform.name == name)
            .unwrap()
            .literal = literal.into();
    }

    fn controller_for_uniforms(project: Project) -> super::AppController {
        let node_id = project.graph().nodes()[0].id.clone();
        let draft = InspectorDraft::from_node(&project.graph().nodes()[0]);
        super::AppController {
            window: slint::Weak::default(),
            project: std::sync::Arc::new(project),
            renderer: None,
            onnx_states: std::collections::BTreeMap::new(),
            selected_node_ids: [node_id].into_iter().collect(),
            inspector_draft: Some(draft),
            inspector_uniforms: std::rc::Rc::new(slint::VecModel::from(Vec::<
                crate::InspectorUniform,
            >::new())),
            inspector_forms: std::rc::Rc::new(slint::VecModel::default()),
            clipboard: None,
            graph_stress_count: None,
            selected_edge_id: None,
            pending_connection: None,
            undo_edits: Vec::new(),
            redo_edits: Vec::new(),
            files: super::ProjectIo::new().unwrap(),
            busy: false,
            timer: slint::Timer::default(),
        }
    }

    #[test]
    fn uniform_parser_covers_supported_families_bounds_and_column_major_order() {
        use serde_json::json;
        for (data_type, literal, expected) in [
            (DataType::Float, " -1.25e2 ", json!(-125.0)),
            (DataType::Int, "-2147483648", json!(i32::MIN)),
            (DataType::Int, "2147483647", json!(i32::MAX)),
            (DataType::Uint, "4294967295", json!(u32::MAX)),
            (DataType::Bool, "1", json!(true)),
            (DataType::Bool, "0", json!(false)),
            (DataType::Bool, "true", json!(true)),
            (DataType::Bool, "false", json!(false)),
            (DataType::Vec2, "1, 2", json!([1.0, 2.0])),
            (DataType::Vec3, "1,2,3", json!([1.0, 2.0, 3.0])),
            (DataType::Vec4, "1,2,3,4", json!([1.0, 2.0, 3.0, 4.0])),
            (DataType::Ivec2, "-1,2", json!([-1, 2])),
            (DataType::Ivec3, "-1,2,3", json!([-1, 2, 3])),
            (DataType::Ivec4, "-1,2,3,4", json!([-1, 2, 3, 4])),
            (DataType::Uvec2, "0,2", json!([0, 2])),
            (DataType::Uvec3, "0,2,3", json!([0, 2, 3])),
            (DataType::Uvec4, "0,2,3,4", json!([0, 2, 3, 4])),
            (DataType::Mat2, "1,2,3,4", json!([1.0, 2.0, 3.0, 4.0])),
            (
                DataType::Mat3,
                "1,2,3,4,5,6,7,8,9",
                json!([1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]),
            ),
            (
                DataType::Mat4,
                "1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16",
                json!([
                    1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0,
                    15.0, 16.0
                ]),
            ),
        ] {
            assert_eq!(
                super::parse_uniform_literal(data_type, literal).unwrap(),
                expected,
                "{data_type:?}: {literal}"
            );
        }
    }

    #[test]
    fn uniform_parser_rejects_invalid_tokens_ranges_and_exact_arity() {
        for (data_type, literal) in [
            (DataType::Float, "NaN"),
            (DataType::Float, "inf"),
            (DataType::Float, "-inf"),
            (DataType::Float, "1e400"),
            (DataType::Float, "1e39"),
            (DataType::Float, ""),
            (DataType::Float, "1 2"),
            (DataType::Float, "1,"),
            (DataType::Int, "2147483648"),
            (DataType::Int, "-2147483649"),
            (DataType::Int, "1.5"),
            (DataType::Int, "1.0"),
            (DataType::Uint, "-1"),
            (DataType::Uint, "-0"),
            (DataType::Uint, "4294967296"),
            (DataType::Uint, "0.5"),
            (DataType::Bool, "yes"),
            (DataType::Bool, "2"),
            (DataType::Bool, "TRUE"),
            (DataType::Vec2, "1"),
            (DataType::Vec2, "1,,2"),
            (DataType::Vec2, "1,2,"),
            (DataType::Vec2, "1,NaN"),
            (DataType::Vec3, "1,2"),
            (DataType::Vec4, "1,2,3,4,5"),
            (DataType::Mat2, "1,2"),
            (DataType::Mat3, "1,2,3"),
            (DataType::Mat4, "1,2,3,4"),
            (DataType::Ivec2, "1,2147483648"),
            (DataType::Ivec3, "1,2,1.5"),
            (DataType::Ivec4, "1,2,3,-2147483649"),
            (DataType::Uvec2, "1,-1"),
            (DataType::Uvec3, "1,2,4294967296"),
            (DataType::Uvec4, "1,2,3,0.5"),
        ] {
            assert!(
                super::parse_uniform_literal(data_type, literal).is_err(),
                "{data_type:?}: {literal}"
            );
        }
        for data_type in [
            DataType::Auto,
            DataType::Bvec2,
            DataType::Bvec3,
            DataType::Bvec4,
            DataType::Sampler2d,
            DataType::SamplerCube,
            DataType::Roi,
            DataType::Mesh,
            DataType::Json,
        ] {
            assert!(super::parse_uniform_literal(data_type, "0").is_err());
        }
    }

    #[test]
    fn uniform_missing_defaults_ignore_port_defaults_and_do_not_seed_zeros() {
        let mut project = uniform_project(&[
            ("gain", DataType::Float),
            ("offset", DataType::Vec2),
            ("enabled", DataType::Bool),
            ("matrix", DataType::Mat2),
        ]);
        let node = project.graph().nodes()[0].clone();
        let mut data = node.data.clone();
        data.inputs[0].default_value = Some(serde_json::json!(9.0));
        project
            .apply_graph_command(
                GraphCommand::UpdateNodeData {
                    node_id: NodeId::new(&node.id),
                    data,
                },
                project.graph_revision(),
            )
            .unwrap();
        let before = project.graph_snapshot();
        let revision = project.graph_revision();
        let mut draft = InspectorDraft::from_node(&project.graph().nodes()[0]);
        assert_eq!(
            draft
                .uniforms
                .iter()
                .map(|uniform| uniform.literal.as_str())
                .collect::<Vec<_>>(),
            ["0.0", "0.0, 0.0", "false", "0.0, 0.0, 0.0, 0.0"]
        );
        set_uniform(&mut draft, "gain", "5");
        set_uniform(&mut draft, "gain", "0");
        assert!(!super::uniform_drafts_dirty(
            &project,
            &project.graph().nodes()[0],
            &draft
        ));
        assert!(!commit_inspector(&mut project, &draft, InspectorChange::Uniforms).unwrap());
        assert_eq!(project.graph_revision(), revision);
        assert_eq!(project.graph_snapshot(), before);
        set_uniform(&mut draft, "gain", "2");
        assert!(commit_inspector(&mut project, &draft, InspectorChange::Uniforms).unwrap());
        let values = &project.graph().nodes()[0].data.uniforms;
        assert_eq!(values.len(), 2);
        assert_eq!(values["gain"], serde_json::json!(2.0));
        assert!(!values.contains_key("offset"));
        assert!(!values.contains_key("enabled"));
        assert!(!values.contains_key("matrix"));
    }

    #[test]
    fn uniform_apply_is_one_content_edit_preserving_other_drafts_history_and_persistence() {
        let project = uniform_project(&[
            ("gain", DataType::Float),
            ("offset", DataType::Vec2),
            ("enabled", DataType::Bool),
        ]);
        let before = project.graph_snapshot();
        let revision = project.graph_revision();
        let mut controller = controller_for_uniforms(project);
        let draft = controller.inspector_draft.as_mut().unwrap();
        draft.label = "unapplied name".into();
        draft.shader_code = "unapplied source".into();
        draft.output_width = 120;
        set_uniform(draft, "gain", "0.5");
        set_uniform(draft, "enabled", "1");
        controller.apply_inspector(InspectorChange::Uniforms);
        assert_eq!(controller.project.graph_revision(), revision + 1);
        assert_eq!(controller.undo_edits.len(), 1);
        assert!(matches!(controller.undo_edits[0], super::EditKind::Content));
        let mut expected = before.clone();
        expected.nodes[0]
            .data
            .uniforms
            .insert("gain".into(), serde_json::json!(0.5));
        expected.nodes[0]
            .data
            .uniforms
            .insert("enabled".into(), serde_json::json!(true));
        assert_eq!(controller.project.graph_snapshot(), expected);
        let draft = controller.inspector_draft.as_ref().unwrap();
        assert_eq!(draft.label, "unapplied name");
        assert_eq!(draft.shader_code, "unapplied source");
        assert_eq!(draft.output_width, 120);
        assert_eq!(
            draft
                .uniforms
                .iter()
                .find(|uniform| uniform.name == "enabled")
                .unwrap()
                .literal,
            "true"
        );
        assert_eq!(
            controller.inspector_dirty_fields(),
            (true, true, true, false)
        );
        controller.apply_inspector(InspectorChange::Uniforms);
        assert_eq!(controller.project.graph_revision(), revision + 1);
        assert_eq!(controller.undo_edits.len(), 1);
        let sdk = OpenQuartz::new(Environment::headless());
        let json = serde_json::to_string(&controller.project.to_file()).unwrap();
        assert_eq!(
            sdk.open_project_json(&json).unwrap().graph_snapshot(),
            expected
        );
        let project = std::sync::Arc::make_mut(&mut controller.project);
        project.rollback_graph(project.graph_revision()).unwrap();
        assert_eq!(project.graph_snapshot(), before);
        project.redo_graph(project.graph_revision()).unwrap();
        assert_eq!(project.graph_snapshot(), expected);
    }

    #[test]
    fn uniform_invalid_draft_preserves_project_and_other_drafts() {
        let project = uniform_project(&[("gain", DataType::Float), ("offset", DataType::Vec2)]);
        let before = project.graph_snapshot();
        let revision = project.graph_revision();
        let mut controller = controller_for_uniforms(project);
        controller.publish_uniforms();
        let model = controller.inspector_uniforms.clone();
        {
            let draft = controller.inspector_draft.as_mut().unwrap();
            draft.label = "name draft".into();
            draft.shader_code = "source draft".into();
            draft.output_height = 32;
        }
        controller.change_uniform_draft("gain", "NaN");
        assert!(!model.row_data(0).unwrap().valid);
        assert!(!model.row_data(0).unwrap().error.is_empty());
        assert!(model.row_data(1).unwrap().valid);
        assert!(controller.edits_blocked());
        controller.apply_inspector(InspectorChange::Uniforms);
        assert_eq!(controller.project.graph_snapshot(), before);
        assert_eq!(controller.project.graph_revision(), revision);
        assert!(controller.undo_edits.is_empty());
        let draft = controller.inspector_draft.as_ref().unwrap();
        assert_eq!(draft.label, "name draft");
        assert_eq!(draft.shader_code, "source draft");
        assert_eq!(draft.output_height, 32);
        assert_eq!(draft.uniforms[0].literal, "NaN");
        controller.apply_inspector(InspectorChange::Label);
        assert_eq!(
            controller.inspector_draft.as_ref().unwrap().uniforms[0].literal,
            "NaN"
        );
        controller.change_uniform_draft("gain", "2");
        assert!(model.row_data(0).unwrap().valid);
        controller.busy = true;
        controller.change_uniform_draft("gain", "3");
        assert_eq!(
            controller.inspector_draft.as_ref().unwrap().uniforms[0].literal,
            "2"
        );
    }

    #[test]
    fn uniform_connected_inputs_are_read_only_excluded_and_shader_only() {
        let mut project = uniform_project(&[
            ("gain", DataType::Float),
            ("offset", DataType::Vec2),
            ("texture", DataType::Sampler2d),
            ("dynamic", DataType::Auto),
            ("flags", DataType::Bvec2),
            ("json", DataType::Json),
        ]);
        let (source, _) = project
            .create_graph_node(
                NodeFactoryRequest::Math {
                    position: None,
                    op: "add".into(),
                },
                project.graph_revision(),
            )
            .unwrap();
        let target = &project.graph().nodes()[0];
        let target_key = PortKey::new(
            NodeId::new(&target.id),
            PortId::new(&target.data.inputs[0].id),
        );
        project
            .apply_graph_command(
                GraphCommand::Connect {
                    source: PortKey::new(
                        NodeId::new(&source.id),
                        PortId::new(&source.data.outputs[0].id),
                    ),
                    target: target_key,
                },
                project.graph_revision(),
            )
            .unwrap();
        let mut controller = controller_for_uniforms(project);
        set_uniform(controller.inspector_draft.as_mut().unwrap(), "gain", "NaN");
        controller.publish_uniforms();
        assert_eq!(controller.inspector_uniforms.row_count(), 2);
        let row = controller.inspector_uniforms.row_data(0).unwrap();
        assert!(row.connected && row.valid && row.error.is_empty());
        assert!(!controller.inspector_is_dirty());
        controller.change_uniform_draft("gain", "12");
        assert_eq!(
            controller.inspector_draft.as_ref().unwrap().uniforms[0].literal,
            "NaN"
        );
        controller.change_uniform_draft("offset", "1,2");
        controller.apply_inspector(InspectorChange::Uniforms);
        assert!(!controller.project.graph().nodes()[0]
            .data
            .uniforms
            .contains_key("gain"));
        assert_eq!(
            controller.project.graph().nodes()[0].data.uniforms["offset"],
            serde_json::json!([1.0, 2.0])
        );
        let source = controller.project.graph().nodes()[1].clone();
        assert!(InspectorDraft::from_node(&source).uniforms.is_empty());
        let revision = controller.project.graph_revision();
        let (constant, _) = std::sync::Arc::make_mut(&mut controller.project)
            .create_graph_node(NodeFactoryRequest::Constant { position: None }, revision)
            .unwrap();
        let draft = InspectorDraft::from_node(&constant);
        assert!(draft.uniforms.is_empty());
        assert_eq!(
            commit_inspector(
                std::sync::Arc::make_mut(&mut controller.project),
                &draft,
                InspectorChange::Uniforms
            )
            .unwrap_err()
            .code,
            SdkErrorCode::InvalidState
        );
        assert_eq!(source.node_type, NodeType::Math);
    }

    #[test]
    fn uniform_shader_schema_refresh_retains_matching_drafts_and_drops_obsolete_rows() {
        let mut project = uniform_project(&[
            ("gain", DataType::Float),
            ("offset", DataType::Vec2),
            ("removed", DataType::Float),
        ]);
        let mut draft = InspectorDraft::from_node(&project.graph().nodes()[0]);
        set_uniform(&mut draft, "gain", "2.5");
        set_uniform(&mut draft, "offset", "NaN,0");
        set_uniform(&mut draft, "removed", "NaN");
        draft.label = "keep label".into();
        draft.output_width = 120;
        draft.shader_code = "@group(0) @binding(0) var<uniform> gain: f32;
@group(0) @binding(1) var<uniform> offset: vec3f;
@group(0) @binding(2) var<uniform> added: i32;
@fragment fn main() -> @location(0) vec4f { return vec4f(gain); }"
            .into();
        commit_inspector(&mut project, &draft, InspectorChange::Shader).unwrap();
        draft.refresh_uniforms(&project.graph().nodes()[0], true);
        assert_eq!(draft.uniforms.len(), 3);
        assert_eq!(draft.uniforms[0].literal, "2.5");
        assert_eq!(draft.uniforms[1].literal, "0.0, 0.0, 0.0");
        assert_eq!(draft.uniforms[2].literal, "0");
        assert_eq!(draft.label, "keep label");
        assert_eq!(draft.output_width, 120);
        assert_eq!(
            project.graph().nodes()[0].data.uniforms["stale_key"],
            serde_json::json!([7, 8])
        );
        assert!(!project.graph().nodes()[0]
            .data
            .uniforms
            .contains_key("gain"));
        commit_inspector(&mut project, &draft, InspectorChange::Uniforms).unwrap();
        assert_eq!(
            project.graph().nodes()[0].data.uniforms["gain"],
            serde_json::json!(2.5)
        );
    }

    #[test]
    fn uniform_untouched_legacy_value_shows_error_without_dirty_lock() {
        let mut project = uniform_project(&[("gain", DataType::Float)]);
        let node = project.graph().nodes()[0].clone();
        let mut data = node.data.clone();
        data.uniforms
            .insert("gain".into(), serde_json::json!("legacy invalid"));
        project
            .apply_graph_command(
                GraphCommand::UpdateNodeData {
                    node_id: NodeId::new(&node.id),
                    data,
                },
                project.graph_revision(),
            )
            .unwrap();
        let mut controller = controller_for_uniforms(project);
        controller.publish_uniforms();
        assert!(!controller.inspector_uniforms.row_data(0).unwrap().valid);
        assert!(!controller.inspector_is_dirty());
        controller.change_uniform_draft("gain", "still invalid");
        assert!(controller.inspector_is_dirty());
        controller.change_uniform_draft("gain", "legacy invalid");
        assert!(!controller.inspector_is_dirty());
    }

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
