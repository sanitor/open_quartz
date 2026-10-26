use std::collections::HashMap;
use std::fmt::Write;

use open_quartz_sdk::Project;
use slint::{ModelRc, VecModel};

use crate::GraphNode;

pub struct GraphViewModel {
    pub nodes: ModelRc<GraphNode>,
    pub summary: String,
}

impl GraphViewModel {
    pub fn from_project(project: &Project) -> Self {
        let graph = project.graph();
        let mut connections: HashMap<&str, (usize, String)> = HashMap::new();
        for edge in graph.edges() {
            connections.entry(&edge.target).or_default().0 += 1;
            let outgoing = &mut connections.entry(&edge.source).or_default().1;
            if !outgoing.is_empty() {
                outgoing.push('\n');
            }
            write!(
                outgoing,
                "{} → {}.{}",
                edge.source_handle, edge.target, edge.target_handle
            )
            .expect("Writing to a String cannot fail");
        }
        let nodes = graph
            .nodes()
            .iter()
            .map(|node| {
                let inputs = node
                    .data
                    .inputs
                    .iter()
                    .map(|port| port.label.as_str())
                    .collect::<Vec<_>>();
                let outputs = node
                    .data
                    .outputs
                    .iter()
                    .map(|port| port.label.as_str())
                    .collect::<Vec<_>>();
                let (incoming, outgoing) = connections
                    .get(node.id.as_str())
                    .map(|(count, description)| (*count, description.as_str()))
                    .unwrap_or((0, ""));
                GraphNode {
                    node_id: node.id.as_str().into(),
                    label: node.data.label.as_str().into(),
                    kind: format!("{:?}", node.node_type).into(),
                    position: format!("({}, {})", node.position.x, node.position.y).into(),
                    ports: format!(
                        "Inputs: {}\nOutputs: {}",
                        inputs.join(", "),
                        outputs.join(", ")
                    )
                    .into(),
                    connections: format!("{incoming} incoming connection(s)\n{outgoing}").into(),
                }
            })
            .collect::<Vec<_>>();
        Self {
            nodes: ModelRc::new(VecModel::from(nodes)),
            summary: format!(
                "{} nodes · {} connections",
                graph.nodes().len(),
                graph.edges().len()
            ),
        }
    }
}
