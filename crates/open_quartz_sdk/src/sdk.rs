use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use serde::{Deserialize, Serialize};

use open_quartz_schema::{SdkError, SdkErrorCode};
use open_quartz_execution::event::EngineState;
use open_quartz_execution::runtime::{
    DeliveryPolicy, OutputKey, OutputSubscription, OutputTransport, Runtime, RuntimeCapabilities,
    RuntimeFrameInput,
};
use open_quartz_schema::{
    DataType, Edge, FramebufferFormat, Graph, GraphChange, GraphCommand, InputMode, NodeData,
    NodeFactoryRequest, NodeId, NodeType, OnnxSource, Port, PortDirection, Position, ProjectFile,
    ProjectNode, ResourceId, SystemSource, PROJECT_FILE_VERSION,
};
use serde_json::Value;

const SCREEN_SAVER_RESAMPLE_NODE_ID: &str = "__screen_saver_output_resample";
const SCREEN_SAVER_SOURCE_EDGE_ID: &str = "__screen_saver_source_edge";
const SCREEN_SAVER_RENDERER_EDGE_ID: &str = "__screen_saver_renderer_edge";
const SCREEN_SAVER_RESAMPLE_SHADER: &str = r#"
@group(0) @binding(0) var inputImage: texture_2d<f32>;
@group(0) @binding(1) var inputImageSampler: sampler;

@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  return textureSample(inputImage, inputImageSampler, v_uv);
}"#;

#[derive(Clone, Debug)]
pub struct Environment {
    capabilities: RuntimeCapabilities,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            capabilities: RuntimeCapabilities { data_paths: vec![] },
        }
    }
}

impl Environment {
    pub fn headless() -> Self {
        Self::default()
    }

    pub(crate) fn capabilities(&self) -> RuntimeCapabilities {
        self.capabilities.clone()
    }
}

#[derive(Clone, Debug)]
pub struct OpenQuartz {
    environment: Environment,
}

impl OpenQuartz {
    pub fn new(environment: Environment) -> Self {
        Self { environment }
    }

    pub fn create_project(&self, name: impl Into<String>) -> Project {
        Project {
            version: PROJECT_FILE_VERSION.to_owned(),
            name: name.into(),
            created_at: String::new(),
            updated_at: String::new(),
            graph: Graph::default(),
            graph_revision: 0,
            graph_history: Vec::new(),
            graph_redo: Vec::new(),
            layout: GraphLayout::default(),
            resources: ResourceCatalog::default(),
        }
    }

    pub fn open_project_json(&self, project_json: &str) -> Result<Project, SdkError> {
        let file: ProjectFile = serde_json::from_str(project_json)
            .map_err(|error| {
                SdkError::new(SdkErrorCode::InvalidState, "Cannot decode project")
                    .with_details(error.to_string())
            })?;
        Project::try_from_file(file)
    }

    pub fn normalize_project_json(&self, project_json: &str) -> Result<String, SdkError> {
        let project = self.open_project_json(project_json)?;
        serde_json::to_string(&project.to_file()).map_err(|error| {
            SdkError::new(SdkErrorCode::InvalidState, "Cannot encode project")
                .with_details(error.to_string())
        })
    }

    pub fn screen_saver_export_project_json(
        &self,
        project_json: &str,
        renderer_node_id: &str,
    ) -> Result<String, SdkError> {
        let mut project = self.open_project_json(project_json)?;
        project.graph = project
            .graph
            .upstream_subgraph(&NodeId::new(renderer_node_id))?;
        serde_json::to_string(&project.to_file()).map_err(|error| {
            SdkError::new(
                SdkErrorCode::InvalidState,
                "Cannot encode screen saver project",
            )
            .with_details(error.to_string())
        })
    }

