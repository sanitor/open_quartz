use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Serialize;

use crate::compiler::topology::{topological_sort, GraphEdge, GraphNode};
use crate::compiler::shader::{compile_shader, validate_shader, CompilePort, CompileRequest, CompiledShader, WgslCompilationError};
use crate::project::model::{DataType, Edge, InputMode, NodeType, ProjectNode};
use crate::operators::{ColorDomain, OperatorStructuralKey};
use std::sync::Arc;

#[derive(Clone, Debug, Serialize)]
pub struct StructuredOperatorPlan {
    pub structural_key: OperatorStructuralKey,
    #[serde(serialize_with = "serialize_program")]
    pub program: Option<Arc<CompiledShader>>,
    pub parameters: BTreeMap<String, Vec<f32>>,
    /// Logical output -> connected input label. No target or draw for aliases.
    pub alias_input: Option<String>,
}

fn serialize_program<S: serde::Serializer>(program: &Option<Arc<CompiledShader>>, serializer: S) -> Result<S::Ok, S::Error> {
    program.as_deref().serialize(serializer)
}

const BUILTIN_UNIFORMS: &[&str] = &[
    "iTime",
    "iTimeDelta",
    "iFrame",
    "iDate",
    "iMouse",
    "iResolution",
    "previousFrame",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum TargetFormat {
    Rgba8Unorm,
    Rgba16Float,
    Rgba32Float,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetSpec {
    pub width: u32,
    pub height: u32,
    pub format: TargetFormat,
    /// Auto-sized targets follow this connected sampler's current texture dimensions.
    pub size_source: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeExecutionPlan {
    pub id: String,
    pub node_type: NodeType,
    pub upstream: BTreeMap<String, String>,
    pub builtin_ports: Vec<String>,
    pub target: Option<TargetSpec>,
    #[serde(serialize_with = "serialize_program")]
    pub shader: Option<Arc<CompiledShader>>,
    pub structured: Option<StructuredOperatorPlan>,
    pub output_domain: ColorDomain,
    pub validation_errors: Vec<WgslCompilationError>,
    pub feedback: bool,
}

impl NodeExecutionPlan {
    pub fn gpu_program(&self) -> Option<&CompiledShader> {
        self.shader.as_deref().or_else(|| self.structured.as_ref()?.program.as_deref())
    }

    pub fn alias_source(&self) -> Option<&str> {
        self.structured.as_ref()?.alias_input.as_ref()
            .and_then(|label| self.upstream.get(label)).map(String::as_str)
    }
}

/// Repack a typed value-only candidate without regenerating any WGSL. Every
/// topology/data/structural difference falls back to full checked lowering.
pub(crate) fn update_parameter_plan(previous: &ExecutionPlan, old_nodes: &[ProjectNode], old_edges: &[Edge], nodes: &[ProjectNode], edges: &[Edge]) -> Option<ExecutionPlan> {
    if nodes.len() != old_nodes.len() || edges != old_edges { return None; }
    let mut parameters = Vec::new();
    for node in nodes {
        let old = old_nodes.iter().find(|old| old.id == node.id)?;
        let plan = previous.nodes.iter().find(|plan| plan.id == node.id)?;
        if node.node_type != old.node_type || upstream_for(node, edges) != plan.upstream { return None; }
        if node.data == old.data { continue; }
        if node.data.node_type != NodeType::ColorCorrector { return None; }
        let mut data = old.data.clone();
        data.color_corrector = node.data.color_corrector.clone();
        if data != node.data { return None; }
        let operator = plan.structured.as_ref()?;
        let config = node.data.color_corrector.as_ref()?;
        if config.structure_key(operator.structural_key.input_domain).ok()? != operator.structural_key { return None; }
        parameters.push((node.id.as_str(), config.parameter_values().ok()?));
    }
    let mut next = previous.clone();
    for (id, values) in parameters {
        next.nodes.iter_mut().find(|node| node.id == id)?.structured.as_mut()?.parameters = values;
    }
    Some(next)
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionPlan {
    pub sorted_ids: Vec<String>,
    pub nodes: Vec<NodeExecutionPlan>,
    pub output_nodes: Vec<String>,
    pub default_width: u32,
    pub default_height: u32,
    pub cycle: bool,
}

impl ExecutionPlan {
    /// Resolve only proven resource aliases. Logical IDs remain unchanged in
    /// graph history, evaluation stamps and output subscriptions.
    pub fn resolve_alias_source<'a>(&'a self, mut id: &'a str) -> Option<&'a str> {
        for _ in 0..=self.nodes.len() {
            let node = self.nodes.iter().find(|node| node.id == id)?;
            let source = node.alias_source().or_else(|| {
                (node.node_type == NodeType::Renderer && node.gpu_program().is_none())
                    .then(|| node.upstream.values().next().map(String::as_str)).flatten()
            });
            match source { Some(source) => id = source, None => return Some(id) }
        }
        None
    }

    /// ColorCorrector is a straight-alpha operator; aliases and explicit
    /// display-only Renderer boundaries preserve that contract transitively.
    pub fn output_is_straight_alpha<'a>(&'a self, mut id: &'a str) -> bool {
        for _ in 0..=self.nodes.len() {
            let Some(node) = self.nodes.iter().find(|node| node.id == id) else { return false; };
            if node.node_type == NodeType::ColorCorrector {
                if let Some(source) = node.alias_source() { id = source; continue; }
                return node.structured.is_some();
            }
            if node.node_type == NodeType::Renderer {
                if let Some(source) = node.upstream.values().next() { id = source; continue; }
            }
            return false;
        }
        false
    }
}

pub fn build_execution_plan(nodes: &[ProjectNode], edges: &[Edge]) -> ExecutionPlan {
    build_execution_plan_with_options(nodes, edges, true)
}

pub fn build_execution_plan_with_options(
    nodes: &[ProjectNode],
    edges: &[Edge],
    external_video_textures: bool,
) -> ExecutionPlan {
    let graph_nodes = nodes
        .iter()
        .map(|node| GraphNode {
            id: node.id.clone(),
        })
        .collect::<Vec<_>>();
    let graph_edges = edges
        .iter()
        .map(|edge| GraphEdge {
            source: edge.source.clone(),
            target: edge.target.clone(),
        })
        .collect::<Vec<_>>();
    let sorted_ids = topological_sort(&graph_nodes, &graph_edges);
    let cycle = sorted_ids.len() < nodes.len();
    let node_map = nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let (default_width, default_height) = default_size(nodes);
    let mut node_plans = Vec::new();
    let mut output_sizes = HashMap::new();
    let mut output_domains = HashMap::new();

    for node_id in &sorted_ids {
        let node = node_map
            .get(node_id.as_str())
            .expect("topological sort only returns input node IDs");
        let upstream = upstream_for(node, edges);
        let connected = upstream.keys().collect::<HashSet<_>>();
        let mut input_ports = node.data.inputs.iter().map(|port| CompilePort {
            label: port.label.clone(),
            data_type: data_type_name(port.data_type).to_owned(),
        }).collect::<Vec<_>>();
        // Builtins are deliberately omitted from parsed user ports, but shader code
        // can refer to them without declaring a binding (as in the catalog).
        let shader_code = crate::compiler::shader::parser::strip_comments(&node.data.shader_code);
        for (name, data_type) in [
            ("iTime", "float"), ("iTimeDelta", "float"), ("iFrame", "float"),
            ("iDate", "vec4"), ("iMouse", "vec4"), ("iResolution", "vec3"),
        ] {
            if !input_ports.iter().any(|port| port.label == name)
                && shader_code.split(|c: char| !c.is_ascii_alphanumeric() && c != '_').any(|word| word == name)
            {
                input_ports.push(CompilePort { label: name.to_owned(), data_type: data_type.to_owned() });
            }
        }
        let builtin_ports = input_ports.iter()
            .filter(|port| !connected.contains(&port.label) && BUILTIN_UNIFORMS.contains(&port.label.as_str()))
            .map(|port| port.label.clone())
            .collect::<Vec<_>>();
        let size_source = node.data.inputs.iter()
            .filter(|port| matches!(port.data_type, DataType::Sampler2d | DataType::SamplerCube))
            .find_map(|port| upstream.get(&port.label));
        let mut target = target_for(node, default_width, default_height);
        if let Some(target) = &mut target {
            if node.data.auto_size != Some(false) {
                target.size_source = size_source.cloned();
                if let Some(&(width, height)) = size_source.and_then(|id| output_sizes.get(id.as_str())) {
                    target.width = width;
                    target.height = height;
                }
            }
            output_sizes.insert(node.id.as_str(), (target.width, target.height));
        } else if let Some(size) = source_size(node).or_else(|| {
            size_source.and_then(|id| output_sizes.get(id.as_str()).copied())
        }) {
            output_sizes.insert(node.id.as_str(), size);
        }
        let (mut shader, mut validation_errors, feedback) =
            if matches!(node.data.node_type, NodeType::Shader | NodeType::Constant) {
                let video_inputs = if external_video_textures {
                    upstream
                        .iter()
                        .filter_map(|(label, source_id)| {
                            node_map
                                .get(source_id.as_str())
                                .filter(|source| source.data.input_mode == Some(InputMode::Video))
                                .map(|_| label.clone())
                        })
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                let request = CompileRequest {
                    user_code: node.data.shader_code.clone(),
                    input_ports,
                    upstream_map: upstream.clone(),
                    video_inputs,
                    target_format: "rgba8unorm".to_owned(),
                };
                let compiled = compile_shader(&request);
                let errors = validate_shader(&compiled.full_fragment_code, compiled.preamble_lines);
                let feedback = compiled.needs_feedback;
                (Some(Arc::new(compiled)), errors, feedback)
            } else {
                (None, Vec::new(), false)
            };
        let mut structured = None;
        let input_domain = size_source.and_then(|id| output_domains.get(id)).copied().unwrap_or(ColorDomain::LegacySrgb);
        let mut output_domain = ColorDomain::LegacySrgb;
        if node.data.node_type == NodeType::ColorCorrector {
            let lowered = node.data.color_corrector.as_ref()
                .ok_or_else(|| crate::operators::OperatorError("ColorCorrector config is missing".into()))
                .and_then(|config| crate::operators::lower_color_corrector(config, input_domain));
            match lowered {
                Ok(lowered) => {
                    output_domain = lowered.output_domain;
                    let program = lowered.shader_source.map(|user_code| {
                        let mut ports = vec![CompilePort { label: "source".into(), data_type: "sampler2D".into() }];
                        ports.extend(lowered.parameter_types.iter().map(|(label, _)| CompilePort { label: label.clone(), data_type: "vec4".into() }));
                        Arc::new(compile_shader(&CompileRequest { user_code, input_ports: ports,
                            upstream_map: upstream.clone(), video_inputs: Vec::new(), target_format: "rgba16float".into() }))
                    });
                    if let Some(program) = &program {
                        validation_errors.extend(validate_shader(&program.full_fragment_code, program.preamble_lines));
                    } else {
                        target = None;
                    }
                    structured = Some(StructuredOperatorPlan { structural_key: lowered.structural_key, program,
                        parameters: lowered.parameters, alias_input: lowered.alias.map(|alias| alias.input_port) });
                }
                Err(error) => validation_errors.push(plan_error(error.to_string())),
            }
            if !upstream.contains_key("source") { validation_errors.push(plan_error("ColorCorrector requires a connected source")); }
        } else if node.data.node_type == NodeType::Renderer && input_domain == ColorDomain::AcesCgLinear {
            if let Some((label, source)) = upstream.iter().next() {
                shader = Some(Arc::new(super::color_boundary::display_program(label, source)));
                target = Some(TargetSpec { width: default_width, height: default_height, format: TargetFormat::Rgba8Unorm, size_source: Some(source.clone()) });
            }
        } else if matches!(node.data.node_type, NodeType::Shader | NodeType::Constant) {
            if let Some(program) = &mut shader {
                let managed: HashSet<&str> = upstream.iter().filter(|(_, id)| output_domains.get(*id) == Some(&ColorDomain::AcesCgLinear))
                    .map(|(label, _)| label.as_str()).collect();
                if !managed.is_empty() {
                    let program = Arc::make_mut(program);
                    match super::color_boundary::convert_managed_samples(&program.full_fragment_code, &managed) {
                        Ok(code) => { program.full_fragment_code = code; validation_errors = validate_shader(&program.full_fragment_code, program.preamble_lines); }
                        Err(error) => validation_errors.push(plan_error(error)),
                    }
                }
            }
        }
        output_domains.insert(node.id.clone(), output_domain);
        node_plans.push(NodeExecutionPlan {
            id: node.id.clone(),
            node_type: node.data.node_type,
            upstream,
            builtin_ports,
            target,
            shader,
            structured,
            output_domain,
            validation_errors,
            feedback,
        });
    }

    let renderer_nodes = nodes
        .iter()
        .filter(|node| node.data.node_type == NodeType::Renderer)
        .map(|node| node.id.clone())
        .collect::<Vec<_>>();
    let output_nodes = if renderer_nodes.is_empty() {
        nodes
            .iter()
            .filter(|node| {
                (matches!(
                    node.data.node_type,
                    NodeType::Shader | NodeType::Constant | NodeType::Renderer | NodeType::Inference | NodeType::ColorCorrector
                ) || (node.data.node_type == NodeType::Input
                    && node.data.outputs.iter().any(|port| port.data_type == DataType::Sampler2d)))
                    && !edges.iter().any(|edge| edge.source == node.id)
            })
            .map(|node| node.id.clone())
            .collect()
    } else {
        renderer_nodes
    };

    ExecutionPlan {
        sorted_ids,
        nodes: node_plans,
        output_nodes,
        default_width,
        default_height,
        cycle,
    }
}

fn upstream_for(node: &ProjectNode, edges: &[Edge]) -> BTreeMap<String, String> {
    let mut upstream = BTreeMap::new();
    for edge in edges.iter().filter(|edge| edge.target == node.id) {
        if let Some(port) = node
            .data
            .inputs
            .iter()
            .find(|port| port.id == edge.target_handle)
        {
            upstream.insert(port.label.clone(), edge.source.clone());
        }
    }
    upstream
}

fn default_size(nodes: &[ProjectNode]) -> (u32, u32) {
    for node in nodes {
        if let Some(size) = source_size(node) {
            return size;
        }
    }
    (512, 512)
}

fn source_size(node: &ProjectNode) -> Option<(u32, u32)> {
    if node.data.input_mode == Some(InputMode::Framebuffer) {
        if let (Some(width), Some(height)) = (node.data.fb_width, node.data.fb_height) {
            return Some((width, height));
        }
    }
    node.data.image_width.zip(node.data.image_height)
}

fn target_for(node: &ProjectNode, default_width: u32, default_height: u32) -> Option<TargetSpec> {
    if !matches!(node.data.node_type, NodeType::Shader | NodeType::Constant | NodeType::ColorCorrector) {
        return None;
    }
    let (width, height) = if node.data.auto_size == Some(false) {
        (
            node.data.width.unwrap_or(default_width),
            node.data.height.unwrap_or(default_height),
        )
    } else {
        (default_width, default_height)
    };
    let format = if node.data.node_type == NodeType::ColorCorrector {
        TargetFormat::Rgba16Float
    } else {
        match node.data.out_format {
            Some(crate::project::model::FramebufferFormat::Rgba32f)
            | Some(crate::project::model::FramebufferFormat::Rg32f)
            | Some(crate::project::model::FramebufferFormat::R32f) => TargetFormat::Rgba32Float,
            _ => TargetFormat::Rgba8Unorm,
        }
    };
    Some(TargetSpec {
        width,
        height,
        format,
        size_source: None,
    })
}

fn plan_error(message: impl Into<String>) -> WgslCompilationError {
    WgslCompilationError { message: message.into(), line: 0, column: 0, offset: 0, length: 0 }
}

fn data_type_name(data_type: DataType) -> &'static str {
    match data_type {
        DataType::Float => "float",
        DataType::Int => "int",
        DataType::Uint => "uint",
        DataType::Bool => "bool",
        DataType::Vec2 => "vec2",
        DataType::Vec3 => "vec3",
        DataType::Vec4 => "vec4",
        DataType::Ivec2 => "ivec2",
        DataType::Ivec3 => "ivec3",
        DataType::Ivec4 => "ivec4",
        DataType::Uvec2 => "uvec2",
        DataType::Uvec3 => "uvec3",
        DataType::Uvec4 => "uvec4",
        DataType::Bvec2 => "bvec2",
        DataType::Bvec3 => "bvec3",
        DataType::Bvec4 => "bvec4",
        DataType::Mat2 => "mat2",
        DataType::Mat3 => "mat3",
        DataType::Mat4 => "mat4",
        DataType::Sampler2d => "sampler2D",
        DataType::SamplerCube => "samplerCube",
        DataType::Roi => "roi",
        DataType::Mesh => "mesh",
        DataType::Json => "json",
        DataType::Auto => "auto",
    }
}

#[cfg(test)]
#[path = "plan_coverage_tests.rs"]
mod coverage_tests;
