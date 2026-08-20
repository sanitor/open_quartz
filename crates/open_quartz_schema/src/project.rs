use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::error::{SdkError, SdkErrorCode};

use super::{
    DataType, InputMode, NodeData, NodeId, NodeType, Port, PortDirection, PortKey, SystemSource,
};

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

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Graph {
    pub nodes: Vec<ProjectNode>,
    pub edges: Vec<Edge>,
}

/// Typed domain commands accepted by the graph aggregate.
///
/// The command payload is deliberately framework-neutral. React Flow (or any
/// other UI toolkit) is responsible for projecting its own change objects into
/// these commands; the graph owns validation and mutation semantics.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum GraphCommand {
    AddNode { node: ProjectNode },
    UpdateNode { node: ProjectNode },
    UpdateNodeData { node_id: NodeId, data: NodeData },
    UpdateShaderCode { node_id: NodeId, shader_code: String },
    UpdateInputType {
        node_id: NodeId,
        data_type: DataType,
        input_mode: Option<InputMode>,
    },
    UpdateNodePorts {
        node_id: NodeId,
        inputs: Vec<super::Port>,
        outputs: Vec<super::Port>,
    },
    SetNodePosition { node_id: NodeId, position: Position },
    RemoveNode { node_id: NodeId },
    Connect { source: PortKey, target: PortKey },
    Disconnect { edge_id: String },
}

/// Inputs to the Rust-owned executable node factory.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum NodeFactoryRequest {
    Shader {
        #[serde(default)]
        position: Option<Position>,
        code: String,
        label: String,
        #[serde(default, alias = "templateName")]
        template_name: Option<String>,
        #[serde(default, alias = "shaderTemplateId")]
        shader_template_id: Option<String>,
    },
    Input {
        #[serde(default)]
        position: Option<Position>,
        #[serde(alias = "dataType")]
        data_type: DataType,
        #[serde(default, alias = "inputMode")]
        input_mode: Option<InputMode>,
    },
    System {
        #[serde(default)]
        position: Option<Position>,
        source: SystemSource,
    },
    Constant {
        #[serde(default)]
        position: Option<Position>,
    },
    Math {
        #[serde(default)]
        position: Option<Position>,
        op: String,
    },
    Renderer {
        #[serde(default)]
        position: Option<Position>,
    },
    Onnx {
        #[serde(default)]
        position: Option<Position>,
        label: String,
        #[serde(default, alias = "templateName")]
        template_name: Option<String>,
        #[serde(default, alias = "modelId")]
        model_id: Option<String>,
        #[serde(default, alias = "catalogId")]
        catalog_id: Option<String>,
        #[serde(default)]
        inputs: Vec<Port>,
        #[serde(default)]
        outputs: Vec<Port>,
    },
    CustomOnnx {
        #[serde(default)]
        position: Option<Position>,
    },
}

