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
const MIN_CONTENT_WIDTH: f32 = 640.0;
const MIN_CONTENT_HEIGHT: f32 = 480.0;

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
        let (min_x, min_y, max_x, max_y) = graph.nodes().iter().fold(
            (0.0_f32, 0.0_f32, 0.0_f32, 0.0_f32),
            |(min_x, min_y, max_x, max_y), node| {
                let x = node.position.x as f32;
                let y = node.position.y as f32;
                let height = node_height(node.data.inputs.len().max(node.data.outputs.len()));
                (
                    min_x.min(x),
                    min_y.min(y),
                    max_x.max(x),
                    max_y.max(y + height),
                )
            },
        );
        let origin_x = CONTENT_PADDING - min_x;
        let origin_y = CONTENT_PADDING - min_y;
        let content_width =
            (max_x + origin_x + NODE_WIDTH + CONTENT_PADDING).max(MIN_CONTENT_WIDTH);
        let content_height = (max_y + origin_y + CONTENT_PADDING).max(MIN_CONTENT_HEIGHT);
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
