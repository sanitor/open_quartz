use std::collections::{HashMap, HashSet, VecDeque};

use super::GraphEdge;

/// Tracks nodes that must execute, propagating invalidation downstream.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DirtySet {
    dirty: HashSet<String>,
    dependents: HashMap<String, Vec<String>>,
}

impl DirtySet {
    pub fn new(edges: &[GraphEdge]) -> Self {
        let mut result = Self::default();
        result.set_edges(edges);
        result
    }

    pub fn set_edges(&mut self, edges: &[GraphEdge]) {
        self.dependents.clear();
        for edge in edges {
            self.dependents
                .entry(edge.source.clone())
                .or_default()
                .push(edge.target.clone());
        }
    }

    pub fn mark_dirty(&mut self, node_id: &str) {
        let mut queue = VecDeque::from([node_id.to_owned()]);
        while let Some(current) = queue.pop_front() {
            if !self.dirty.insert(current.clone()) {
                continue;
            }
            if let Some(children) = self.dependents.get(&current) {
                queue.extend(children.iter().cloned());
            }
        }
    }

    pub fn mark_all<I>(&mut self, node_ids: I)
    where
        I: IntoIterator<Item = String>,
    {
        for node_id in node_ids {
            self.dirty.insert(node_id);
        }
    }

    pub fn contains(&self, node_id: &str) -> bool {
        self.dirty.contains(node_id)
    }

    pub fn is_empty(&self) -> bool {
        self.dirty.is_empty()
    }

    pub fn len(&self) -> usize {
        self.dirty.len()
    }

    /// Returns dirty nodes in cached topological order, then clears them.
    pub fn take_in_order(&mut self, topo_order: &[String]) -> Vec<String> {
        let result = topo_order
            .iter()
            .filter(|node_id| self.dirty.contains(node_id.as_str()))
            .cloned()
            .collect();
        self.dirty.clear();
        result
    }
}
