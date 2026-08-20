use serde::{Deserialize, Serialize};

use crate::graph::{topological_sort, GraphEdge, GraphNode};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphRequest {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphPlan {
    pub order: Vec<String>,
    pub cycle: bool,
}

pub fn plan_graph(request: GraphRequest) -> GraphPlan {
    let order = topological_sort(&request.nodes, &request.edges);
    GraphPlan {
        cycle: order.len() < request.nodes.len(),
        order,
    }
}
