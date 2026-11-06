use super::*;
use super::node_factory::{input_shader_code, remap_ports_preserving_ids};
use serde_json::Value;

impl Project {
    pub fn replace_graph(
        &mut self,
        graph: Graph,
        expected_revision: u32,
    ) -> Result<GraphChange, SdkError> {
        self.ensure_graph_revision(expected_revision)?;
        graph.validate()?;
        let previous = self.graph.clone();
        let changed_nodes = changed_node_ids(&self.graph, &graph);
        self.graph_history.push(self.graph.clone());
        self.graph = graph;
        self.graph_redo.clear();
        self.reconcile_layout(&previous);
        self.graph_revision = self.graph_revision.saturating_add(1);
        Ok(GraphChange::from_changed_nodes(changed_nodes))
    }

    /// Apply one typed graph command through the revisioned aggregate.
    pub fn apply_graph_command(
        &mut self,
        command: GraphCommand,
        expected_revision: u32,
    ) -> Result<GraphChange, SdkError> {
        self.ensure_graph_revision(expected_revision)?;
        let previous = self.graph.clone();
        let mut next = previous.clone();
        let change = match command {
            GraphCommand::UpdateShaderCode {
                node_id,
                shader_code,
            } => {
                let node = next
                    .nodes()
                    .iter()
                    .find(|node| node.id == node_id.as_str())
                    .ok_or_else(|| {
                        SdkError::new(SdkErrorCode::UnknownNode, "Graph node does not exist")
                            .for_node(node_id.as_str())
                    })?
                    .clone();
                let parsed = crate::compiler::shader::parse_shader(&shader_code);
                let inputs = remap_ports_preserving_ids(&node.data.inputs, &node.id, parsed.inputs);
                let outputs =
                    remap_ports_preserving_ids(&node.data.outputs, &node.id, parsed.outputs);
                let mut data = node.data;
                data.shader_code = shader_code;
                data.inputs = inputs;
                data.outputs = outputs;
                if let Some(error) = parsed.parse_error {
                    data.extra
                        .insert("parseError".to_owned(), Value::String(error));
                } else {
                    data.extra.remove("parseError");
                }
                next.apply_command(GraphCommand::UpdateNodeData { node_id, data })?
            }
            GraphCommand::UpdateInputType {
                node_id,
                data_type,
                input_mode,
            } => {
                let node = next
                    .nodes()
                    .iter()
                    .find(|node| node.id == node_id.as_str())
                    .ok_or_else(|| {
                        SdkError::new(SdkErrorCode::UnknownNode, "Graph node does not exist")
                            .for_node(node_id.as_str())
                    })?
                    .clone();
                if node.node_type != NodeType::Input {
                    return Err(SdkError::new(
                        SdkErrorCode::InvalidGraph,
                        "Only input nodes support input type updates",
                    )
                    .for_node(node_id.as_str()));
                }
                let shader_code = input_shader_code(data_type);
                let parsed = crate::compiler::shader::parse_shader(&shader_code);
                let inputs = remap_ports_preserving_ids(&node.data.inputs, &node.id, parsed.inputs);
                let outputs =
                    remap_ports_preserving_ids(&node.data.outputs, &node.id, parsed.outputs)
                        .into_iter()
                        .map(|mut port| {
                            port.data_type = data_type;
                            port
                        })
                        .collect();
                let mut data = node.data;
                data.shader_code = shader_code;
                data.input_data_type = Some(data_type);
                data.input_mode = if data_type == DataType::Sampler2d {
                    Some(input_mode.unwrap_or(InputMode::Image))
                } else {
                    input_mode
                };
                data.uniforms = serde_json::Map::new();
                data.inputs = inputs;
                data.outputs = outputs;
                next.apply_command(GraphCommand::UpdateNodeData { node_id, data })?
            }
            other => next.apply_command(other)?,
        };
        self.graph_history.push(previous.clone());
        self.graph = next;
        self.graph_redo.clear();
        self.reconcile_layout(&previous);
        self.graph_revision = self.graph_revision.saturating_add(1);
        Ok(change)
    }

    pub fn initialize_graph(&mut self, graph: Graph) -> Result<(), SdkError> {
        if self.graph_revision != 0
            || !self.graph.nodes().is_empty()
            || !self.graph_history.is_empty()
        {
            return Err(SdkError::new(
                SdkErrorCode::InvalidState,
                "Graph has already been initialized",
            ));
        }
        graph.validate()?;
        self.graph = graph;
        self.graph_history.clear();
        self.graph_redo.clear();
        self.reconcile_layout(&Graph::default());
        Ok(())
    }

    pub fn rollback_graph(&mut self, expected_revision: u32) -> Result<GraphChange, SdkError> {
        self.ensure_graph_revision(expected_revision)?;
        let previous = self.graph_history.pop().ok_or_else(|| {
            SdkError::new(
                SdkErrorCode::InvalidState,
                "Graph has no revision to roll back",
            )
        })?;
        let changed_nodes = changed_node_ids(&self.graph, &previous);
        let current = self.graph.clone();
        self.graph_redo.push(current.clone());
        self.graph = previous;
        self.reconcile_layout(&current);
        self.graph_revision = self.graph_revision.saturating_add(1);
        Ok(GraphChange::from_changed_nodes(changed_nodes))
    }

    pub fn redo_graph(&mut self, expected_revision: u32) -> Result<GraphChange, SdkError> {
        self.ensure_graph_revision(expected_revision)?;
        let next = self.graph_redo.pop().ok_or_else(|| {
            SdkError::new(
                SdkErrorCode::InvalidState,
                "Graph has no revision to reapply",
            )
        })?;
        let previous = self.graph.clone();
        let changed_nodes = changed_node_ids(&previous, &next);
        self.graph_history.push(previous.clone());
        self.graph = next;
        self.reconcile_layout(&previous);
        self.graph_revision = self.graph_revision.saturating_add(1);
        Ok(GraphChange::from_changed_nodes(changed_nodes))
    }

    pub(super) fn ensure_graph_revision(&self, expected_revision: u32) -> Result<(), SdkError> {
        if expected_revision == self.graph_revision {
            return Ok(());
        }
        Err(
            SdkError::new(SdkErrorCode::StaleRevision, "Graph revision is stale").with_details(
                format!(
                    "expected revision {expected_revision}, current revision {}",
                    self.graph_revision
                ),
            ),
        )
    }
}

fn changed_node_ids(before: &Graph, after: &Graph) -> Vec<NodeId> {
    let mut changed = std::collections::BTreeSet::new();
    let node_ids = before
        .nodes()
        .iter()
        .chain(after.nodes())
        .map(|node| node.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for node_id in node_ids {
        let previous = before.nodes().iter().find(|node| node.id == node_id);
        let current = after.nodes().iter().find(|node| node.id == node_id);
        if previous != current {
            changed.insert(node_id.to_owned());
        }
    }
    for edge in before.edges() {
        if !after.edges().contains(edge) {
            changed.insert(edge.source.clone());
            changed.insert(edge.target.clone());
        }
    }
    for edge in after.edges() {
        if !before.edges().contains(edge) {
            changed.insert(edge.source.clone());
            changed.insert(edge.target.clone());
        }
    }
    changed.into_iter().map(NodeId::new).collect()
}

#[cfg(test)]
#[path = "history_coverage_tests.rs"]
mod coverage_tests;
