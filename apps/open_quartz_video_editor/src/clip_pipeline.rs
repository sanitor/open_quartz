use open_quartz::compiler::build_execution_plan_with_options;
use open_quartz::compiler::shader::parse_shader;
use open_quartz::project::{
    DataType, GraphCommand, InputMode, NodeFactoryRequest, NodeId, NodeType, PortDirection,
    PortKey, Position, Project, ProjectFile, ProjectNode, PROJECT_FILE_VERSION,
};
use open_quartz::Graph;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::LazyLock;

pub const SOURCE_ID: &str = "video";
pub const OUTPUT_ID: &str = "output";

const OUTPUT_CODE: &str = "@fragment\nfn main(@location(0) uv: vec2f) -> @location(0) vec4f {\n    return textureSample(image, imageSampler, uv);\n}\n";

#[derive(Clone, Copy)]
pub enum BranchComposite { Blend, LumaMask, Compare }

impl BranchComposite {
    pub fn from_node(node: &ProjectNode) -> Option<Self> {
        match (node.data.shader_template_id.as_deref(), node.data.shader_code.as_str()) {
            (Some("oq-clip-blend-v1"), BLEND_CODE) => Some(Self::Blend),
            (Some("oq-clip-luma-mask-v1"), MASK_CODE) => Some(Self::LumaMask),
            (Some("oq-monitor-wipe-v1"), COMPARE_CODE) => Some(Self::Compare),
            _ => None,
        }
    }

    fn definition(self) -> (&'static str, &'static str, &'static str, &'static str) {
        match self {
            Self::Blend => ("Blend", "oq-clip-blend-v1", BLEND_CODE, "overlay"),
            Self::LumaMask => ("Luma Mask", "oq-clip-luma-mask-v1", MASK_CODE, "matte"),
            Self::Compare => ("Monitor wipe", "oq-monitor-wipe-v1", COMPARE_CODE, "original"),
        }
    }
}

const BLEND_CODE: &str = r#"
@group(0) @binding(0) var base: texture_2d<f32>;
@group(0) @binding(1) var baseSampler: sampler;
@group(0) @binding(2) var overlay: texture_2d<f32>;
@group(0) @binding(3) var overlaySampler: sampler;
@group(0) @binding(4) var<uniform> amount: f32; // Overlay opacity, 0–1. Default 0.5
@fragment fn main(@location(0) uv: vec2f) -> @location(0) vec4f {
    let b = textureSample(base, baseSampler, uv);
    let o = textureSample(overlay, overlaySampler, uv);
    let opacity = o.a * clamp(amount, 0.0, 1.0);
    let alpha = opacity + b.a * (1.0 - opacity);
    let rgb = (o.rgb * opacity + b.rgb * b.a * (1.0 - opacity)) / max(alpha, 0.0000001);
    return vec4f(rgb, alpha);
}
"#;

const MASK_CODE: &str = r#"
@group(0) @binding(0) var base: texture_2d<f32>;
@group(0) @binding(1) var baseSampler: sampler;
@group(0) @binding(2) var matte: texture_2d<f32>;
@group(0) @binding(3) var matteSampler: sampler;
@group(0) @binding(4) var<uniform> amount: f32; // Matte strength, 0–1. Default 1
@fragment fn main(@location(0) uv: vec2f) -> @location(0) vec4f {
    let b = textureSample(base, baseSampler, uv);
    let m = textureSample(matte, matteSampler, uv);
    let coverage = clamp(dot(m.rgb, vec3f(0.27222872, 0.67408177, 0.05368952)), 0.0, 1.0) * m.a;
    return vec4f(b.rgb, b.a * mix(1.0, coverage, clamp(amount, 0.0, 1.0)));
}
"#;

const COMPARE_CODE: &str = r#"
@group(0) @binding(0) var base: texture_2d<f32>;
@group(0) @binding(1) var baseSampler: sampler;
@group(0) @binding(2) var original: texture_2d<f32>;
@group(0) @binding(3) var originalSampler: sampler;
@group(0) @binding(4) var<uniform> amount: f32; // Original frame fraction. Default 0.5
@fragment fn main(@location(0) uv: vec2f) -> @location(0) vec4f {
    let graded = textureSample(base, baseSampler, uv);
    let ungraded = textureSample(original, originalSampler, uv);
    return select(graded, ungraded, uv.x < amount);
}
"#;

/// A clip-local static graph. All mutations validate a candidate before committing it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipPipeline {
    graph: Graph,
}

