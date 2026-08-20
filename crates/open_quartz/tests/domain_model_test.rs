use open_quartz::{
    DataType, Edge, Graph, InputMode, NodeData, NodeId, NodeType, Port, PortDirection, PortId,
    PortKey, Position, Project, ProjectFile, ProjectNode, SdkError, SdkErrorCode,
};
use serde_json::{Map, Value};

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

fn node(id: &str, inputs: Vec<Port>, outputs: Vec<Port>) -> ProjectNode {
    ProjectNode {
        id: id.to_owned(),
        node_type: NodeType::Input,
        position: Position { x: 0.0, y: 0.0 },
        data: NodeData {
            node_type: NodeType::Input,
            label: id.to_owned(),
            template_name: None,
            shader_template_id: None,
            shader_code: String::new(),
            inputs,
            outputs,
            uniforms: Map::<String, Value>::new(),
            input_data_type: Some(DataType::Float),
            input_mode: Some(InputMode::System),
            image_data_url: None,
            image_width: None,
            image_height: None,
            fb_format: None,
            fb_width: None,
            fb_height: None,
            fb_stride: None,
            raw_data_url: None,
            tex_filter: None,
            tex_wrap: None,
            width: None,
            height: None,
            auto_size: None,
            out_format: None,
            onnx_model_id: None,
            onnx_score_threshold: None,
            onnx_iou_threshold: None,
            onnx_target_size: None,
            onnx_source: None,
            onnx_catalog_id: None,
            onnx_custom_path: None,
            onnx_params: None,
            video_source_type: None,
            video_url: None,
            video_file_path: None,
            video_device_id: None,
            video_loop: None,
            video_playback_rate: None,
            system_source: None,
            math_op: None,
            feedback_enabled: None,
            feedback_clear_color: None,
            extra: Map::new(),
        },
    }
}

fn typed_node(
    id: &str,
    node_type: NodeType,
    inputs: Vec<Port>,
    outputs: Vec<Port>,
) -> ProjectNode {
    let mut node = node(id, inputs, outputs);
    node.node_type = node_type;
    node.data.node_type = node_type;
    node
}

#[test]
fn typed_ids_round_trip_as_stable_string_values() {
    let node_id = NodeId::new("source");
    let port_id = PortId::new("out");
    let key = PortKey::new(node_id.clone(), port_id.clone());

    assert_eq!(node_id.as_str(), "source");
    assert_eq!(port_id.as_str(), "out");
    assert_eq!(key.node_id(), &node_id);
    assert_eq!(key.port_id(), &port_id);
    assert_eq!(serde_json::to_string(&node_id).unwrap(), "\"source\"");
}

#[test]
fn graph_edit_commits_valid_nodes_and_connection_atomically() {
    let mut graph = Graph::default();
    let change = graph
        .edit(|edit| {
            edit.add_node(node(
                "source",
                vec![],
                vec![port("out", DataType::Float, PortDirection::Output)],
            ))?;
            edit.add_node(node(
                "target",
                vec![port("in", DataType::Float, PortDirection::Input)],
                vec![],
            ))?;
            edit.connect(
                PortKey::new(NodeId::new("source"), PortId::new("out")),
                PortKey::new(NodeId::new("target"), PortId::new("in")),
            )?;
            Ok(())
        })
        .unwrap();

    assert_eq!(graph.nodes().len(), 2);
    assert_eq!(graph.edges().len(), 1);
    assert_eq!(change.changed_nodes().len(), 2);
}

#[test]
fn graph_edit_rolls_back_every_change_when_operation_fails() {
    let mut graph = Graph::default();
    let result = graph.edit(|edit| {
        edit.add_node(node(
            "source",
            vec![],
            vec![port("out", DataType::Float, PortDirection::Output)],
        ))?;
        Err(SdkError::new(
            SdkErrorCode::InvalidGraph,
            "reject transaction",
        ))
    });

    assert!(result.is_err());
    assert!(graph.nodes().is_empty());
    assert!(graph.edges().is_empty());
}

