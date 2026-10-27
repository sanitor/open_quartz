use std::collections::HashMap;
use std::time::{Duration, Instant};

use open_quartz_sdk::{Port, Project};
use slint::{ModelRc, VecModel};

use crate::{GraphEdge, GraphNode, GraphPort};

const NODE_WIDTH: f32 = 184.0;
const NODE_HEIGHT: f32 = 92.0;
const PORT_TOP: f32 = 72.0;
const PORT_SPACING: f32 = 24.0;
const CONTENT_PADDING: f32 = 24.0;

pub struct GraphViewModel {
    pub nodes: ModelRc<GraphNode>,
    pub edges: ModelRc<GraphEdge>,
    pub summary: String,
    pub content_width: f32,
    pub content_height: f32,
    pub origin_x: f32,
    pub origin_y: f32,
    pub node_count: usize,
    pub edge_count: usize,
    pub projection_duration: Duration,
}

impl GraphViewModel {
    pub fn from_project(project: &Project, selected_node_id: Option<&str>) -> Self {
        let started = Instant::now();
        let graph = project.graph();
        let node_indices = graph
            .nodes()
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id.as_str(), index))
            .collect::<HashMap<_, _>>();
        // Tight graph-space bounds keep Fit View independent of the world's origin.
        let bounds = graph
            .nodes()
            .iter()
            .map(|node| {
                let x = node.position.x as f32;
                let y = node.position.y as f32;
                let height = node_height(node.data.inputs.len().max(node.data.outputs.len()));
                (x, y, x + NODE_WIDTH, y + height)
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
                max_x = max_x.max(graph.nodes()[source].position.x as f32 + NODE_WIDTH + 48.0);
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
                x: node.position.x as f32,
                y: node.position.y as f32,
                width: NODE_WIDTH,
                height: node_height(node.data.inputs.len().max(node.data.outputs.len())),
                ports: ModelRc::new(VecModel::from(
                    node.data
                        .inputs
                        .iter()
                        .enumerate()
                        .map(|(index, port)| project_port(port, index, false))
                        .chain(
                            node.data
                                .outputs
                                .iter()
                                .enumerate()
                                .map(|(index, port)| project_port(port, index, true)),
                        )
                        .collect::<Vec<_>>(),
                )),
                selected: selected_node_id == Some(node.id.as_str()),
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
                    source_x: source.position.x as f32 + NODE_WIDTH,
                    source_y: source.position.y as f32
                        + port_y(
                            source
                                .data
                                .outputs
                                .iter()
                                .position(|port| port.id == edge.source_handle)?,
                        ),
                    target_x: target.position.x as f32,
                    target_y: target.position.y as f32
                        + port_y(
                            target
                                .data
                                .inputs
                                .iter()
                                .position(|port| port.id == edge.target_handle)?,
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
            summary: format!(
                "{node_count} nodes · {edge_count} connections · projected in {} µs",
                projection_duration.as_micros()
            ),
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

fn port_y(index: usize) -> f32 {
    PORT_TOP + index as f32 * PORT_SPACING
}

fn node_height(port_count: usize) -> f32 {
    (PORT_TOP + port_count.saturating_sub(1) as f32 * PORT_SPACING + 20.0).max(NODE_HEIGHT)
}

fn project_port(port: &Port, index: usize, output: bool) -> GraphPort {
    GraphPort {
        port_id: port.id.as_str().into(),
        label: format!("{} · {:?}", port.label, port.data_type).into(),
        y: port_y(index),
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
        let baseline = GraphViewModel::from_project(&project, None);
        for (dx, dy) in [(12000.0, 8000.0), (-12000.0, -8000.0)] {
            let mut graph = original.clone();
            for node in &mut graph.nodes {
                node.position.x += dx;
                node.position.y += dy;
            }
            project
                .replace_graph(graph, project.graph_revision())
                .unwrap();
            let projection = GraphViewModel::from_project(&project, None);
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
