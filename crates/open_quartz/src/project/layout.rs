use std::collections::BTreeMap;

use super::{Graph, NodeId, Position, Project, SdkError, SdkErrorCode};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GraphLayout {
    positions: BTreeMap<NodeId, Position>,
}

impl GraphLayout {
    pub(super) fn from_graph(graph: &Graph) -> Self {
        Self {
            positions: graph
                .nodes()
                .iter()
                .map(|node| (NodeId::new(&node.id), node.position))
                .collect(),
        }
    }

    fn reconcile(&mut self, graph: &Graph, previous: &Graph) {
        self.positions.retain(|node_id, _| {
            graph
                .nodes()
                .iter()
                .any(|node| node.id == node_id.as_str())
        });
        for node in graph.nodes() {
            let changed = previous
                .nodes()
                .iter()
                .find(|candidate| candidate.id == node.id)
                .map(|candidate| candidate.position != node.position)
                .unwrap_or(true);
            if changed || !self.positions.contains_key(&NodeId::new(&node.id)) {
                self.positions
                    .insert(NodeId::new(&node.id), node.position);
            }
        }
    }

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

impl Project {
    pub(super) fn reconcile_layout(&mut self, previous: &Graph) {
        self.layout.reconcile(&self.graph, previous);
    }
}

#[cfg(test)]
#[path = "layout_coverage_tests.rs"]
mod coverage_tests;
