use super::*;
use crate::{NodeData, PortId};

fn port(id: &str, data_type: DataType, direction: PortDirection) -> Port {
    Port {
        id: id.to_owned(),
        label: id.to_owned(),
        data_type,
        direction,
        default_value: None,
        description: None,
    }
}

fn node(id: &str, node_type: NodeType, input: DataType, output: DataType) -> ProjectNode {
    ProjectNode {
        id: id.to_owned(),
        node_type,
        position: Position { x: 0.0, y: 0.0 },
        data: NodeData {
            node_type,
            label: id.to_owned(),
            inputs: vec![port("in", input, PortDirection::Input)],
            outputs: vec![port("out", output, PortDirection::Output)],
            ..NodeData::default()
        },
    }
}

fn edge(id: &str, source: &str, source_handle: &str, target: &str, target_handle: &str) -> Edge {
    Edge {
        id: id.to_owned(),
        source: source.to_owned(),
        source_handle: source_handle.to_owned(),
        target: target.to_owned(),
        target_handle: target_handle.to_owned(),
    }
}

fn valid_graph() -> Graph {
    Graph {
        nodes: vec![
            node("a", NodeType::Input, DataType::Float, DataType::Float),
            node("b", NodeType::Math, DataType::Float, DataType::Float),
        ],
        edges: vec![edge("a-b", "a", "out", "b", "in")],
    }
}

#[test]
fn validation_covers_every_structural_error_and_compatibility_rule() {
    let graph = valid_graph();
    assert_eq!(graph.nodes().len(), 2);
    assert_eq!(graph.edges().len(), 1);
    graph.validate().unwrap();

    let mut duplicate_node = graph.clone();
    duplicate_node.nodes.push(duplicate_node.nodes[0].clone());
    assert_eq!(
        duplicate_node.validate().unwrap_err().code,
        SdkErrorCode::InvalidGraph
    );

    let mut duplicate_edge = graph.clone();
    duplicate_edge.edges.push(duplicate_edge.edges[0].clone());
    assert_eq!(
        duplicate_edge.validate().unwrap_err().code,
        SdkErrorCode::InvalidGraph
    );

    for (source, source_handle, target, target_handle, code) in [
        ("missing", "out", "b", "in", SdkErrorCode::UnknownNode),
        ("a", "missing", "b", "in", SdkErrorCode::InvalidGraph),
        ("a", "out", "missing", "in", SdkErrorCode::UnknownNode),
        ("a", "out", "b", "missing", SdkErrorCode::InvalidGraph),
    ] {
        let mut invalid = graph.clone();
        invalid.edges[0] = edge("bad", source, source_handle, target, target_handle);
        assert_eq!(invalid.validate().unwrap_err().code, code);
    }

    let mut mismatch = graph.clone();
    mismatch.nodes[1].data.inputs[0].data_type = DataType::Vec4;
    assert_eq!(
        mismatch.validate().unwrap_err().code,
        SdkErrorCode::InvalidGraph
    );

    let mut multiple = graph.clone();
    multiple
        .nodes
        .push(node("c", NodeType::Input, DataType::Float, DataType::Float));
    multiple.edges.push(edge("c-b", "c", "out", "b", "in"));
    assert_eq!(
        multiple.validate().unwrap_err().code,
        SdkErrorCode::InvalidGraph
    );

    let mut cycle = graph.clone();
    cycle.edges.push(edge("b-a", "b", "out", "a", "in"));
    assert_eq!(
        cycle.validate().unwrap_err().code,
        SdkErrorCode::InvalidGraph
    );

    assert!(compatible_types(
        DataType::Float,
        DataType::Float,
        NodeType::Input
    ));
    assert!(compatible_types(
        DataType::Auto,
        DataType::Float,
        NodeType::Math
    ));
    assert!(compatible_types(
        DataType::Float,
        DataType::Auto,
        NodeType::Math
    ));
    assert!(!compatible_types(
        DataType::Auto,
        DataType::Json,
        NodeType::Math
    ));
    assert!(compatible_types(
        DataType::Vec4,
        DataType::Sampler2d,
        NodeType::Shader
    ));
    assert!(compatible_types(
        DataType::SamplerCube,
        DataType::Sampler2d,
        NodeType::Input
    ));
    assert!(!compatible_types(
        DataType::Vec4,
        DataType::Sampler2d,
        NodeType::Input
    ));
    assert!(!compatible_types(
        DataType::Float,
        DataType::Vec4,
        NodeType::Input
    ));
}