#[test]
fn graph_edit_rejects_type_mismatch_without_partial_edge() {
    let mut graph = Graph {
        nodes: vec![
            node(
                "source",
                vec![],
                vec![port("out", DataType::Float, PortDirection::Output)],
            ),
            node(
                "target",
                vec![port("in", DataType::Vec4, PortDirection::Input)],
                vec![],
            ),
        ],
        edges: Vec::<Edge>::new(),
    };

    let result = graph.edit(|edit| {
        edit.connect(
            PortKey::new(NodeId::new("source"), PortId::new("out")),
            PortKey::new(NodeId::new("target"), PortId::new("in")),
        )?;
        Ok(())
    });

    assert!(result.is_err());
    assert!(graph.edges().is_empty());
}

#[test]
fn project_file_normalization_keeps_layout_out_of_executable_graph_changes() {
    let file = ProjectFile {
        version: "0.4.0".to_owned(),
        name: "Layout".to_owned(),
        created_at: "created".to_owned(),
        updated_at: "updated".to_owned(),
        graph: Graph {
            nodes: vec![node("source", vec![], vec![])],
            edges: vec![],
        },
    };
    let mut project = Project::from_file(file);
    let executable_before = project.graph().clone();

    project
        .layout_mut()
        .set_position(NodeId::new("source"), Position { x: 80.0, y: 40.0 })
        .unwrap();

    assert_eq!(project.graph(), &executable_before);
    assert_eq!(
        project.to_file().graph.nodes[0].position,
        Position { x: 80.0, y: 40.0 }
    );
}

#[test]
fn project_file_round_trip_preserves_metadata_and_graph() {
    let file = ProjectFile {
        version: "0.4.0".to_owned(),
        name: "Round trip".to_owned(),
        created_at: "created".to_owned(),
        updated_at: "updated".to_owned(),
        graph: Graph::default(),
    };

    let project = Project::from_file(file.clone());

    assert_eq!(project.to_file(), file);
}

#[test]
fn project_try_from_file_rejects_incompatible_versions() {
    let file = ProjectFile {
        version: "0.3.0".to_owned(),
        name: "Old".to_owned(),
        created_at: "created".to_owned(),
        updated_at: "updated".to_owned(),
        graph: Graph::default(),
    };

    let error = Project::try_from_file(file).unwrap_err();

    assert_eq!(error.code, SdkErrorCode::InvalidState);
    assert_eq!(error.message, "Incompatible project version");
}

#[test]
fn rust_project_io_normalizes_save_reload_to_a_stable_file_contract() {
    let sdk = open_quartz::OpenQuartz::new(open_quartz::Environment::headless());
    let source = ProjectFile {
        version: "0.4.0".to_owned(),
        name: "Round trip".to_owned(),
        created_at: "created".to_owned(),
        updated_at: "updated".to_owned(),
        graph: Graph {
            nodes: vec![typed_node(
                "source",
                NodeType::Input,
                vec![],
                vec![port("out", DataType::Sampler2d, PortDirection::Output)],
            )],
            edges: vec![],
        },
    };
    let first_json = serde_json::to_string(&source).unwrap();
    let normalized = sdk.normalize_project_json(&first_json).unwrap();
    let reloaded = sdk.open_project_json(&normalized).unwrap();
    let second_json = serde_json::to_string(&reloaded.to_file()).unwrap();

    assert_eq!(second_json, normalized);
    assert_eq!(reloaded.graph_revision(), 0);
}