impl Graph {
    pub fn nodes(&self) -> &[ProjectNode] {
        &self.nodes
    }

    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    pub fn validate(&self) -> Result<(), SdkError> {
        let mut node_ids = BTreeSet::new();
        for node in &self.nodes {
            if !node_ids.insert(node.id.as_str()) {
                return Err(SdkError::new(
                    SdkErrorCode::InvalidGraph,
                    "Graph contains duplicate node IDs",
                )
                .for_node(&node.id));
            }
        }

        let mut edge_ids = BTreeSet::new();
        for edge in &self.edges {
            if !edge_ids.insert(edge.id.as_str()) {
                return Err(SdkError::new(
                    SdkErrorCode::InvalidGraph,
                    "Graph contains duplicate edge IDs",
                )
                .with_details(edge.id.clone()));
            }
            let source_node = self
                .nodes
                .iter()
                .find(|node| node.id == edge.source)
                .ok_or_else(|| {
                    SdkError::new(SdkErrorCode::UnknownNode, "Source node does not exist")
                        .for_node(edge.source.clone())
                })?;
            let source_port = source_node
                .data
                .outputs
                .iter()
                .find(|port| port.id == edge.source_handle)
                .filter(|port| port.direction == PortDirection::Output)
                .ok_or_else(|| {
                    SdkError::new(
                        SdkErrorCode::InvalidGraph,
                        "Source output port does not exist",
                    )
                    .for_node(edge.source.clone())
                })?;
            let target_node = self
                .nodes
                .iter()
                .find(|node| node.id == edge.target)
                .ok_or_else(|| {
                    SdkError::new(SdkErrorCode::UnknownNode, "Target node does not exist")
                        .for_node(edge.target.clone())
                })?;
            let target_port = target_node
                .data
                .inputs
                .iter()
                .find(|port| port.id == edge.target_handle)
                .filter(|port| port.direction == PortDirection::Input)
                .ok_or_else(|| {
                    SdkError::new(
                        SdkErrorCode::InvalidGraph,
                        "Target input port does not exist",
                    )
                    .for_node(edge.target.clone())
                })?;
            if !compatible_types(
                source_port.data_type,
                target_port.data_type,
                source_node.data.node_type,
            ) {
                return Err(SdkError::new(
                    SdkErrorCode::InvalidGraph,
                    "Connected ports have incompatible data types",
                )
                .for_node(edge.target.clone()));
            }
            if self.edges.iter().any(|candidate| {
                candidate.id != edge.id
                    && candidate.target == edge.target
                    && candidate.target_handle == edge.target_handle
            }) {
                return Err(SdkError::new(
                    SdkErrorCode::InvalidGraph,
                    "Graph input port has multiple connections",
                )
                .for_node(edge.target.clone()));
            }
            if has_path(&self.edges, &edge.target, &edge.source) {
                return Err(SdkError::new(
                    SdkErrorCode::InvalidGraph,
                    "Graph contains a cycle",
                )
                .for_node(edge.target.clone()));
            }
        }
        Ok(())
    }
    pub fn upstream_subgraph(&self, output: &NodeId) -> Result<Self, SdkError> {
        if !self.nodes.iter().any(|node| node.id == output.as_str()) {
            return Err(
                SdkError::new(SdkErrorCode::UnknownNode, "Output node does not exist")
                    .for_node(output.as_str()),
            );
        }
        let mut keep = BTreeSet::from([output.as_str()]);
        let mut pending = vec![output.as_str()];
        while let Some(target) = pending.pop() {
            for edge in self.edges.iter().filter(|edge| edge.target == target) {
                if keep.insert(edge.source.as_str()) {
                    pending.push(edge.source.as_str());
                }
            }
        }
        Ok(Self {
            nodes: self
                .nodes
                .iter()
                .filter(|node| keep.contains(node.id.as_str()))
                .cloned()
                .collect(),
            edges: self
                .edges
                .iter()
                .filter(|edge| {
                    keep.contains(edge.source.as_str()) && keep.contains(edge.target.as_str())
                })
                .cloned()
                .collect(),
        })
    }