#[test]
fn subgraphs_edits_and_commands_cover_success_and_failure_transitions() {
    let graph = valid_graph();
    assert_eq!(
        graph
            .upstream_subgraph(&NodeId::new("b"))
            .unwrap()
            .nodes
            .len(),
        2
    );
    assert_eq!(
        graph
            .upstream_subgraph(&NodeId::new("missing"))
            .unwrap_err()
            .code,
        SdkErrorCode::UnknownNode
    );

    let mut graph = Graph::default();
    assert!(graph
        .edit(&mut |_| Ok(()))
        .unwrap()
        .changed_nodes()
        .is_empty());
    let a = node("a", NodeType::Input, DataType::Float, DataType::Float);
    let b = node("b", NodeType::Math, DataType::Float, DataType::Float);
    assert_eq!(
        graph
            .apply_command(GraphCommand::AddNode { node: a.clone() })
            .unwrap()
            .changed_nodes(),
        &[NodeId::new("a")]
    );
    let diamond = Graph {
        nodes: vec![
            node("a", NodeType::Input, DataType::Float, DataType::Float),
            node("b", NodeType::Math, DataType::Float, DataType::Float),
            node("c", NodeType::Math, DataType::Float, DataType::Float),
            node("d", NodeType::Math, DataType::Float, DataType::Float),
        ],
        edges: vec![
            edge("a-b", "a", "out", "b", "in"),
            edge("a-c", "a", "out", "c", "in"),
            edge("b-d", "b", "out", "d", "in"),
            edge("c-d", "c", "out", "d", "in"),
        ],
    };
    assert_eq!(
        diamond
            .upstream_subgraph(&NodeId::new("d"))
            .unwrap()
            .nodes
            .len(),
        4
    );

    let mut invalid_edit = Graph::default();
    assert_eq!(
        invalid_edit
            .edit(&mut |edit| {
                edit.graph.nodes.extend([
                    node("same", NodeType::Input, DataType::Float, DataType::Float),
                    node("same", NodeType::Input, DataType::Float, DataType::Float),
                ]);
                Ok(())
            })
            .unwrap_err()
            .code,
        SdkErrorCode::InvalidGraph
    );
    let mut operation_error = valid_graph();
    assert_eq!(
        operation_error
            .edit(&mut |edit| {
                edit.update_node(NodeId::new("a"), &mut |_| {
                    Err(SdkError::new(SdkErrorCode::InvalidState, "operation"))
                })
            })
            .unwrap_err()
            .code,
        SdkErrorCode::InvalidState
    );

    assert_eq!(
        graph
            .apply_command(GraphCommand::AddNode { node: a.clone() })
            .unwrap_err()
            .code,
        SdkErrorCode::InvalidGraph
    );
    let mut invalid_command_graph = valid_graph();
    let mut invalid_node = invalid_command_graph.nodes[0].clone();
    invalid_node.data.outputs.clear();
    assert!(invalid_command_graph
        .apply_command(GraphCommand::UpdateNode { node: invalid_node })
        .is_err());
    graph
        .apply_command(GraphCommand::AddNode { node: b.clone() })
        .unwrap();

    let mut updated = a.clone();
    updated.data.label = "updated".to_owned();
    graph
        .apply_command(GraphCommand::UpdateNode { node: updated })
        .unwrap();
    let mut data = graph.nodes[0].data.clone();
    data.label = "data".to_owned();
    graph
        .apply_command(GraphCommand::UpdateNodeData {
            node_id: NodeId::new("a"),
            data,
        })
        .unwrap();
    graph
        .apply_command(GraphCommand::UpdateNodePorts {
            node_id: NodeId::new("a"),
            inputs: vec![],
            outputs: vec![port("out", DataType::Float, PortDirection::Output)],
        })
        .unwrap();
    graph
        .apply_command(GraphCommand::SetNodePosition {
            node_id: NodeId::new("a"),
            position: Position { x: 1.0, y: 2.0 },
        })
        .unwrap();
    assert_eq!(graph.nodes[0].position, Position { x: 1.0, y: 2.0 });

    assert_eq!(
        graph
            .apply_command(GraphCommand::UpdateShaderCode {
                node_id: NodeId::new("a"),
                shader_code: String::new(),
            })
            .unwrap_err()
            .code,
        SdkErrorCode::InvalidState
    );
    assert_eq!(
        graph
            .apply_command(GraphCommand::UpdateInputType {
                node_id: NodeId::new("a"),
                data_type: DataType::Float,
                input_mode: None,
            })
            .unwrap_err()
            .code,
        SdkErrorCode::InvalidState
    );

    let source = PortKey::new(NodeId::new("a"), PortId::new("out"));
    let target = PortKey::new(NodeId::new("b"), PortId::new("in"));
    graph
        .apply_command(GraphCommand::Connect {
            source: source.clone(),
            target: target.clone(),
        })
        .unwrap();
    let edge_id = graph.edges[0].id.clone();
    graph
        .apply_command(GraphCommand::Disconnect { edge_id })
        .unwrap();
    let mut mismatch = graph.clone();
    mismatch.nodes[1].data.inputs[0].data_type = DataType::Vec4;
    assert_eq!(
        mismatch
            .apply_command(GraphCommand::Connect {
                source: source.clone(),
                target: target.clone(),
            })
            .unwrap_err()
            .code,
        SdkErrorCode::InvalidGraph
    );
    graph
        .apply_command(GraphCommand::Connect {
            source: source.clone(),
            target: target.clone(),
        })
        .unwrap();
    graph
        .apply_command(GraphCommand::Connect {
            source: source.clone(),
            target: target.clone(),
        })
        .unwrap();
    assert_eq!(graph.edges.len(), 1);

    assert_eq!(
        graph
            .apply_command(GraphCommand::Disconnect {
                edge_id: "missing".to_owned()
            })
            .unwrap_err()
            .code,
        SdkErrorCode::InvalidGraph
    );

    for (bad_source, bad_target, expected) in [
        (
            PortKey::new(NodeId::new("missing"), PortId::new("out")),
            target.clone(),
            SdkErrorCode::UnknownNode,
        ),
        (
            PortKey::new(NodeId::new("a"), PortId::new("missing")),
            target.clone(),
            SdkErrorCode::InvalidGraph,
        ),
        (
            source.clone(),
            PortKey::new(NodeId::new("missing"), PortId::new("in")),
            SdkErrorCode::UnknownNode,
        ),
        (
            source.clone(),
            PortKey::new(NodeId::new("b"), PortId::new("missing")),
            SdkErrorCode::InvalidGraph,
        ),
    ] {
        assert_eq!(
            graph
                .apply_command(GraphCommand::Connect {
                    source: bad_source,
                    target: bad_target,
                })
                .unwrap_err()
                .code,
            expected
        );
    }

    let mut incoming = Graph {
        nodes: vec![a.clone(), b.clone()],
        edges: vec![edge("a-b", "a", "out", "b", "in")],
    };
    incoming
        .apply_command(GraphCommand::RemoveNode {
            node_id: NodeId::new("b"),
        })
        .unwrap();
    assert!(incoming.edges.is_empty());

    assert_eq!(
        graph
            .edit(&mut |edit| edit.update_node(NodeId::new("missing"), &mut |_| Ok(())))
            .unwrap_err()
            .code,
        SdkErrorCode::UnknownNode
    );
    assert_eq!(
        graph
            .edit(&mut |edit| edit.remove_node(&NodeId::new("missing")).map(|_| ()))
            .unwrap_err()
            .code,
        SdkErrorCode::UnknownNode
    );

    graph
        .apply_command(GraphCommand::RemoveNode {
            node_id: NodeId::new("a"),
        })
        .unwrap();
    assert!(graph.edges.is_empty());

    let empty = GraphChange::default();
    assert!(empty.is_empty());
    assert_eq!(
        GraphChange::from_changed_nodes([NodeId::new("x")])
            .changed_nodes()
            .len(),
        1
    );
}
#[test]
fn cycle_connect_rolls_back_and_path_search_handles_revisits() {
    let mut graph = Graph {
        nodes: vec![
            node("a", NodeType::Input, DataType::Float, DataType::Float),
            node("b", NodeType::Input, DataType::Float, DataType::Float),
        ],
        edges: vec![edge("a-b", "a", "out", "b", "in")],
    };
    let result = graph.apply_command(GraphCommand::Connect {
        source: PortKey::new(NodeId::new("b"), PortId::new("out")),
        target: PortKey::new(NodeId::new("a"), PortId::new("in")),
    });
    assert_eq!(result.unwrap_err().code, SdkErrorCode::InvalidGraph);
    assert_eq!(graph.edges.len(), 1);
    assert!(has_path(&graph.edges, "a", "b"));
    assert!(!has_path(&graph.edges, "b", "missing"));
    let diamond = vec![
        edge("a-b", "a", "out", "b", "in"),
        edge("a-c", "a", "out", "c", "in"),
        edge("b-d", "b", "out", "d", "in"),
        edge("c-d", "c", "out", "d", "in"),
    ];
    assert!(!has_path(&diamond, "a", "missing"));
}