#[test]
fn graph_replace_preserves_changed_positions_in_project_serialization() {
    let file = ProjectFile {
        version: "0.4.0".to_owned(),
        name: "Position".to_owned(),
        created_at: "created".to_owned(),
        updated_at: "updated".to_owned(),
        graph: Graph {
            nodes: vec![node("source", vec![], vec![])],
            edges: vec![],
        },
    };
    let mut project = Project::from_file(file);
    let mut changed = project.graph_snapshot();
    changed.nodes[0].position = Position { x: 120.0, y: 60.0 };

    project.replace_graph(changed, 0).unwrap();

    assert_eq!(
        project.to_file().graph.nodes[0].position,
        Position { x: 120.0, y: 60.0 }
    );
}

#[test]
fn project_replace_rejects_invalid_graph_without_mutating_revision() {
    let sdk = open_quartz::OpenQuartz::new(open_quartz::Environment::headless());
    let mut project = sdk.create_project("Validation");
    let invalid = Graph {
        nodes: vec![node("same", vec![], vec![]), node("same", vec![], vec![])],
        edges: vec![],
    };

    let error = project.replace_graph(invalid, 0).unwrap_err();

    assert_eq!(error.code, SdkErrorCode::InvalidGraph);
    assert_eq!(project.graph_revision(), 0);
    assert!(project.graph().nodes().is_empty());
}

#[test]
fn upstream_subgraph_keeps_only_nodes_required_by_selected_output() {
    let graph = Graph {
        nodes: vec![
            node(
                "source",
                vec![],
                vec![port("out", DataType::Float, PortDirection::Output)],
            ),
            node(
                "output",
                vec![port("in", DataType::Float, PortDirection::Input)],
                vec![],
            ),
            node("unused", vec![], vec![]),
        ],
        edges: vec![Edge {
            id: "source-output".to_owned(),
            source: "source".to_owned(),
            source_handle: "out".to_owned(),
            target: "output".to_owned(),
            target_handle: "in".to_owned(),
        }],
    };

    let exported = graph.upstream_subgraph(&NodeId::new("output")).unwrap();

    assert_eq!(
        exported
            .nodes()
            .iter()
            .map(|node| node.id.as_str())
            .collect::<Vec<_>>(),
        vec!["source", "output"]
    );
    assert_eq!(exported.edges().len(), 1);
}

#[test]
fn screen_saver_graph_uses_rust_upstream_and_resample_semantics() {
    let file = ProjectFile {
        version: "0.4.0".to_owned(),
        name: "Saver".to_owned(),
        created_at: "created".to_owned(),
        updated_at: "updated".to_owned(),
        graph: Graph {
            nodes: vec![
                typed_node(
                    "source",
                    NodeType::Shader,
                    vec![],
                    vec![port("out", DataType::Sampler2d, PortDirection::Output)],
                ),
                typed_node(
                    "renderer",
                    NodeType::Renderer,
                    vec![port("in", DataType::Sampler2d, PortDirection::Input)],
                    vec![],
                ),
                typed_node("unused", NodeType::Shader, vec![], vec![]),
            ],
            edges: vec![Edge {
                id: "source-renderer".to_owned(),
                source: "source".to_owned(),
                source_handle: "out".to_owned(),
                target: "renderer".to_owned(),
                target_handle: "in".to_owned(),
            }],
        },
    };
    let project = Project::try_from_file(file).unwrap();

    let graph = project
        .screen_saver_graph("renderer", 3840, 2160)
        .unwrap();

    assert_eq!(
        graph
            .nodes()
            .iter()
            .map(|node| node.id.as_str())
            .collect::<Vec<_>>(),
        vec!["source", "renderer", "__screen_saver_output_resample"]
    );
    let resample = graph
        .nodes()
        .iter()
        .find(|node| node.id == "__screen_saver_output_resample")
        .unwrap();
    assert_eq!(resample.data.width, Some(3840));
    assert_eq!(resample.data.height, Some(2160));
    assert_eq!(
        graph
            .edges()
            .iter()
            .map(|edge| edge.id.as_str())
            .collect::<Vec<_>>(),
        vec!["__screen_saver_source_edge", "__screen_saver_renderer_edge"]
    );
}
