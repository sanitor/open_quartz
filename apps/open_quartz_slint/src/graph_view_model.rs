use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::{Duration, Instant};

use open_quartz_sdk::{DataType, InputMode, NodeType, Port, Project, ProjectNode};
use slint::{ModelRc, VecModel};

use crate::{GraphEdge, GraphNode, GraphPort};

const NODE_WIDTH: f32 = 200.0;
const PORT_TOP: f32 = 41.0;
const PORT_SPACING: f32 = 26.0;
const CONTENT_PADDING: f32 = 24.0;

pub struct GraphViewModel {
    pub nodes: ModelRc<GraphNode>,
    pub edges: ModelRc<GraphEdge>,
    pub content_width: f32,
    pub content_height: f32,
    pub origin_x: f32,
    pub origin_y: f32,
    pub node_count: usize,
    pub edge_count: usize,
    pub projection_duration: Duration,
}

impl GraphViewModel {
    pub fn from_project(project: &Project, selected_node_ids: &BTreeSet<String>) -> Self {
        let started = Instant::now();
        let graph = project.graph();
        let node_indices = graph
            .nodes()
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id.as_str(), index))
            .collect::<HashMap<_, _>>();
        let connected_inputs = graph
            .edges()
            .iter()
            .map(|edge| (edge.target.as_str(), edge.target_handle.as_str()))
            .collect::<HashSet<_>>();
        // Tight graph-space bounds keep Fit View independent of the world's origin.
        let bounds = graph
            .nodes()
            .iter()
            .map(|node| {
                let x = node.position.x as f32;
                let y = node.position.y as f32;
                let height = node_height(node);
                (x, y, x + node_width(node), y + height)
            })
            .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)));
        let (mut min_x, min_y, mut max_x, max_y) = bounds.unwrap_or((0.0, 0.0, 0.0, 0.0));
        // Include the Bezier control hull, including backwards-running connections.
        for edge in graph.edges() {
            if let (Some(&source), Some(&target)) = (
                node_indices.get(edge.source.as_str()),
                node_indices.get(edge.target.as_str()),
            ) {
                min_x = min_x.min(graph.nodes()[target].position.x as f32 - 48.0);
                max_x = max_x.max(
                    graph.nodes()[source].position.x as f32
                        + node_width(&graph.nodes()[source])
                        + 48.0,
                );
            }
        }
        let origin_x = CONTENT_PADDING - min_x;
        let origin_y = CONTENT_PADDING - min_y;
        let content_width = max_x - min_x + CONTENT_PADDING * 2.0;
        let content_height = max_y - min_y + CONTENT_PADDING * 2.0;
        let nodes = graph
            .nodes()
            .iter()
            .map(|node| GraphNode {
                node_id: node.id.as_str().into(),
                label: node.data.label.as_str().into(),
                kind: format!("{:?}", node.node_type).into(),
                title: node_title(node).into(),
                status: node_status(node, &connected_inputs).into(),
                preview: slint::Image::default(),
                has_preview: false,
                preview_top: port_body_height(node) + 2.0,
                preview_height: preview_height(node),
                preview_expanded: node.data.extra.get("expanded").and_then(serde_json::Value::as_bool).unwrap_or(true),
                resolution: Default::default(),
                x: node.position.x as f32,
                y: node.position.y as f32,
                width: node_width(node),
                height: node_height(node),
                ports: ModelRc::new(VecModel::from(
                    node.data
                        .inputs
                        .iter()
                        .enumerate()
                        .map(|(index, port)| {
                            project_port(
                                port,
                                index,
                                false,
                                node,
                                connected_inputs.contains(&(node.id.as_str(), port.id.as_str())),
                            )
                        })
                        .chain(
                            node.data
                                .outputs
                                .iter()
                                .enumerate()
                                .map(|(index, port)| project_port(port, index, true, node, true)),
                        )
                        .collect::<Vec<_>>(),
                )),
                selected: selected_node_ids.contains(&node.id),
            })
            .collect::<Vec<_>>();
        let edges = graph
            .edges()
            .iter()
            .filter_map(|edge| {
                let source_index = *node_indices.get(edge.source.as_str())?;
                let target_index = *node_indices.get(edge.target.as_str())?;
                let source = &graph.nodes()[source_index];
                let target = &graph.nodes()[target_index];
                Some(GraphEdge {
                    edge_id: edge.id.as_str().into(),
                    source_index: source_index as i32,
                    target_index: target_index as i32,
                    source_x: source.position.x as f32 + node_width(source),
                    source_y: source.position.y as f32
                        + port_y(
                            source
                                .data
                                .outputs
                                .iter()
                                .position(|port| port.id == edge.source_handle)?,
                            source,
                            true,
                        ),
                    target_x: target.position.x as f32,
                    target_y: target.position.y as f32
                        + port_y(
                            target
                                .data
                                .inputs
                                .iter()
                                .position(|port| port.id == edge.target_handle)?,
                            target,
                            false,
                        ),
                })
            })
            .collect::<Vec<_>>();
        let projection_duration = started.elapsed();
        let node_count = nodes.len();
        let edge_count = edges.len();
        Self {
            nodes: ModelRc::new(VecModel::from(nodes)),
            edges: ModelRc::new(VecModel::from(edges)),
            content_width,
            content_height,
            origin_x,
            origin_y,
            node_count,
            edge_count,
            projection_duration,
        }
    }
}

