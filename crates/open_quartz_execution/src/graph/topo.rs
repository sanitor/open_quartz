use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
}

/// Kahn topological sort matching the existing TypeScript executor behavior:
/// unknown edge endpoints are ignored and cycles return the reachable partial order.
pub fn topological_sort(nodes: &[GraphNode], edges: &[GraphEdge]) -> Vec<String> {
    let mut adjacency: HashMap<&str, Vec<&str>> = nodes
        .iter()
        .map(|node| (node.id.as_str(), Vec::new()))
        .collect();
    let mut indegree: HashMap<&str, usize> =
        nodes.iter().map(|node| (node.id.as_str(), 0)).collect();

    for edge in edges {
        let Some(targets) = adjacency.get_mut(edge.source.as_str()) else {
            continue;
        };
        targets.push(edge.target.as_str());
        if let Some(degree) = indegree.get_mut(edge.target.as_str()) {
            *degree += 1;
        }
    }

    let mut queue = VecDeque::new();
    for node in nodes {
        if indegree.get(node.id.as_str()) == Some(&0) {
            queue.push_back(node.id.as_str());
        }
    }

    let mut order = Vec::with_capacity(nodes.len());
    while let Some(source) = queue.pop_front() {
        order.push(source.to_owned());
        for target in adjacency.get(source).into_iter().flatten() {
            let Some(degree) = indegree.get_mut(target) else {
                continue;
            };
            *degree -= 1;
            if *degree == 0 {
                queue.push_back(target);
            }
        }
    }
    order
}