    pub fn edit(
        &mut self,
        operation: impl FnOnce(&mut GraphEdit<'_>) -> Result<(), SdkError>,
    ) -> Result<GraphChange, SdkError> {
        let mut draft = self.clone();
        let mut changed_nodes = BTreeSet::new();
        {
            let mut edit = GraphEdit {
                graph: &mut draft,
                changed_nodes: &mut changed_nodes,
            };
            operation(&mut edit)?;
        }
        draft.validate()?;
        *self = draft;
        Ok(GraphChange {
            changed_nodes: changed_nodes.into_iter().map(NodeId::new).collect(),
        })
    }

    /// Apply one typed graph command atomically.
    pub fn apply_command(&mut self, command: GraphCommand) -> Result<GraphChange, SdkError> {
        self.edit(|edit| match command {
            GraphCommand::AddNode { node } => edit.add_node(node),
            GraphCommand::UpdateNode { node } => {
                let node_id = NodeId::new(node.id.clone());
                edit.update_node(node_id, |current| {
                    *current = node;
                    Ok(())
                })
            }
            GraphCommand::UpdateNodeData { node_id, data } => {
                edit.update_node(node_id, |node| {
                    node.data = data;
                    Ok(())
                })
            }
            GraphCommand::UpdateShaderCode { .. } => Err(SdkError::new(
                SdkErrorCode::InvalidState,
                "Shader code updates must be applied through the project aggregate",
            )),
            GraphCommand::UpdateInputType { .. } => Err(SdkError::new(
                SdkErrorCode::InvalidState,
                "Input type updates must be applied through the project aggregate",
            )),
            GraphCommand::UpdateNodePorts {
                node_id,
                inputs,
                outputs,
            } => edit.update_node(node_id, |node| {
                node.data.inputs = inputs;
                node.data.outputs = outputs;
                Ok(())
            }),
            GraphCommand::SetNodePosition { node_id, position } => {
                edit.update_node(node_id, |node| {
                    node.position = position;
                    Ok(())
                })
            }
            GraphCommand::RemoveNode { node_id } => edit.remove_node(&node_id).map(|_| ()),
            GraphCommand::Connect { source, target } => edit.connect(source, target),
            GraphCommand::Disconnect { edge_id } => edit.disconnect(&edge_id).map(|_| ()),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GraphChange {
    changed_nodes: Vec<NodeId>,
}

impl GraphChange {
    pub fn from_changed_nodes(changed_nodes: impl IntoIterator<Item = NodeId>) -> Self {
        Self {
            changed_nodes: changed_nodes.into_iter().collect(),
        }
    }

    pub fn changed_nodes(&self) -> &[NodeId] {
        &self.changed_nodes
    }

    pub fn is_empty(&self) -> bool {
        self.changed_nodes.is_empty()
    }
}

pub struct GraphEdit<'graph> {
    graph: &'graph mut Graph,
    changed_nodes: &'graph mut BTreeSet<String>,
}

impl GraphEdit<'_> {
    pub fn add_node(&mut self, node: ProjectNode) -> Result<(), SdkError> {
        if self
            .graph
            .nodes
            .iter()
            .any(|candidate| candidate.id == node.id)
        {
            return Err(SdkError::new(
                SdkErrorCode::InvalidGraph,
                "Graph already contains the node ID",
            )
            .for_node(node.id));
        }
        self.changed_nodes.insert(node.id.clone());
        self.graph.nodes.push(node);
        Ok(())
    }

    pub fn update_node(
        &mut self,
        node_id: NodeId,
        operation: impl FnOnce(&mut ProjectNode) -> Result<(), SdkError>,
    ) -> Result<(), SdkError> {
        let node = self
            .graph
            .nodes
            .iter_mut()
            .find(|node| node.id == node_id.as_str())
            .ok_or_else(|| {
                SdkError::new(SdkErrorCode::UnknownNode, "Graph node does not exist")
                    .for_node(node_id.as_str())
            })?;
        operation(node)?;
        self.changed_nodes.insert(node_id.into_string());
        Ok(())
    }

    pub fn remove_node(&mut self, node_id: &NodeId) -> Result<ProjectNode, SdkError> {
        let index = self
            .graph
            .nodes
            .iter()
            .position(|node| node.id == node_id.as_str())
            .ok_or_else(|| {
                SdkError::new(SdkErrorCode::UnknownNode, "Graph node does not exist")
                    .for_node(node_id.as_str())
            })?;
        let node = self.graph.nodes.remove(index);
        let mut affected = self
            .graph
            .edges
            .iter()
            .filter(|edge| edge.source == node.id || edge.target == node.id)
            .flat_map(|edge| [edge.source.clone(), edge.target.clone()])
            .collect::<Vec<_>>();
        self.graph
            .edges
            .retain(|edge| edge.source != node.id && edge.target != node.id);
        affected.push(node.id.clone());
        self.changed_nodes.extend(affected);
        Ok(node)
    }

    pub fn connect(&mut self, source: PortKey, target: PortKey) -> Result<(), SdkError> {
        let source_node = self
            .graph
            .nodes
            .iter()
            .find(|node| node.id == source.node_id().as_str())
            .ok_or_else(|| {
                SdkError::new(SdkErrorCode::UnknownNode, "Source node does not exist")
                    .for_node(source.node_id().as_str())
            })?;
        let source_port = source_node
            .data
            .outputs
            .iter()
            .find(|port| port.id == source.port_id().as_str())
            .filter(|port| port.direction == PortDirection::Output)
            .ok_or_else(|| {
                SdkError::new(
                    SdkErrorCode::InvalidGraph,
                    "Source output port does not exist",
                )
                .for_node(source.node_id().as_str())
            })?;
        let target_node = self
            .graph
            .nodes
            .iter()
            .find(|node| node.id == target.node_id().as_str())
            .ok_or_else(|| {
                SdkError::new(SdkErrorCode::UnknownNode, "Target node does not exist")
                    .for_node(target.node_id().as_str())
            })?;
        let target_port = target_node
            .data
            .inputs
            .iter()
            .find(|port| port.id == target.port_id().as_str())
            .filter(|port| port.direction == PortDirection::Input)
            .ok_or_else(|| {
                SdkError::new(
                    SdkErrorCode::InvalidGraph,
                    "Target input port does not exist",
                )
                .for_node(target.node_id().as_str())
            })?;
        if !compatible_types(
            source_port.data_type,
            target_port.data_type,
            source_node.data.node_type,
        ) {
            return Err(SdkError::new(
                SdkErrorCode::InvalidGraph,
                "Connected ports have incompatible data types",
            )
            .for_node(target.node_id().as_str()));
        }

        self.graph.edges.retain(|edge| {
            edge.target != target.node_id().as_str()
                || edge.target_handle != target.port_id().as_str()
        });
        let edge = Edge {
            id: format!(
                "{}:{}->{}:{}",
                source.node_id(),
                source.port_id(),
                target.node_id(),
                target.port_id()
            ),
            source: source.node_id().to_string(),
            source_handle: source.port_id().to_string(),
            target: target.node_id().to_string(),
            target_handle: target.port_id().to_string(),
        };
        self.graph.edges.push(edge);
        if has_path(
            &self.graph.edges,
            target.node_id().as_str(),
            source.node_id().as_str(),
        ) {
            return Err(SdkError::new(
                SdkErrorCode::InvalidGraph,
                "Connection would create a graph cycle",
            ));
        }
        self.changed_nodes.insert(source.node_id().to_string());
        self.changed_nodes.insert(target.node_id().to_string());
        Ok(())
    }

    pub fn disconnect(&mut self, edge_id: &str) -> Result<Edge, SdkError> {
        let index = self
            .graph
            .edges
            .iter()
            .position(|edge| edge.id == edge_id)
            .ok_or_else(|| {
                SdkError::new(SdkErrorCode::InvalidGraph, "Graph edge does not exist")
            })?;
        let edge = self.graph.edges.remove(index);
        self.changed_nodes.insert(edge.source.clone());
        self.changed_nodes.insert(edge.target.clone());
        Ok(edge)
    }
}

fn compatible_types(source: DataType, target: DataType, source_node_type: NodeType) -> bool {
    if source == target {
        return true;
    }
    if source == DataType::Auto || target == DataType::Auto {
        let other = if source == DataType::Auto { target } else { source };
        return !matches!(
            other,
            DataType::Sampler2d
                | DataType::SamplerCube
                | DataType::Roi
                | DataType::Mesh
                | DataType::Json
        );
    }
    if matches!(target, DataType::Sampler2d | DataType::SamplerCube) {
        return matches!(source, DataType::Sampler2d | DataType::SamplerCube)
            || matches!(
                source_node_type,
                NodeType::Shader | NodeType::Constant | NodeType::Onnx
            );
    }
    false
}

fn has_path(edges: &[Edge], start: &str, goal: &str) -> bool {
    let mut pending = vec![start];
    let mut visited = BTreeSet::new();
    while let Some(node) = pending.pop() {
        if node == goal {
            return true;
        }
        if !visited.insert(node) {
            continue;
        }
        pending.extend(
            edges
                .iter()
                .filter(|edge| edge.source == node)
                .map(|edge| edge.target.as_str()),
        );
    }
    false
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