impl Default for ClipPipeline {
    fn default() -> Self {
        Self { graph: default_graph().clone() }
    }
}

impl ClipPipeline {
    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    pub fn is_protected(node_id: &str) -> bool {
        matches!(node_id, SOURCE_ID | OUTPUT_ID)
    }

    pub fn validate(&self) -> Result<(), String> {
        self.graph.validate().map_err(|error| error.to_string())?;
        for expected in &default_graph().nodes {
            let current = self.graph.nodes.iter().find(|node| node.id == expected.id)
                .ok_or_else(|| format!("Clip pipeline is missing protected node {}", expected.id))?;
            if current.node_type != expected.node_type || current.data != expected.data {
                return Err(format!("Clip pipeline node {} is read-only", expected.id));
            }
        }
        if self.graph.edges.iter().any(|edge| edge.target == SOURCE_ID || edge.source == OUTPUT_ID) {
            return Err("Clip source cannot have incoming edges and clip output cannot have outgoing edges".into());
        }
        for node in &self.graph.nodes {
            if node.id.is_empty() || !node.position.x.is_finite() || !node.position.y.is_finite() {
                return Err("Clip pipeline node ID and position must be valid".into());
            }
            if node.node_type != node.data.node_type {
                return Err(format!("Node {} has inconsistent node types", node.id));
            }
            if node.id == SOURCE_ID {
                continue;
            }
            if !matches!(node.node_type, NodeType::Shader | NodeType::Constant | NodeType::Math | NodeType::ColorCorrector) {
                return Err("Clip pipelines support only static Shader, Constant, Math and Color Corrector nodes".into());
            }
            let data = &node.data;
            if data.input_mode.is_some() || data.input_data_type.is_some() || data.system_source.is_some()
                || data.image_data_url.is_some() || data.raw_data_url.is_some()
                || data.video_source_type.is_some() || data.video_url.is_some()
                || data.video_file_path.is_some() || data.video_device_id.is_some()
                || data.inference_source.is_some() || data.inference_model_id.is_some()
                || data.inference_model_path.is_some() || data.inference_catalog_id.is_some()
                || data.inference_policy.is_some() || data.feedback_enabled == Some(true)
            {
                return Err(format!("Node {} requests an unsupported external or temporal resource", node.id));
            }
            let mut port_ids = BTreeSet::new();
            let mut input_labels = BTreeSet::new();
            for port in data.inputs.iter().chain(&data.outputs) {
                if port.id.is_empty() || !port_ids.insert(port.id.as_str()) {
                    return Err(format!("Node {} has missing or duplicate port IDs", node.id));
                }
            }
            for port in &data.inputs {
                if port.direction != PortDirection::Input || !input_labels.insert(port.label.as_str()) {
                    return Err(format!("Node {} has invalid input ports", node.id));
                }
                if matches!(port.data_type, DataType::Sampler2d | DataType::SamplerCube)
                    && !self.graph.edges.iter().any(|edge| edge.target == node.id && edge.target_handle == port.id)
                {
                    return Err(format!("Node {} has an unbound texture input {}", node.id, port.label));
                }
            }
            if data.outputs.iter().any(|port| port.direction != PortDirection::Output) {
                return Err(format!("Node {} has invalid output ports", node.id));
            }
            if node.node_type == NodeType::Math {
                let op = data.math_op.as_ref().ok_or("Math node is missing an operation")?;
                let mut factory = project(Graph::default());
                let (expected, _) = factory.create_graph_node(NodeFactoryRequest::Math {
                    position: None, op: op.clone(),
                }, 0).map_err(|error| error.to_string())?;
                if !ports_match(&data.inputs, &expected.data.inputs)
                    || !ports_match(&data.outputs, &expected.data.outputs)
                    || !data.shader_code.is_empty()
                {
                    return Err(format!("Math node {} has invalid executable ports", node.id));
                }
            } else if node.node_type == NodeType::ColorCorrector {
                data.color_corrector.as_ref().ok_or("Color Corrector node is missing its configuration")?
                    .validate().map_err(|error| error.to_string())?;
                let mut factory = project(Graph::default());
                let (expected, _) = factory.create_graph_node(NodeFactoryRequest::ColorCorrector {
                    position: None,
                }, 0).map_err(|error| error.to_string())?;
                if !ports_match(&data.inputs, &expected.data.inputs)
                    || !ports_match(&data.outputs, &expected.data.outputs)
                    || !data.shader_code.is_empty()
                {
                    return Err(format!("Color Corrector node {} has invalid executable ports", node.id));
                }
            } else {
                let parsed = parse_shader(&data.shader_code);
                if let Some(error) = parsed.parse_error {
                    return Err(format!("Node {}: {error}", node.id));
                }
                if !ports_match(&data.inputs, &parsed.inputs) || !ports_match(&data.outputs, &parsed.outputs) {
                    return Err(format!("Node {} ports do not match its shader", node.id));
                }
                if data.outputs.len() != 1 || data.outputs[0].data_type != DataType::Vec4 {
                    return Err(format!("Node {} must have one vec4 fragment output", node.id));
                }
            }
        }
        let plan = build_execution_plan_with_options(&self.graph.nodes, &self.graph.edges, false);
        if plan.cycle {
            return Err("Clip pipeline contains a cycle".into());
        }
        for node in plan.nodes {
            if node.feedback || node.builtin_ports.iter().any(|name| name != "iResolution") {
                return Err(format!("Node {} uses unsupported temporal or interactive shader builtins", node.id));
            }
            if let Some(error) = node.validation_errors.first() {
                return Err(format!("Node {}: {}", node.id, error.message));
            }
            if let Some(shader) = &node.shader {
                let module = naga::front::wgsl::parse_str(&shader.full_fragment_code)
                    .map_err(|error| format!("Node {}: {}", node.id, error.emit_to_string(&shader.full_fragment_code)))?;
                if module.entry_points.len() != 1 || module.entry_points[0].stage != naga::ShaderStage::Fragment
                    || module.entry_points[0].name != "main"
                {
                    return Err(format!("Node {} must define only the main fragment entry point", node.id));
                }
                for (_, global) in module.global_variables.iter() {
                    let owned = global.binding.as_ref().is_some_and(|binding| {
                        binding.group == 0 && shader.bindings.iter().any(|expected| {
                            expected.binding == binding.binding && global.name.as_deref() == Some(expected.name.as_str())
                        })
                    });
                    if !matches!(global.space, naga::AddressSpace::Private)
                        && (!owned || !matches!(global.space, naga::AddressSpace::Uniform | naga::AddressSpace::Handle))
                    {
                        return Err(format!("Node {} declares a resource outside its graph ports", node.id));
                    }
                }
                naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
                    .validate(&module).map_err(|error| format!("Node {}: {error}", node.id))?;
            }
        }
        Ok(())
    }