fn node_width(node: &ProjectNode) -> f32 {
    if node.node_type == NodeType::Math {
        120.0
    } else {
        NODE_WIDTH
    }
}

fn port_y(index: usize, node: &ProjectNode, output: bool) -> f32 {
    if node.node_type == NodeType::Math {
        return 39.0 + index as f32 * 22.0;
    }
    let offset = if output && !node.data.inputs.is_empty() {
        node.data.inputs.len() as f32 * PORT_SPACING + 1.0
    } else {
        0.0
    };
    PORT_TOP + offset + index as f32 * PORT_SPACING
}

fn port_body_height(node: &ProjectNode) -> f32 {
    if node.node_type == NodeType::Math {
        28.0 + node.data.inputs.len().max(node.data.outputs.len()).max(1) as f32 * 22.0 + 4.0
    } else {
        28.0 + (node.data.inputs.len() + node.data.outputs.len()).max(1) as f32 * PORT_SPACING + 5.0
    }
}

fn preview_height(node: &ProjectNode) -> f32 {
    match node.node_type {
        NodeType::Renderer => 112.0,
        NodeType::Shader => 64.0,
        NodeType::Input if matches!(node.data.input_mode, Some(InputMode::Image | InputMode::Framebuffer | InputMode::Video)) => 100.0,
        NodeType::Onnx if node.data.outputs.iter().any(|port| port.data_type == DataType::Sampler2d) => 64.0,
        _ => 0.0,
    }
}

fn node_height(node: &ProjectNode) -> f32 {
    let expanded = node.data.extra.get("expanded").and_then(serde_json::Value::as_bool).unwrap_or(true);
    port_body_height(node) + if expanded && preview_height(node) > 0.0 { preview_height(node) + 22.0 } else { 0.0 }
}

fn node_title(node: &ProjectNode) -> String {
    match node.node_type {
        NodeType::Shader | NodeType::Onnx => node.data.template_name.as_deref().unwrap_or(&node.data.label).to_uppercase(),
        NodeType::Input => match node.data.input_mode {
            Some(InputMode::Image) => "IMAGE".into(), Some(InputMode::Video) => "VIDEO".into(),
            Some(InputMode::Framebuffer) => "FRAMEBUFFER".into(), Some(InputMode::System) => node.data.system_source.map(|source| format!("{source:?}").to_uppercase()).unwrap_or_else(|| "SYSTEM".into()),
            _ => node.data.input_data_type.map(|kind| format!("{kind:?}").to_uppercase()).unwrap_or_else(|| "SOURCE".into()),
        },
        NodeType::Constant => "COLOR".into(), NodeType::Math => "MATH".into(), NodeType::Renderer => "RENDERER".into(),
    }
}

