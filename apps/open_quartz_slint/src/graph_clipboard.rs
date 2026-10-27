use std::collections::{BTreeSet, HashMap, HashSet};

use open_quartz_sdk::{Graph, Project, SdkError};

/// An owned graph snapshot, independent of later source edits or project opens.
/// Only edges whose endpoints are both selected belong to the clipboard.
pub struct GraphClipboard {
    graph: Graph,
    paste_count: u32,
}

impl GraphClipboard {
    pub fn capture(project: &Project, selected: &BTreeSet<String>) -> Option<Self> {
        let nodes = project
            .graph()
            .nodes()
            .iter()
            .filter(|node| selected.contains(&node.id))
            .cloned()
            .collect::<Vec<_>>();
        if nodes.is_empty() {
            return None;
        }
        let ids = nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<HashSet<_>>();
        let edges = project
            .graph()
            .edges()
            .iter()
            .filter(|edge| ids.contains(edge.source.as_str()) && ids.contains(edge.target.as_str()))
            .cloned()
            .collect();
        Some(Self {
            graph: Graph { nodes, edges },
            paste_count: 0,
        })
    }

    pub fn node_count(&self) -> usize {
        self.graph.nodes.len()
    }

    /// Commit the entire cloned subgraph in one SDK revision/history entry.
    /// Neither the destination nor the paste offset changes on failure.
    pub fn paste(&mut self, project: &mut Project) -> Result<BTreeSet<String>, SdkError> {
        let mut graph = project.graph_snapshot();
        let mut ids = graph
            .nodes
            .iter()
            .map(|node| node.id.clone())
            .collect::<HashSet<_>>();
        let mut node_map = HashMap::with_capacity(self.graph.nodes.len());
        let mut inputs = HashMap::new();
        let mut outputs = HashMap::new();
        let next_paste = self.paste_count.saturating_add(1);
        let offset = f64::from(next_paste) * 40.0;
        let mut selected = BTreeSet::new();
        for original in &self.graph.nodes {
            let mut serial = 1usize;
            let id = loop {
                let id = format!("{}_copy_{serial}", original.id);
                if ids.insert(id.clone()) {
                    break id;
                }
                serial += 1;
            };
            let mut node = original.clone();
            node.id = id.clone();
            node.position.x += offset;
            node.position.y += offset;
            for (index, port) in node.data.inputs.iter_mut().enumerate() {
                port.id = format!("{id}_input_{index}");
                inputs.insert(
                    (
                        original.id.as_str(),
                        original.data.inputs[index].id.as_str(),
                    ),
                    port.id.clone(),
                );
            }
            for (index, port) in node.data.outputs.iter_mut().enumerate() {
                port.id = format!("{id}_output_{index}");
                outputs.insert(
                    (
                        original.id.as_str(),
                        original.data.outputs[index].id.as_str(),
                    ),
                    port.id.clone(),
                );
            }
            selected.insert(id.clone());
            node_map.insert(original.id.as_str(), id);
            graph.nodes.push(node);
        }
        // SDK Connect owns edge identity, compatibility, cycles and occupied-input rules.
        graph.edit(&mut |edit| {
            for edge in &self.graph.edges {
                use open_quartz_sdk::{NodeId, PortId, PortKey};
                edit.connect(
                    PortKey::new(
                        NodeId::new(&node_map[edge.source.as_str()]),
                        PortId::new(&outputs[&(edge.source.as_str(), edge.source_handle.as_str())]),
                    ),
                    PortKey::new(
                        NodeId::new(&node_map[edge.target.as_str()]),
                        PortId::new(&inputs[&(edge.target.as_str(), edge.target_handle.as_str())]),
                    ),
                )?;
            }
            Ok(())
        })?;
        project.replace_graph(graph, project.graph_revision())?;
        self.paste_count = next_paste;
        Ok(selected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use open_quartz_sdk::{
        Environment, GraphCommand, NodeFactoryRequest, NodeId, OpenQuartz, PortId, PortKey,
        Position,
    };
    use serde_json::json;

    fn fixture() -> Project {
        let sdk = OpenQuartz::new(Environment::headless());
        let mut project = sdk.create_project("Clipboard");
        for index in 0..3 {
            let (node, _) = project
                .create_graph_node(
                    NodeFactoryRequest::Math {
                        position: Some(Position {
                            x: index as f64 * 220.0 - 100.0,
                            y: -50.0,
                        }),
                        op: "add".into(),
                    },
                    project.graph_revision(),
                )
                .unwrap();
            let mut data = node.data;
            // Imported projects need not use factory IDs or globally unique port IDs.
            data.inputs[0].id = "custom-input-a".into();
            data.inputs[1].id = "custom-input-b".into();
            data.inputs[1].default_value = Some(json!(3.5));
            data.inputs[1].description = Some("Retain this port metadata".into());
            data.outputs[0].id = "custom-output".into();
            data.uniforms.insert("a".into(), json!(2.0));
            data.extra
                .insert("editorMetadata".into(), json!({"tags": ["original"]}));
            project
                .apply_graph_command(
                    GraphCommand::UpdateNodeData {
                        node_id: NodeId::new(&node.id),
                        data,
                    },
                    project.graph_revision(),
                )
                .unwrap();
        }
        for index in 0..2 {
            let source = &project.graph().nodes()[index];
            let target = &project.graph().nodes()[index + 1];
            let command = GraphCommand::Connect {
                source: PortKey::new(
                    NodeId::new(&source.id),
                    PortId::new(&source.data.outputs[0].id),
                ),
                target: PortKey::new(
                    NodeId::new(&target.id),
                    PortId::new(&target.data.inputs[1].id),
                ),
            };
            project
                .apply_graph_command(command, project.graph_revision())
                .unwrap();
        }
        project
    }

    #[test]
    fn selected_subgraph_paste_remaps_ports_and_is_one_persisted_undoable_edit() {
        let mut project = fixture();
        let before = project.graph_snapshot();
        let selected = before.nodes[..2]
            .iter()
            .map(|node| node.id.clone())
            .collect();
        let mut clipboard = GraphClipboard::capture(&project, &selected).unwrap();
        let revision = project.graph_revision();
        let pasted_ids = clipboard.paste(&mut project).unwrap();
        assert_eq!(project.graph_revision(), revision + 1);
        assert_eq!(&project.graph().nodes()[..3], before.nodes.as_slice());
        assert_eq!(&project.graph().edges()[..2], before.edges.as_slice());
        let pasted = &project.graph().nodes()[3..];
        assert_eq!(
            pasted_ids,
            pasted.iter().map(|node| node.id.clone()).collect()
        );
        for (original, copy) in before.nodes.iter().zip(pasted) {
            assert_ne!(copy.id, original.id);
            assert_eq!(
                copy.position,
                Position {
                    x: original.position.x + 40.0,
                    y: original.position.y + 40.0
                }
            );
            let mut normalized = copy.data.clone();
            for (copy, source) in normalized
                .inputs
                .iter_mut()
                .zip(&original.data.inputs)
                .chain(normalized.outputs.iter_mut().zip(&original.data.outputs))
            {
                assert_ne!(copy.id, source.id);
                copy.id.clone_from(&source.id);
            }
            assert_eq!(normalized, original.data);
        }
        let edge = &project.graph().edges()[2];
        assert_eq!(
            (&edge.source, &edge.source_handle),
            (&pasted[0].id, &pasted[0].data.outputs[0].id)
        );
        assert_eq!(
            (&edge.target, &edge.target_handle),
            (&pasted[1].id, &pasted[1].data.inputs[1].id)
        );
        assert_eq!(project.graph().edges().len(), 3); // The boundary edge to node 3 was not copied.
        let after = project.graph_snapshot();
        let sdk = OpenQuartz::new(Environment::headless());
        let reloaded = sdk
            .open_project_json(&serde_json::to_string(&project.to_file()).unwrap())
            .unwrap();
        assert_eq!(reloaded.graph_snapshot(), after);
        project.rollback_graph(project.graph_revision()).unwrap();
        assert_eq!(project.graph_snapshot(), before);
        project.redo_graph(project.graph_revision()).unwrap();
        assert_eq!(project.graph_snapshot(), after);
    }

    #[test]
    fn clipboard_survives_source_removal_and_pastes_independent_collision_free_copies() {
        let mut source = fixture();
        let original = source.graph().nodes()[0].clone();
        let mut clipboard =
            GraphClipboard::capture(&source, &[original.id.clone()].into()).unwrap();
        source
            .apply_graph_command(
                GraphCommand::RemoveNode {
                    node_id: NodeId::new(&original.id),
                },
                source.graph_revision(),
            )
            .unwrap();
        let sdk = OpenQuartz::new(Environment::headless());
        let mut destination = sdk.create_project("Other project");
        for index in 1..=8 {
            let ids = clipboard.paste(&mut destination).unwrap();
            let copy = destination.graph().nodes().last().unwrap();
            assert!(ids.contains(&copy.id));
            assert_eq!(copy.position.x, original.position.x + index as f64 * 40.0);
            assert_eq!(copy.position.y, original.position.y + index as f64 * 40.0);
            assert_eq!(copy.data.extra, original.data.extra);
            assert_eq!(copy.data.uniforms, original.data.uniforms);
            assert!(destination.graph().edges().is_empty());
            if index == 1 {
                let mut data = copy.data.clone();
                data.extra
                    .insert("editorMetadata".into(), json!({"tags": ["edited"]}));
                destination
                    .apply_graph_command(
                        GraphCommand::UpdateNodeData {
                            node_id: NodeId::new(&copy.id),
                            data,
                        },
                        destination.graph_revision(),
                    )
                    .unwrap();
            }
        }
        let unique = destination
            .graph()
            .nodes()
            .iter()
            .map(|node| &node.id)
            .collect::<HashSet<_>>();
        assert_eq!(unique.len(), 8);
        // A fresh Copy restarts offsets but must not reuse any existing pasted ID.
        let mut second =
            GraphClipboard::capture(&fixture(), &[original.id.clone()].into()).unwrap();
        second.paste(&mut destination).unwrap();
        let unique = destination
            .graph()
            .nodes()
            .iter()
            .map(|node| &node.id)
            .collect::<HashSet<_>>();
        assert_eq!(unique.len(), 9);
        assert_eq!(
            destination.graph().nodes()[8].position.x,
            original.position.x + 40.0
        );
        assert!(GraphClipboard::capture(&source, &BTreeSet::new()).is_none());
    }

    #[test]
    fn rejected_paste_keeps_destination_redo_and_next_offset() {
        let sdk = OpenQuartz::new(Environment::headless());
        let source = fixture();
        let selected = source.graph().nodes()[..2]
            .iter()
            .map(|node| node.id.clone())
            .collect();
        let mut clipboard = GraphClipboard::capture(&source, &selected).unwrap();
        let mut destination = fixture();
        // Edge IDs in imported projects are arbitrary: a valid old edge can have
        // the SDK's generated identity for the would-be pasted connection.
        let collision =
            "math_1_copy_1:math_1_copy_1_output_0->math_2_copy_1:math_2_copy_1_input_1".to_owned();
        let mut graph = destination.graph_snapshot();
        graph.edges[0].id = collision.clone();
        destination
            .replace_graph(graph, destination.graph_revision())
            .unwrap();
        let before = destination.to_file();
        destination
            .apply_graph_command(
                GraphCommand::SetNodePosition {
                    node_id: NodeId::new("math_1"),
                    position: Position { x: 99.0, y: 88.0 },
                },
                destination.graph_revision(),
            )
            .unwrap();
        let redone = destination.graph_snapshot();
        destination
            .rollback_graph(destination.graph_revision())
            .unwrap();
        let revision = destination.graph_revision();
        assert!(clipboard.paste(&mut destination).is_err());
        assert_eq!(destination.graph_revision(), revision);
        assert_eq!(destination.to_file(), before);
        destination
            .redo_graph(destination.graph_revision())
            .unwrap();
        assert_eq!(destination.graph_snapshot(), redone);
        destination
            .apply_graph_command(
                GraphCommand::Disconnect { edge_id: collision },
                destination.graph_revision(),
            )
            .unwrap();
        clipboard.paste(&mut destination).unwrap();
        assert_eq!(
            destination.graph().nodes()[3].position.x,
            source.graph().nodes()[0].position.x + 40.0
        );
        let serialized = serde_json::to_string(&destination.to_file()).unwrap();
        assert_eq!(
            sdk.open_project_json(&serialized).unwrap().graph_snapshot(),
            destination.graph_snapshot()
        );
    }
}
