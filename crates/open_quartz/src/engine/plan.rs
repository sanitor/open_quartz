use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Serialize;

use crate::graph::{topological_sort, GraphEdge, GraphNode};
use crate::types::{DataType, Edge, InputMode, NodeType, ProjectNode};
use crate::wgsl::{
    compile_shader, validate_shader, CompilePort, CompileRequest, CompiledShader,
    WgslCompilationError,
};

const BUILTIN_UNIFORMS: &[&str] = &[
    "iTime",
    "iTimeDelta",
    "iFrame",
    "iDate",
    "iMouse",
    "iResolution",
    "previousFrame",
];

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetSpec {
    pub width: u32,
    pub height: u32,
    pub float: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeExecutionPlan {
    pub id: String,
    pub node_type: NodeType,
    pub upstream: BTreeMap<String, String>,
    pub builtin_ports: Vec<String>,
    pub target: Option<TargetSpec>,
    pub shader: Option<CompiledShader>,
    pub validation_errors: Vec<WgslCompilationError>,
    pub feedback: bool,
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

    for node_id in &sorted_ids {
        let Some(node) = node_map.get(node_id.as_str()) else {
            continue;
        };
        let upstream = upstream_for(node, edges);
        let connected = upstream.keys().collect::<HashSet<_>>();
        let builtin_ports = node
            .data
            .inputs
            .iter()
            .filter(|port| {
                !connected.contains(&port.label) && BUILTIN_UNIFORMS.contains(&port.label.as_str())
            })
            .map(|port| port.label.clone())
            .collect::<Vec<_>>();
        let target = target_for(node, default_width, default_height);
        let (shader, validation_errors, feedback) =
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
                    input_ports: node
                        .data
                        .inputs
                        .iter()
                        .map(|port| CompilePort {
                            label: port.label.clone(),
                            data_type: data_type_name(port.data_type).to_owned(),
                        })
                        .collect(),
                    upstream_map: upstream.clone(),
                    video_inputs,
                    target_format: "rgba8unorm".to_owned(),
                };
                let compiled = compile_shader(&request);
                let errors = validate_shader(&compiled.full_fragment_code, compiled.preamble_lines);
                let feedback = compiled.needs_feedback;
                (Some(compiled), errors, feedback)
            } else {
                (None, Vec::new(), false)
            };
        node_plans.push(NodeExecutionPlan {
            id: node.id.clone(),
            node_type: node.data.node_type,
            upstream,
            builtin_ports,
            target,
            shader,
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
                matches!(
                    node.data.node_type,
                    NodeType::Shader | NodeType::Constant | NodeType::Renderer | NodeType::Onnx
                ) && !edges.iter().any(|edge| edge.source == node.id)
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
        if node.data.input_mode == Some(InputMode::Framebuffer) {
            if let (Some(width), Some(height)) = (node.data.fb_width, node.data.fb_height) {
                return (width, height);
            }
        }
        if let (Some(width), Some(height)) = (node.data.image_width, node.data.image_height) {
            return (width, height);
        }
    }
    (512, 512)
}

fn target_for(node: &ProjectNode, default_width: u32, default_height: u32) -> Option<TargetSpec> {
    if !matches!(node.data.node_type, NodeType::Shader | NodeType::Constant) {
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
    let float = matches!(
        node.data.out_format,
        Some(crate::types::FramebufferFormat::Rgba32f)
            | Some(crate::types::FramebufferFormat::Rg32f)
            | Some(crate::types::FramebufferFormat::R32f)
    );
    Some(TargetSpec {
        width,
        height,
        float,
    })
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