fn node_status(node: &ProjectNode, connected: &HashSet<(&str, &str)>) -> &'static str {
    if node.data.extra.get("parseError").is_some_and(|error| !error.is_null() && error.as_str() != Some("")) { return "error"; }
    match node.node_type {
        NodeType::Input if matches!(node.data.input_mode, Some(InputMode::Image | InputMode::Video | InputMode::Framebuffer)) => {
            if node.data.image_data_url.is_some() || node.data.raw_data_url.is_some() || node.data.video_file_path.is_some() || node.data.video_device_id.is_some() { "ready" } else { "not-ready" }
        }
        NodeType::Onnx => "not-ready",
        _ if node.data.inputs.iter().any(|port| !connected.contains(&(node.id.as_str(), port.id.as_str()))) => "not-ready",
        _ => "ready",
    }
}

fn project_port(
    port: &Port,
    index: usize,
    output: bool,
    node: &ProjectNode,
    connected: bool,
) -> GraphPort {
    let data_type = match port.data_type {
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
    };
    GraphPort {
        port_id: port.id.as_str().into(),
        label: port.label.as_str().into(),
        data_type: data_type.into(),
        connected,
        y: port_y(index, node, output),
        output,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use open_quartz_sdk::{
        Environment, GraphCommand, NodeFactoryRequest, NodeId, OpenQuartz, PortId, PortKey,
        Position,
    };
    use slint::Model;

    #[test]
    fn fit_bounds_ignore_world_origin_and_include_backwards_bezier_hull() {
        let sdk = OpenQuartz::new(Environment::headless());
        let mut project = sdk.create_project("Backwards connection");
        let mut nodes = Vec::new();
        for x in [400.0, 0.0] {
            let (node, _) = project
                .create_graph_node(
                    NodeFactoryRequest::Math {
                        position: Some(Position { x, y: 0.0 }),
                        op: "add".into(),
                    },
                    project.graph_revision(),
                )
                .unwrap();
            nodes.push(node);
        }
        project
            .apply_graph_command(
                GraphCommand::Connect {
                    source: PortKey::new(
                        NodeId::new(&nodes[0].id),
                        PortId::new(&nodes[0].data.outputs[0].id),
                    ),
                    target: PortKey::new(
                        NodeId::new(&nodes[1].id),
                        PortId::new(&nodes[1].data.inputs[1].id),
                    ),
                },
                project.graph_revision(),
            )
            .unwrap();
        let original = project.graph_snapshot();
        let baseline = GraphViewModel::from_project(&project, &BTreeSet::new());
        for (dx, dy) in [(12000.0, 8000.0), (-12000.0, -8000.0)] {
            let mut graph = original.clone();
            for node in &mut graph.nodes {
                node.position.x += dx;
                node.position.y += dy;
            }
            project
                .replace_graph(graph, project.graph_revision())
                .unwrap();
            let projection = GraphViewModel::from_project(&project, &BTreeSet::new());
            assert_eq!(projection.content_width, baseline.content_width);
            assert_eq!(projection.content_height, baseline.content_height);
            for index in 0..projection.nodes.row_count() {
                let node = projection.nodes.row_data(index).unwrap();
                let before = baseline.nodes.row_data(index).unwrap();
                assert_eq!(node.x + projection.origin_x, before.x + baseline.origin_x);
                assert_eq!(node.y + projection.origin_y, before.y + baseline.origin_y);
                assert!(node.y + projection.origin_y + node.height < projection.content_height);
            }
            let edge = projection.edges.row_data(0).unwrap();
            assert!(edge.target_x - 48.0 + projection.origin_x >= CONTENT_PADDING);
            assert!(
                edge.source_x + 48.0 + projection.origin_x
                    <= projection.content_width - CONTENT_PADDING
            );
        }
    }
}