    pub fn player(&self, graph: &Graph) -> PlayerBuilder {
        PlayerBuilder {
            environment: self.environment.clone(),
            graph: graph.clone(),
            resources: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Project {
    version: String,
    name: String,
    created_at: String,
    updated_at: String,
    graph: Graph,
    graph_revision: u32,
    graph_history: Vec<Graph>,
    graph_redo: Vec<Graph>,
    layout: GraphLayout,
    resources: ResourceCatalog,
}

impl Project {
    pub fn try_from_file(file: ProjectFile) -> Result<Self, SdkError> {
        if file.version != PROJECT_FILE_VERSION {
            return Err(SdkError::new(
                SdkErrorCode::InvalidState,
                "Incompatible project version",
            )
            .with_details(format!(
                "expected {PROJECT_FILE_VERSION}, got {}",
                file.version
            )));
        }
        file.graph.validate()?;
        Ok(Self::from_file(file))
    }

    pub fn from_file(file: ProjectFile) -> Self {
        let positions = file
            .graph
            .nodes
            .iter()
            .map(|node| (NodeId::new(&node.id), node.position))
            .collect();
        Self {
            version: file.version,
            name: file.name,
            created_at: file.created_at,
            updated_at: file.updated_at,
            graph: file.graph,
            graph_revision: 0,
            graph_history: Vec::new(),
            graph_redo: Vec::new(),
            layout: GraphLayout { positions },
            resources: ResourceCatalog::default(),
        }
    }

    pub fn to_file(&self) -> ProjectFile {
        let mut graph = self.graph.clone();
        for node in &mut graph.nodes {
            if let Some(position) = self.layout.position(&NodeId::new(&node.id)) {
                node.position = position;
            }
        }
        ProjectFile {
            version: self.version.clone(),
            name: self.name.clone(),
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
            graph,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
    }

    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    pub fn graph_revision(&self) -> u32 {
        self.graph_revision
    }

    pub fn graph_snapshot(&self) -> Graph {
        self.graph.clone()
    }

    pub fn screen_saver_graph(
        &self,
        renderer_node_id: &str,
        width: u32,
        height: u32,
    ) -> Result<Graph, SdkError> {
        let mut graph = self
            .graph
            .upstream_subgraph(&NodeId::new(renderer_node_id))?;
        graph.insert_screen_saver_resample(renderer_node_id, width, height)?;
        graph.validate()?;
        Ok(graph)
    }

    pub fn replace_graph(
        &mut self,
        graph: Graph,
        expected_revision: u32,
    ) -> Result<GraphChange, SdkError> {
        self.ensure_graph_revision(expected_revision)?;
        graph.validate()?;
        let previous = self.graph.clone();
        let changed_nodes = changed_node_ids(&self.graph, &graph);
        self.graph_history.push(self.graph.clone());
        self.graph = graph;
        self.graph_redo.clear();
        self.reconcile_layout(&previous);
        self.graph_revision = self.graph_revision.saturating_add(1);
        Ok(GraphChange::from_changed_nodes(changed_nodes))
    }

    /// Apply one typed graph command through the revisioned aggregate.
    pub fn apply_graph_command(
        &mut self,
        command: GraphCommand,
        expected_revision: u32,
    ) -> Result<GraphChange, SdkError> {
        self.ensure_graph_revision(expected_revision)?;
        let previous = self.graph.clone();
        let mut next = previous.clone();
        let change = match command {
            GraphCommand::UpdateShaderCode {
                node_id,
                shader_code,
            } => {
                let node = next
                    .nodes()
                    .iter()
                    .find(|node| node.id == node_id.as_str())
                    .ok_or_else(|| {
                        SdkError::new(SdkErrorCode::UnknownNode, "Graph node does not exist")
                            .for_node(node_id.as_str())
                    })?
                    .clone();
                let parsed = open_quartz_execution::wgsl::parse_shader(&shader_code);
                let inputs = remap_ports_preserving_ids(&node.data.inputs, &node.id, parsed.inputs);
                let outputs = remap_ports_preserving_ids(&node.data.outputs, &node.id, parsed.outputs);
                let mut data = node.data;
                data.shader_code = shader_code;
                data.inputs = inputs;
                data.outputs = outputs;
                if let Some(error) = parsed.parse_error {
                    data.extra
                        .insert("parseError".to_owned(), Value::String(error));
                } else {
                    data.extra.remove("parseError");
                }
                next.apply_command(GraphCommand::UpdateNodeData { node_id, data })?
            }
            GraphCommand::UpdateInputType {
                node_id,
                data_type,
                input_mode,
            } => {
                let node = next
                    .nodes()
                    .iter()
                    .find(|node| node.id == node_id.as_str())
                    .ok_or_else(|| {
                        SdkError::new(SdkErrorCode::UnknownNode, "Graph node does not exist")
                            .for_node(node_id.as_str())
                    })?
                    .clone();
                if node.node_type != NodeType::Input {
                    return Err(SdkError::new(
                        SdkErrorCode::InvalidGraph,
                        "Only input nodes support input type updates",
                    )
                    .for_node(node_id.as_str()));
                }
                let shader_code = input_shader_code(data_type);
                let parsed = open_quartz_execution::wgsl::parse_shader(&shader_code);
                let inputs = remap_ports_preserving_ids(&node.data.inputs, &node.id, parsed.inputs);
                let outputs = remap_ports_preserving_ids(&node.data.outputs, &node.id, parsed.outputs)
                    .into_iter()
                    .map(|mut port| {
                        port.data_type = data_type;
                        port
                    })
                    .collect();
                let mut data = node.data;
                data.shader_code = shader_code;
                data.input_data_type = Some(data_type);
                data.input_mode = if data_type == DataType::Sampler2d {
                    Some(input_mode.unwrap_or(InputMode::Image))
                } else {
                    input_mode
                };
                data.uniforms = serde_json::Map::new();
                data.inputs = inputs;
                data.outputs = outputs;
                next.apply_command(GraphCommand::UpdateNodeData { node_id, data })?
            }
            other => next.apply_command(other)?,
        };
        self.graph_history.push(previous.clone());
        self.graph = next;
        self.graph_redo.clear();
        self.reconcile_layout(&previous);
        self.graph_revision = self.graph_revision.saturating_add(1);
        Ok(change)
    }

    pub fn initialize_graph(&mut self, graph: Graph) -> Result<(), SdkError> {
        if self.graph_revision != 0
            || !self.graph.nodes().is_empty()
            || !self.graph_history.is_empty()
        {
            return Err(SdkError::new(
                SdkErrorCode::InvalidState,
                "Graph has already been initialized",
            ));
        }
        graph.validate()?;
        self.graph = graph;
        self.graph_history.clear();
        self.graph_redo.clear();
        self.reconcile_layout(&Graph::default());
        Ok(())
    }

    pub fn rollback_graph(&mut self, expected_revision: u32) -> Result<GraphChange, SdkError> {
        self.ensure_graph_revision(expected_revision)?;
        let previous = self.graph_history.pop().ok_or_else(|| {
            SdkError::new(SdkErrorCode::InvalidState, "Graph has no revision to roll back")
        })?;
        let changed_nodes = changed_node_ids(&self.graph, &previous);
        let current = self.graph.clone();
        self.graph_redo.push(current.clone());
        self.graph = previous;
        self.reconcile_layout(&current);
        self.graph_revision = self.graph_revision.saturating_add(1);
        Ok(GraphChange::from_changed_nodes(changed_nodes))
    }

    pub fn redo_graph(&mut self, expected_revision: u32) -> Result<GraphChange, SdkError> {
        self.ensure_graph_revision(expected_revision)?;
        let next = self.graph_redo.pop().ok_or_else(|| {
            SdkError::new(SdkErrorCode::InvalidState, "Graph has no revision to reapply")
        })?;
        let previous = self.graph.clone();
        let changed_nodes = changed_node_ids(&previous, &next);
        self.graph_history.push(previous.clone());
        self.graph = next;
        self.reconcile_layout(&previous);
        self.graph_revision = self.graph_revision.saturating_add(1);
        Ok(GraphChange::from_changed_nodes(changed_nodes))
    }

    /// Build a node with Rust-owned executable defaults and add it atomically.
    pub fn create_graph_node(
        &mut self,
        request: NodeFactoryRequest,
        expected_revision: u32,
    ) -> Result<(ProjectNode, GraphChange), SdkError> {
        self.ensure_graph_revision(expected_revision)?;
        let node = self.build_graph_node(request)?;
        let change = self.apply_graph_command(
            GraphCommand::AddNode { node: node.clone() },
            expected_revision,
        )?;
        Ok((node, change))
    }

    pub fn graph_mut(&mut self) -> &mut Graph {
        &mut self.graph
    }

    pub fn layout(&self) -> &GraphLayout {
        &self.layout
    }

    pub fn layout_mut(&mut self) -> &mut GraphLayout {
        &mut self.layout
    }

    pub fn resources(&self) -> &ResourceCatalog {
        &self.resources
    }

    pub fn resources_mut(&mut self) -> &mut ResourceCatalog {
        &mut self.resources
    }

    fn ensure_graph_revision(&self, expected_revision: u32) -> Result<(), SdkError> {
        if expected_revision == self.graph_revision {
            return Ok(());
        }
        Err(SdkError::new(
            SdkErrorCode::StaleRevision,
            "Graph revision is stale",
        )
        .with_details(format!(
            "expected revision {expected_revision}, current revision {}",
            self.graph_revision
        )))
    }

    fn build_graph_node(&self, request: NodeFactoryRequest) -> Result<ProjectNode, SdkError> {
        let (node_type, position, label_base) = match &request {
            NodeFactoryRequest::Shader {
                position, label, ..
            } => (NodeType::Shader, position, label.as_str()),
            NodeFactoryRequest::Input { position, .. } => (NodeType::Input, position, "input"),
            NodeFactoryRequest::System { position, source } => {
                (NodeType::Input, position, system_label(*source))
            }
            NodeFactoryRequest::Constant { position } => (NodeType::Constant, position, "constant"),
            NodeFactoryRequest::Math { position, op } => {
                (NodeType::Math, position, math_label(op).unwrap_or("math"))
            }
            NodeFactoryRequest::Renderer { position } => {
                (NodeType::Renderer, position, "renderer")
            }
            NodeFactoryRequest::Onnx {
                position, label, ..
            } => (NodeType::Onnx, position, label.as_str()),
            NodeFactoryRequest::CustomOnnx { position } => {
                (NodeType::Onnx, position, "custom_onnx")
            }
        };
        let id = next_node_id(&self.graph, node_type, label_base);
        let position = position
            .as_ref()
            .copied()
            .unwrap_or_else(|| next_node_position(&self.graph));
        let instance_label = format!("{}_{}", normalize_label(label_base), numeric_suffix(&id));

        let mut data = NodeData {
            node_type,
            label: instance_label,
            ..NodeData::default()
        };
        data.shader_code = String::new();

        match request {
            NodeFactoryRequest::Shader {
                code,
                label,
                template_name,
                shader_template_id,
                ..
            } => {
                let parsed = open_quartz_execution::wgsl::parse_shader(&code);
                data.label = format!("{}_{}", normalize_label(&label), numeric_suffix(&id));
                data.template_name = template_name.or_else(|| Some(label));
                data.shader_template_id = shader_template_id;
                data.shader_code = code;
                data.inputs = remap_ports(&id, parsed.inputs);
                data.outputs = remap_ports(&id, parsed.outputs);
            }
            NodeFactoryRequest::Input {
                data_type,
                input_mode,
                ..
            } => {
                let code = input_shader_code(data_type);
                let parsed = open_quartz_execution::wgsl::parse_shader(&code);
                data.shader_code = code;
                data.input_data_type = Some(data_type);
                data.input_mode = Some(input_mode.unwrap_or(if data_type == DataType::Sampler2d {
                    InputMode::Image
                } else {
                    InputMode::System
                }));
                data.inputs = remap_ports(&id, parsed.inputs);
                data.outputs = remap_ports(&id, parsed.outputs)
                    .into_iter()
                    .map(|mut port| {
                        port.data_type = data_type;
                        port
                    })
                    .collect();
                if data_type != DataType::Sampler2d {
                    data.input_mode = input_mode;
                }
            }
            NodeFactoryRequest::System { source, .. } => {
                let data_type = system_data_type(source);
                let code = system_shader_code(source);
                let parsed = open_quartz_execution::wgsl::parse_shader(&code);
                data.shader_code = code;
                data.input_data_type = Some(data_type);
                data.input_mode = Some(InputMode::System);
                data.system_source = Some(source);
                data.inputs = remap_ports(&id, parsed.inputs);
                data.outputs = remap_ports(&id, parsed.outputs)
                    .into_iter()
                    .map(|mut port| {
                        port.data_type = data_type;
                        port
                    })
                    .collect();
            }
            NodeFactoryRequest::Constant { .. } => {
                let code = constant_shader_code();
                let parsed = open_quartz_execution::wgsl::parse_shader(&code);
                data.shader_code = code;
                data.inputs = remap_ports(&id, parsed.inputs);
                data.outputs = remap_ports(&id, parsed.outputs);
            }
            NodeFactoryRequest::Math { op, .. } => {
                let input_count = math_input_count(&op).ok_or_else(|| {
                    SdkError::new(SdkErrorCode::InvalidGraph, "Unknown math operation")
                        .with_details(op.clone())
                })?;
                data.label = format!("{}_{}", normalize_label(math_label(&op).unwrap()), numeric_suffix(&id));
                data.template_name = math_label(&op).map(str::to_owned);
                data.math_op = Some(op);
                data.inputs = math_ports(&id, input_count, PortDirection::Input);
                data.outputs = math_ports(&id, 1, PortDirection::Output)
                    .into_iter()
                    .map(|mut port| {
                        port.label = "result".to_owned();
                        port.id = format!("{id}_result");
                        port
                    })
                    .collect();
            }
            NodeFactoryRequest::Renderer { .. } => {
                data.extra.insert("expanded".to_owned(), Value::Bool(true));
                data.inputs = vec![Port {
                    id: format!("{id}_inputTexture"),
                    label: "inputTexture".to_owned(),
                    data_type: DataType::Sampler2d,
                    direction: PortDirection::Input,
                    default_value: None,
                    description: None,
                }];
            }
            NodeFactoryRequest::Onnx {
                label,
                template_name,
                model_id,
                catalog_id,
                inputs,
                outputs,
                ..
            } => {
                data.label = format!("{}_{}", normalize_label(&label), numeric_suffix(&id));
                data.template_name = template_name.or_else(|| Some(label));
                data.onnx_model_id = model_id;
                data.onnx_catalog_id = catalog_id;
                data.onnx_source = Some(OnnxSource::Catalog);
                data.extra.insert(
                    "onnxStatus".to_owned(),
                    Value::String("not-downloaded".to_owned()),
                );
                data.inputs = remap_ports(&id, inputs);
                data.outputs = remap_ports(&id, outputs);
            }
            NodeFactoryRequest::CustomOnnx { .. } => {
                data.label = "Custom ONNX".to_owned();
                data.onnx_source = Some(OnnxSource::Custom);
                data.inputs = Vec::new();
                data.outputs = Vec::new();
            }
        }

        Ok(ProjectNode {
            id,
            node_type,
            position,
            data,
        })
    }

    fn reconcile_layout(&mut self, previous: &Graph) {
        self.layout
            .positions
            .retain(|node_id, _| self.graph.nodes().iter().any(|node| {
                node.id == node_id.as_str()
            }));
        for node in self.graph.nodes() {
            let changed = previous
                .nodes()
                .iter()
                .find(|candidate| candidate.id == node.id)
                .map(|candidate| candidate.position != node.position)
                .unwrap_or(true);
            if changed || !self.layout.positions.contains_key(&NodeId::new(&node.id)) {
                self.layout
                    .positions
                    .insert(NodeId::new(&node.id), node.position);
            }
        }
    }
}

trait ScreenSaverGraphTransform {
    fn insert_screen_saver_resample(
        &mut self,
        renderer_node_id: &str,
        width: u32,
        height: u32,
    ) -> Result<(), SdkError>;
}

impl ScreenSaverGraphTransform for Graph {
    fn insert_screen_saver_resample(
        &mut self,
        renderer_node_id: &str,
        width: u32,
        height: u32,
    ) -> Result<(), SdkError> {
        if self
            .nodes()
            .iter()
            .any(|node| node.id == SCREEN_SAVER_RESAMPLE_NODE_ID)
        {
            return Err(SdkError::new(
                SdkErrorCode::InvalidGraph,
                "Screen saver resample node ID is already in use",
            )
            .for_node(SCREEN_SAVER_RESAMPLE_NODE_ID));
        }
        let renderer = self
            .nodes()
            .iter()
            .find(|node| node.id == renderer_node_id && node.node_type == NodeType::Renderer)
            .ok_or_else(|| {
                SdkError::new(
                    SdkErrorCode::UnknownNode,
                    "The selected Renderer is missing from the exported graph",
                )
                .for_node(renderer_node_id)
            })?
            .clone();
        let renderer_edge_index = self
            .edges()
            .iter()
            .position(|edge| edge.target == renderer_node_id)
            .ok_or_else(|| {
                SdkError::new(
                    SdkErrorCode::InvalidGraph,
                    "The selected Renderer has no connected input",
                )
                .for_node(renderer_node_id)
            })?;
        let renderer_edge = self.edges.remove(renderer_edge_index);
        let resample_node = screen_saver_resample_node(renderer.position, width, height);
        self.nodes.push(resample_node);
        self.edges.push(Edge {
            id: SCREEN_SAVER_SOURCE_EDGE_ID.to_owned(),
            source: renderer_edge.source.clone(),
            source_handle: renderer_edge.source_handle.clone(),
            target: SCREEN_SAVER_RESAMPLE_NODE_ID.to_owned(),
            target_handle: "inputImage".to_owned(),
        });
        self.edges.push(Edge {
            id: SCREEN_SAVER_RENDERER_EDGE_ID.to_owned(),
            source: SCREEN_SAVER_RESAMPLE_NODE_ID.to_owned(),
            source_handle: "fragColor".to_owned(),
            target: renderer_edge.target,
            target_handle: renderer_edge.target_handle,
        });
        Ok(())
    }
}

fn screen_saver_resample_node(position: Position, width: u32, height: u32) -> ProjectNode {
    ProjectNode {
        id: SCREEN_SAVER_RESAMPLE_NODE_ID.to_owned(),
        node_type: NodeType::Shader,
        position,
        data: NodeData {
            node_type: NodeType::Shader,
            label: "Screen Resolution".to_owned(),
            template_name: Some("Resample".to_owned()),
            shader_template_id: Some("Resample".to_owned()),
            shader_code: SCREEN_SAVER_RESAMPLE_SHADER.to_owned(),
            inputs: vec![Port {
                id: "inputImage".to_owned(),
                label: "inputImage".to_owned(),
                data_type: DataType::Sampler2d,
                direction: PortDirection::Input,
                default_value: None,
                description: None,
            }],
            outputs: vec![Port {
                id: "fragColor".to_owned(),
                label: "fragColor".to_owned(),
                data_type: DataType::Sampler2d,
                direction: PortDirection::Output,
                default_value: None,
                description: None,
            }],
            uniforms: serde_json::Map::new(),
            input_data_type: None,
            input_mode: None,
            image_data_url: None,
            image_width: None,
            image_height: None,
            fb_format: None,
            fb_width: None,
            fb_height: None,
            fb_stride: None,
            raw_data_url: None,
            tex_filter: None,
            tex_wrap: None,
            width: Some(width),
            height: Some(height),
            auto_size: Some(false),
            out_format: Some(FramebufferFormat::Rgba8),
            onnx_model_id: None,
            onnx_score_threshold: None,
            onnx_iou_threshold: None,
            onnx_target_size: None,
            onnx_source: None,
            onnx_catalog_id: None,
            onnx_custom_path: None,
            onnx_params: None,
            video_source_type: None,
            video_url: None,
            video_file_path: None,
            video_device_id: None,
            video_loop: None,
            video_playback_rate: None,
            system_source: None,
            math_op: None,
            feedback_enabled: None,
            feedback_clear_color: None,
            extra: serde_json::Map::new(),
        },
    }
}

fn changed_node_ids(before: &Graph, after: &Graph) -> Vec<NodeId> {
    let mut changed = std::collections::BTreeSet::new();
    let node_ids = before
        .nodes()
        .iter()
        .chain(after.nodes())
        .map(|node| node.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for node_id in node_ids {
        let previous = before.nodes().iter().find(|node| node.id == node_id);
        let current = after.nodes().iter().find(|node| node.id == node_id);
        if previous != current {
            changed.insert(node_id.to_owned());
        }
    }
    for edge in before.edges() {
        if !after.edges().contains(edge) {
            changed.insert(edge.source.clone());
            changed.insert(edge.target.clone());
        }
    }
    for edge in after.edges() {
        if !before.edges().contains(edge) {
            changed.insert(edge.source.clone());
            changed.insert(edge.target.clone());
        }
    }
    changed.into_iter().map(NodeId::new).collect()
}

fn next_node_id(graph: &Graph, node_type: NodeType, label_base: &str) -> String {
    let prefix = match node_type {
        NodeType::Shader => "shader",
        NodeType::Input => "input",
        NodeType::Constant => "constant",
        NodeType::Onnx => {
            if label_base == "custom_onnx" {
                "onnx"
            } else {
                "onnx"
            }
        }
        NodeType::Renderer => "renderer",
        NodeType::Math => "math",
    };
    let mut next = 1u32;
    for node in graph.nodes() {
        let Some((node_prefix, suffix)) = node.id.rsplit_once('_') else {
            continue;
        };
        if node_prefix == prefix {
            if let Ok(number) = suffix.parse::<u32>() {
                next = next.max(number.saturating_add(1));
            }
        }
    }
    format!("{prefix}_{next}")
}

fn numeric_suffix(id: &str) -> &str {
    id.rsplit_once('_').map(|(_, suffix)| suffix).unwrap_or("1")
}

fn normalize_label(value: &str) -> String {
    value
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("_")
}

fn next_node_position(graph: &Graph) -> Position {
    let offset = graph.nodes().len() as f64 * 28.0;
    Position {
        x: 100.0 + offset,
        y: 100.0 + offset,
    }
}

fn remap_ports(node_id: &str, ports: Vec<Port>) -> Vec<Port> {
    ports
        .into_iter()
        .map(|mut port| {
            port.id = format!("{node_id}_{}", port.label);
            port
        })
        .collect()
}

fn remap_ports_preserving_ids(existing: &[Port], node_id: &str, ports: Vec<Port>) -> Vec<Port> {
    ports
        .into_iter()
        .map(|mut port| {
            if let Some(previous) = existing.iter().find(|candidate| candidate.label == port.label) {
                port.id = previous.id.clone();
            } else {
                port.id = format!("{node_id}_{}", port.label);
            }
            port
        })
        .collect()
}

fn input_shader_code(data_type: DataType) -> String {
    if data_type == DataType::Sampler2d {
        "@group(0) @binding(0) var value: texture_2d<f32>;\n\
         @group(0) @binding(1) var valueSampler: sampler;\n\
         @fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f { \
           return textureSample(value, valueSampler, v_uv); \
         }"
            .replace("         ", "")
    } else {
        "@group(0) @binding(0) var<uniform> value: f32;\n\
         @fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f { \
           return vec4f(value, 0.0, 0.0, 1.0); \
         }"
            .replace("         ", "")
    }
}

fn constant_shader_code() -> String {
    "@group(0) @binding(0) var<uniform> color: vec4f;\n\
     @fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f { return color; }"
        .replace("     ", "")
}

fn system_label(source: SystemSource) -> &'static str {
    match source {
        SystemSource::Time => "time",
        SystemSource::TimeDelta => "time_delta",
        SystemSource::Frame => "frame",
        SystemSource::Mouse => "mouse",
        SystemSource::Resolution => "resolution",
    }
}

fn system_data_type(source: SystemSource) -> DataType {
    match source {
        SystemSource::Time | SystemSource::TimeDelta => DataType::Float,
        SystemSource::Frame => DataType::Int,
        SystemSource::Mouse => DataType::Vec4,
        SystemSource::Resolution => DataType::Vec3,
    }
}

fn system_shader_code(source: SystemSource) -> String {
    let (wgsl_type, expression) = match source {
        SystemSource::Time | SystemSource::TimeDelta => ("f32", "vec4f(value, 0.0, 0.0, 1.0)"),
        SystemSource::Frame => ("i32", "vec4f(f32(value), 0.0, 0.0, 1.0)"),
        SystemSource::Mouse => ("vec4f", "value"),
        SystemSource::Resolution => ("vec3f", "vec4f(value, 1.0)"),
    };
    format!(
        "@group(0) @binding(0) var<uniform> value: {wgsl_type};\n\
         @fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {{ return {expression}; }}"
    )
}

fn math_label(op: &str) -> Option<&'static str> {
    Some(match op {
        "add" => "Add",
        "subtract" => "Subtract",
        "multiply" => "Multiply",
        "divide" => "Divide",
        "negate" => "Negate",
        "modulo" => "Modulo",
        "min" => "Min",
        "max" => "Max",
        "clamp" => "Clamp",
        "saturate" => "Saturate",
        "step" => "Step",
        "smoothstep" => "Smoothstep",
        "abs" => "Abs",
        "sign" => "Sign",
        "sin" => "Sin",
        "cos" => "Cos",
        "tan" => "Tan",
        "asin" => "Asin",
        "acos" => "Acos",
        "atan" => "Atan",
        "pow" => "Pow",
        "sqrt" => "Sqrt",
        "exp" => "Exp",
        "log" => "Log",
        "mix" => "Mix",
        "floor" => "Floor",
        "ceil" => "Ceil",
        "round" => "Round",
        "fract" => "Fract",
        _ => return None,
    })
}

fn math_input_count(op: &str) -> Option<usize> {
    Some(match op {
        "negate" | "saturate" | "abs" | "sign" | "sin" | "cos" | "tan" | "asin" | "acos"
        | "atan" | "sqrt" | "exp" | "log" | "floor" | "ceil" | "round" | "fract" => 1,
        "add" | "subtract" | "multiply" | "divide" | "modulo" | "min" | "max" | "step"
        | "pow" => 2,
        "clamp" | "smoothstep" | "mix" => 3,
        _ => return None,
    })
}

fn math_ports(node_id: &str, count: usize, direction: PortDirection) -> Vec<Port> {
    let labels = ["a", "b", "c"];
    (0..count)
        .map(|index| Port {
            id: format!("{node_id}_{}", labels.get(index).copied().unwrap_or("value")),
            label: labels.get(index).copied().unwrap_or("value").to_owned(),
            data_type: DataType::Auto,
            direction,
            default_value: (direction == PortDirection::Input).then_some(Value::from(0)),
            description: None,
        })
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GraphLayout {
    positions: BTreeMap<NodeId, Position>,
}

impl GraphLayout {
    pub fn position(&self, node_id: &NodeId) -> Option<Position> {
        self.positions.get(node_id).copied()
    }

    pub fn set_position(&mut self, node_id: NodeId, position: Position) -> Result<(), SdkError> {
        if !self.positions.contains_key(&node_id) {
            return Err(SdkError::new(
                SdkErrorCode::UnknownNode,
                "Cannot set layout for an unknown node",
            )
            .for_node(node_id.as_str()));
        }
        self.positions.insert(node_id, position);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResourceKind {
    Image,
    Video,
    Camera,
    Model,
    Bytes,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "kebab-case")]
pub enum ResourceSource {
    Path(String),
    Url(String),
    Device(String),
    Embedded(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resource {
    id: ResourceId,
    kind: ResourceKind,
    source: ResourceSource,
}

impl Resource {
    pub fn new(id: ResourceId, kind: ResourceKind, source: ResourceSource) -> Self {
        Self { id, kind, source }
    }

    pub fn id(&self) -> &ResourceId {
        &self.id
    }

    pub fn kind(&self) -> ResourceKind {
        self.kind
    }

    pub fn source(&self) -> &ResourceSource {
        &self.source
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResourceCatalog {
    resources: Vec<Resource>,
}

impl ResourceCatalog {
    pub fn add(&mut self, resource: Resource) -> Result<(), SdkError> {
        if self
            .resources
            .iter()
            .any(|current| current.id == resource.id)
        {
            return Err(SdkError::new(
                SdkErrorCode::InvalidResource,
                "Project resource ID is already registered",
            )
            .with_details(resource.id.to_string()));
        }
        self.resources.push(resource);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.resources.len()
    }

    pub fn is_empty(&self) -> bool {
        self.resources.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Resource> {
        self.resources.iter()
    }
}

pub struct PlayerBuilder {
    environment: Environment,
    graph: Graph,
    resources: Vec<Resource>,
}

impl PlayerBuilder {
    pub fn with_resources(mut self, resources: &ResourceCatalog) -> Self {
        self.resources = resources.iter().cloned().collect();
        self
    }

    pub fn build(self) -> Result<Player, SdkError> {
        let mut runtime = Runtime::new(self.environment.capabilities());
        runtime.set_graph(&self.graph)?;
        let runtime = Arc::new(Mutex::new(runtime));
        let subscription_counter = Arc::new(AtomicU64::new(1));
        let outputs = build_outputs(&self.graph, &runtime, &subscription_counter);
        Ok(Player {
            runtime,
            outputs,
            resources: self.resources,
            subscription_counter,
            now_ns: 0,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerState {
    Empty,
    Ready,
    Playing,
    Paused,
    Stopped,
    Closed,
}

pub struct Player {
    runtime: Arc<Mutex<Runtime>>,
    outputs: Vec<Output>,
    resources: Vec<Resource>,
    subscription_counter: Arc<AtomicU64>,
    now_ns: u64,
}

impl Player {
    pub fn state(&self) -> PlayerState {
        let Ok(runtime) = self.runtime.lock() else {
            return PlayerState::Closed;
        };
        map_player_state(runtime.state())
    }

    pub fn play(&mut self) -> Result<(), SdkError> {
        self.ensure_open()?;
        let start = self.next_now();
        let first_frame = self.next_now();
        let mut runtime = lock_runtime(&self.runtime)?;
        runtime.play(start)?;
        runtime.advance(&RuntimeFrameInput {
            now_ns: first_frame,
            date: [0.0; 4],
            mouse: [0.0; 4],
            resolution: [1.0, 1.0, 1.0],
        })?;
        Ok(())
    }

    pub fn pause(&mut self) -> Result<(), SdkError> {
        self.ensure_open()?;
        let now = self.next_now();
        lock_runtime(&self.runtime)?.pause(now)
    }

    pub fn resume(&mut self) -> Result<(), SdkError> {
        self.ensure_open()?;
        let now = self.next_now();
        lock_runtime(&self.runtime)?.resume(now)
    }

    pub fn stop(&mut self) -> Result<(), SdkError> {
        self.ensure_open()?;
        lock_runtime(&self.runtime)?.stop()
    }

    pub fn apply_graph(&mut self, graph: &Graph, _change: &GraphChange) -> Result<u32, SdkError> {
        self.ensure_open()?;
        let revision = lock_runtime(&self.runtime)?.set_graph(graph)?;
        self.outputs = build_outputs(graph, &self.runtime, &self.subscription_counter);
        Ok(revision)
    }

    pub fn close(&mut self) -> Result<(), SdkError> {
        if self.state() == PlayerState::Closed {
            return Ok(());
        }
        lock_runtime(&self.runtime)?.dispose()
    }

    pub fn graph_revision(&self) -> u32 {
        lock_runtime(&self.runtime)
            .map(|runtime| runtime.revision())
            .unwrap_or_default()
    }

    pub fn outputs(&self) -> &[Output] {
        &self.outputs
    }

    pub fn resources(&self) -> &[Resource] {
        &self.resources
    }

    fn ensure_open(&self) -> Result<(), SdkError> {
        if self.state() == PlayerState::Closed {
            Err(SdkError::new(
                SdkErrorCode::Disposed,
                "Player has been closed",
            ))
        } else {
            Ok(())
        }
    }

    fn next_now(&mut self) -> u64 {
        self.now_ns = self.now_ns.saturating_add(1);
        self.now_ns
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputPolicy {
    delivery: DeliveryPolicy,
}

impl OutputPolicy {
    pub fn latest() -> Self {
        Self {
            delivery: DeliveryPolicy::Latest,
        }
    }

    pub fn every() -> Self {
        Self {
            delivery: DeliveryPolicy::Every,
        }
    }

    pub fn on_change() -> Self {
        Self {
            delivery: DeliveryPolicy::OnChange,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Output {
    key: OutputKey,
    data_type: DataType,
    runtime: Weak<Mutex<Runtime>>,
    subscription_counter: Arc<AtomicU64>,
}

impl Output {
    pub fn key(&self) -> &OutputKey {
        &self.key
    }

    pub fn subscribe(&self, policy: OutputPolicy) -> Result<Subscription, SdkError> {
        let runtime = self.runtime.upgrade().ok_or_else(|| {
            SdkError::new(SdkErrorCode::Disposed, "Output player has been closed")
        })?;
        let subscription_id = format!(
            "sdk-output-{}",
            self.subscription_counter.fetch_add(1, Ordering::Relaxed)
        );
        let transport = if matches!(self.data_type, DataType::Sampler2d | DataType::SamplerCube) {
            OutputTransport::Preview
        } else {
            OutputTransport::Value
        };
        lock_runtime(&runtime)?.subscribe_output(OutputSubscription {
            subscription_id: subscription_id.clone(),
            output: self.key.clone(),
            delivery: policy.delivery,
            transport,
            max_width: None,
            max_height: None,
        })?;
        Ok(Subscription {
            runtime: Arc::downgrade(&runtime),
            subscription_id,
            closed: false,
        })
    }
}

pub struct Subscription {
    runtime: Weak<Mutex<Runtime>>,
    subscription_id: String,
    closed: bool,
}

impl Subscription {
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    pub fn close(&mut self) -> Result<(), SdkError> {
        if self.closed {
            return Ok(());
        }
        if let Some(runtime) = self.runtime.upgrade() {
            lock_runtime(&runtime)?.unsubscribe_output(&self.subscription_id)?;
        }
        self.closed = true;
        Ok(())
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

fn build_outputs(
    graph: &Graph,
    runtime: &Arc<Mutex<Runtime>>,
    subscription_counter: &Arc<AtomicU64>,
) -> Vec<Output> {
    graph
        .nodes()
        .iter()
        .flat_map(|node| {
            node.data.outputs.iter().map(|port| Output {
                key: OutputKey::new(&node.id, &port.id),
                data_type: port.data_type,
                runtime: Arc::downgrade(runtime),
                subscription_counter: subscription_counter.clone(),
            })
        })
        .collect()
}

fn map_player_state(state: EngineState) -> PlayerState {
    match state {
        EngineState::Empty => PlayerState::Empty,
        EngineState::Ready => PlayerState::Ready,
        EngineState::Running => PlayerState::Playing,
        EngineState::Paused => PlayerState::Paused,
        EngineState::Stopped => PlayerState::Stopped,
        EngineState::Disposed => PlayerState::Closed,
    }
}

fn lock_runtime(runtime: &Arc<Mutex<Runtime>>) -> Result<MutexGuard<'_, Runtime>, SdkError> {
    runtime.lock().map_err(|_| {
        SdkError::new(
            SdkErrorCode::InvalidState,
            "Player runtime lock is poisoned",
        )
    })
}
