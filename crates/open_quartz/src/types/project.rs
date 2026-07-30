use serde::{Deserialize, Serialize};

use super::{NodeData, NodeType};

pub const PROJECT_FILE_VERSION: &str = "0.4.0";

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectNode {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: NodeType,
    pub position: Position,
    pub data: NodeData,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Edge {
    pub id: String,
    pub source: String,
    pub source_handle: String,
    pub target: String,
    pub target_handle: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Graph {
    pub nodes: Vec<ProjectNode>,
    pub edges: Vec<Edge>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectFile {
    pub version: String,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    pub graph: Graph,
}