    pub fn replace_graph(&mut self, graph: Graph) -> Result<(), String> {
        let candidate = Self { graph };
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    pub fn apply_command(&mut self, command: GraphCommand) -> Result<(), String> {
        if let GraphCommand::RemoveNode { node_id } = &command {
            return self.remove_nodes(&[node_id.as_str().to_owned()]);
        }
        let mut aggregate = project(self.graph.clone());
        aggregate.apply_graph_command(command, 0).map_err(|error| error.to_string())?;
        self.replace_graph(aggregate.to_file().graph)
    }

    /// Reconnect the protected output to a monitor tap in a transient copy.
    pub fn monitor_tap(&self, id: &str) -> Result<Self, String> {
        let mut graph = self.graph.clone();
        let port = graph.nodes.iter().find(|node| node.id == id && node.id != OUTPUT_ID)
            .and_then(|node| node.data.outputs.iter().find(|port| port.data_type == DataType::Vec4))
            .or_else(|| graph.nodes.iter().find(|node| node.id == id && node.id == SOURCE_ID)
                .and_then(|node| node.data.outputs.first()))
            .ok_or("Select a node with an image output")?;
        let port = port.id.clone();
        let edge = graph.edges.iter_mut().find(|edge| edge.target == OUTPUT_ID)
            .ok_or("Connect the clip output first")?;
        edge.source = id.to_owned();
        edge.source_handle = port;
        let mut candidate = self.clone();
        candidate.replace_graph(graph)?;
        Ok(candidate)
    }

    pub fn monitor_comparison(&self, split: f32) -> Result<Self, String> {
        let mut candidate = self.clone();
        let id = candidate.add_branch_composite(OUTPUT_ID, BranchComposite::Compare)?;
        let node = candidate.graph.nodes.iter_mut().find(|node| node.id == id)
            .ok_or("Missing comparison node")?;
        node.data.uniforms.insert("amount".into(), serde_json::json!(split.clamp(0.0, 1.0)));
        Ok(candidate)
    }

    /// Insert a factory shader into the selected serial connection, preserving downstream fan-out.
    pub fn add_serial(&mut self, after: &str, code: &str, label: &str) -> Result<String, String> {
        self.add_serial_factory(after, None, |position| NodeFactoryRequest::Shader {
            position: Some(position), code: code.to_owned(), label: label.to_owned(),
            template_name: Some(label.to_owned()), shader_template_id: None,
        })
    }

    pub fn add_corrector_serial(&mut self, after: &str) -> Result<String, String> {
        self.add_serial_factory(after, None, |position| NodeFactoryRequest::ColorCorrector {
            position: Some(position),
        })
    }

    pub(crate) fn is_lut(node: &ProjectNode) -> bool {
        node.data.shader_template_id.as_deref() == Some(crate::color_lut::TEMPLATE)
            && node.data.shader_code.starts_with("\n// OpenQuartz embedded ACEScg CUBE\n")
    }

    pub fn add_lut(&mut self, after: &str, code: String, label: String) -> Result<String, String> {
        self.add_serial_factory(after, None, |position| NodeFactoryRequest::Shader {
            position: Some(position), code, label, template_name: None,
            shader_template_id: Some(crate::color_lut::TEMPLATE.to_owned()),
        })
    }

    pub fn add_branch_composite(&mut self, after: &str, operation: BranchComposite) -> Result<String, String> {
        let (label, template, code, auxiliary) = operation.definition();
        let id = self.add_serial_factory(after, Some(auxiliary), |position| NodeFactoryRequest::Shader {
            position: Some(position), code: code.to_owned(), label: label.to_owned(),
            template_name: None, shader_template_id: Some(template.to_owned()),
        })?;
        let amount = match operation { BranchComposite::Blend | BranchComposite::Compare => 0.5, BranchComposite::LumaMask => 1.0 };
        if let Some(node) = self.graph.nodes.iter_mut().find(|node| node.id == id) {
            node.data.uniforms.insert("amount".into(), serde_json::json!(amount));
        }
        Ok(id)
    }

    fn add_serial_factory(&mut self, after: &str, auxiliary: Option<&str>, factory: impl FnOnce(Position) -> NodeFactoryRequest) -> Result<String, String> {
        let before_output = after == OUTPUT_ID;
        let after = if after == OUTPUT_ID {
            self.graph.edges.iter().find(|edge| edge.target == OUTPUT_ID)
                .ok_or("Clip output is not connected")?.source.as_str()
        } else {
            after
        };
        let source = self.graph.nodes.iter().find(|node| node.id == after)
            .ok_or("Selected pipeline node does not exist")?;
        if source.data.outputs.len() != 1 {
            return Err("Serial insertion requires one unambiguous source output".into());
        }
        let downstream = self.graph.edges.iter().filter(|edge| {
            edge.source == after && (!before_output || edge.target == OUTPUT_ID)
        }).cloned().collect::<Vec<_>>();
        if downstream.is_empty() {
            return Err("Serial insertion requires a downstream connection".into());
        }
        let mut aggregate = project(self.graph.clone());
        let (node, _) = aggregate.create_graph_node(factory(Position {
            x: source.position.x + 240.0, y: source.position.y,
        }), 0).map_err(|error| error.to_string())?;
        let samplers = node.data.inputs.iter().filter(|port| port.data_type == DataType::Sampler2d).collect::<Vec<_>>();
        if samplers.len() != 1 + usize::from(auxiliary.is_some()) || node.data.outputs.len() != 1 {
            return Err("Serial insertion requires exactly one image input and one fragment output".into());
        }
        let auxiliary = auxiliary.map(|label| {
            samplers.iter().find(|port| port.label == label).ok_or("Missing auxiliary image input")
        }).transpose()?;
        let primary = samplers.iter().find(|port| auxiliary.is_none_or(|aux| aux.id != port.id))
            .ok_or("Missing primary image input")?;
        let mut graph = aggregate.to_file().graph;
        graph.edit(&mut |edit| {
            for edge in &downstream {
                edit.disconnect(&edge.id)?;
            }
            edit.connect(key(after, &source.data.outputs[0].id), key(&node.id, &primary.id))?;
            if let Some(auxiliary) = auxiliary {
                edit.connect(key(SOURCE_ID, "video_output"), key(&node.id, &auxiliary.id))?;
            }
            for edge in &downstream {
                edit.connect(key(&node.id, &node.data.outputs[0].id), key(&edge.target, &edge.target_handle))?;
            }
            Ok(())
        }).map_err(|error| error.to_string())?;
        self.replace_graph(graph)?;
        Ok(node.id)
    }

    /// Remove serial stages by bridging their sole image input; ambiguous merges are rejected.
    pub fn remove_nodes(&mut self, node_ids: &[String]) -> Result<(), String> {
        let selected = node_ids.iter().map(String::as_str).collect::<BTreeSet<_>>();
        for id in &selected {
            if Self::is_protected(id) {
                return Err(format!("Clip pipeline node {id} cannot be removed"));
            }
            if !self.graph.nodes.iter().any(|node| node.id == *id) {
                return Err(format!("Pipeline node {id} does not exist"));
            }
        }
        let order = build_execution_plan_with_options(&self.graph.nodes, &self.graph.edges, false).sorted_ids;
        let mut graph = self.graph.clone();
        for id in order.iter().filter(|id| selected.contains(id.as_str())) {
            let node = graph.nodes.iter().find(|node| node.id == *id).expect("validated selected node");
            let outgoing = graph.edges.iter().filter(|edge| edge.source == *id).cloned().collect::<Vec<_>>();
            let incoming = graph.edges.iter().filter(|edge| {
                edge.target == *id && node.data.inputs.iter().any(|port| {
                    port.id == edge.target_handle && port.data_type == DataType::Sampler2d
                        && (BranchComposite::from_node(node).is_none() || port.label == "base")
                })
            }).cloned().collect::<Vec<_>>();
            let bridge = if outgoing.is_empty() || matches!(node.node_type, NodeType::Constant | NodeType::Math) {
                None
            } else if incoming.len() == 1 && node.data.outputs.len() == 1 {
                Some(incoming[0].clone())
            } else {
                return Err(format!("Cannot safely bridge node {id}: its input is ambiguous"));
            };
            graph.edit(&mut |edit| {
                edit.remove_node(&NodeId::new(id))?;
                if let Some(upstream) = &bridge {
                    for edge in &outgoing {
                        edit.connect(key(&upstream.source, &upstream.source_handle), key(&edge.target, &edge.target_handle))?;
                    }
                }
                Ok(())
            }).map_err(|error| error.to_string())?;
        }
        self.replace_graph(graph)
    }
}

fn ports_match(actual: &[open_quartz::project::Port], expected: &[open_quartz::project::Port]) -> bool {
    actual.len() == expected.len() && actual.iter().zip(expected).all(|(actual, expected)| {
        actual.label == expected.label && actual.data_type == expected.data_type && actual.direction == expected.direction
    })
}

fn key(node: &str, port: &str) -> PortKey {
    PortKey::new(NodeId::new(node), open_quartz::project::PortId::new(port))
}

pub(super) fn project(graph: Graph) -> Project {
    Project::from_file(ProjectFile {
        version: PROJECT_FILE_VERSION.to_owned(), name: "Clip pipeline".to_owned(),
        created_at: String::new(), updated_at: String::new(), graph,
    })
}

fn rename_node(mut node: ProjectNode, id: &str, label: &str) -> ProjectNode {
    node.id = id.to_owned();
    node.data.label = label.to_owned();
    for port in node.data.inputs.iter_mut().chain(&mut node.data.outputs) {
        port.id = format!("{id}_{}", port.label);
    }
    node
}

fn default_graph() -> &'static Graph {
    static GRAPH: LazyLock<Graph> = LazyLock::new(|| {
        let mut factory = project(Graph::default());
        let (source, _) = factory.create_graph_node(NodeFactoryRequest::Input {
            position: Some(Position { x: 60.0, y: 120.0 }),
            data_type: DataType::Sampler2d, input_mode: Some(InputMode::Video),
        }, 0).expect("valid video source factory");
        let mut source = rename_node(source, SOURCE_ID, "Clip Source");
        source.data.inputs.clear();
        source.data.outputs[0].id = "video_output".to_owned();
        source.data.outputs[0].label = "output".to_owned();
        let (output, _) = factory.create_graph_node(NodeFactoryRequest::Shader {
            position: Some(Position { x: 560.0, y: 120.0 }), code: OUTPUT_CODE.to_owned(),
            label: "Clip Output".to_owned(), template_name: None, shader_template_id: None,
        }, factory.graph_revision()).expect("valid output shader factory");
        let output = rename_node(output, OUTPUT_ID, "Clip Output");
        let mut graph = Graph { nodes: vec![source, output], edges: Vec::new() };
        graph.apply_command(GraphCommand::Connect {
            source: key(SOURCE_ID, "video_output"), target: key(OUTPUT_ID, "output_image"),
        }).expect("valid default clip connection");
        graph
    });
    &GRAPH
}

#[cfg(test)]
#[path = "clip_pipeline_tests.rs"]
mod tests;
