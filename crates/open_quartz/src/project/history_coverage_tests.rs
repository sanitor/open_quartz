use super::*;
use crate::player::{Environment, OpenQuartz};
use crate::project::test_support::*;

#[test]
fn graph_updates_cover_input_shader_parse_and_edge_change_contracts() {
    let sdk = OpenQuartz::new(Environment::headless());
    let mut project = sdk.create_project("Commands");
    assert_eq!(
        project
            .create_graph_node(NodeFactoryRequest::Constant { position: None }, 99)
            .unwrap_err()
            .code,
        SdkErrorCode::StaleRevision
    );
    assert_eq!(
        project
            .apply_graph_command(
                GraphCommand::RemoveNode {
                    node_id: NodeId::new("missing")
                },
                99,
            )
            .unwrap_err()
            .code,
        SdkErrorCode::StaleRevision
    );
    let mut invalid_initialization = sdk.create_project("Invalid initialization");
    assert_eq!(
        invalid_initialization
            .initialize_graph(Graph {
                nodes: vec![
                    node("same", NodeType::Input, vec![], vec![]),
                    node("same", NodeType::Input, vec![], vec![]),
                ],
                edges: vec![],
            })
            .unwrap_err()
            .code,
        SdkErrorCode::InvalidGraph
    );
    let (input, _) = project
        .create_graph_node(
            NodeFactoryRequest::Input {
                position: None,
                data_type: DataType::Float,
                input_mode: None,
            },
            0,
        )
        .unwrap();
    project
        .apply_graph_command(
            GraphCommand::UpdateInputType {
                node_id: NodeId::new(&input.id),
                data_type: DataType::Sampler2d,
                input_mode: None,
            },
            project.graph_revision(),
        )
        .unwrap();
    assert_eq!(
        project.graph().nodes()[0].data.input_mode,
        Some(InputMode::Image)
    );
    project
        .apply_graph_command(
            GraphCommand::UpdateInputType {
                node_id: NodeId::new(&input.id),
                data_type: DataType::Float,
                input_mode: Some(InputMode::System),
            },
            project.graph_revision(),
        )
        .unwrap();

    assert_eq!(
        project
            .apply_graph_command(
                GraphCommand::UpdateInputType {
                    node_id: NodeId::new("missing"),
                    data_type: DataType::Float,
                    input_mode: None,
                },
                project.graph_revision(),
            )
            .unwrap_err()
            .code,
        SdkErrorCode::UnknownNode
    );

    let (shader, _) = project
        .create_graph_node(
            NodeFactoryRequest::Shader {
                position: None,
                code: "@fragment fn main() -> @location(0) vec4f { return vec4f(1.0); }".to_owned(),
                label: "Shader".to_owned(),
                template_name: Some("Shader".to_owned()),
                shader_template_id: None,
            },
            project.graph_revision(),
        )
        .unwrap();
    assert_eq!(
        project
            .apply_graph_command(
                GraphCommand::UpdateInputType {
                    node_id: NodeId::new(&shader.id),
                    data_type: DataType::Float,
                    input_mode: None,
                },
                project.graph_revision(),
            )
            .unwrap_err()
            .code,
        SdkErrorCode::InvalidGraph
    );
    assert_eq!(
        project
            .apply_graph_command(
                GraphCommand::UpdateShaderCode {
                    node_id: NodeId::new("missing"),
                    shader_code: String::new(),
                },
                project.graph_revision(),
            )
            .unwrap_err()
            .code,
        SdkErrorCode::UnknownNode
    );
    assert!(changed_node_ids(&Graph::default(), &Graph::default()).is_empty());
    let unchanged = renderer_graph();
    assert!(changed_node_ids(&unchanged, &unchanged).is_empty());

    let shader_source = node(
        "shader",
        NodeType::Shader,
        vec![],
        vec![port("out", DataType::Sampler2d, PortDirection::Output)],
    );
    let renderer_target = node(
        "renderer",
        NodeType::Renderer,
        vec![port("in", DataType::Sampler2d, PortDirection::Input)],
        vec![],
    );
    let mut invalid_shader_update = Project::try_from_file(project_file(Graph {
        nodes: vec![shader_source, renderer_target],
        edges: vec![Edge {
            id: "shader-renderer".to_owned(),
            source: "shader".to_owned(),
            source_handle: "out".to_owned(),
            target: "renderer".to_owned(),
            target_handle: "in".to_owned(),
        }],
    }))
    .unwrap();
    assert_eq!(
        invalid_shader_update
            .apply_graph_command(
                GraphCommand::UpdateShaderCode {
                    node_id: NodeId::new("shader"),
                    shader_code: "not wgsl".to_owned(),
                },
                0,
            )
            .unwrap_err()
            .code,
        SdkErrorCode::InvalidGraph
    );

    let input_source = node(
        "input",
        NodeType::Input,
        vec![],
        vec![port("out", DataType::Float, PortDirection::Output)],
    );
    let scalar_target = node(
        "target",
        NodeType::Math,
        vec![port("in", DataType::Float, PortDirection::Input)],
        vec![],
    );
    let mut invalid_input_update = Project::try_from_file(project_file(Graph {
        nodes: vec![input_source, scalar_target],
        edges: vec![Edge {
            id: "input-target".to_owned(),
            source: "input".to_owned(),
            source_handle: "out".to_owned(),
            target: "target".to_owned(),
            target_handle: "in".to_owned(),
        }],
    }))
    .unwrap();
    assert_eq!(
        invalid_input_update
            .apply_graph_command(
                GraphCommand::UpdateInputType {
                    node_id: NodeId::new("input"),
                    data_type: DataType::Sampler2d,
                    input_mode: None,
                },
                0,
            )
            .unwrap_err()
            .code,
        SdkErrorCode::InvalidGraph
    );
    project
        .apply_graph_command(
            GraphCommand::UpdateShaderCode {
                node_id: NodeId::new(&shader.id),
                shader_code: "not wgsl".to_owned(),
            },
            project.graph_revision(),
        )
        .unwrap();
    assert!(project
        .graph()
        .nodes()
        .iter()
        .find(|node| node.id == shader.id)
        .unwrap()
        .data
        .extra
        .contains_key("parseError"));

    let source = node(
        "a",
        NodeType::Input,
        vec![],
        vec![port("out", DataType::Float, PortDirection::Output)],
    );
    let target = node(
        "b",
        NodeType::Input,
        vec![port("in", DataType::Float, PortDirection::Input)],
        vec![],
    );
    let graph_without_edge = Graph {
        nodes: vec![source.clone(), target.clone()],
        edges: vec![],
    };
    let graph_with_edge = Graph {
        nodes: vec![source, target],
        edges: vec![Edge {
            id: "a-b".to_owned(),
            source: "a".to_owned(),
            source_handle: "out".to_owned(),
            target: "b".to_owned(),
            target_handle: "in".to_owned(),
        }],
    };
    let mut edge_project = sdk.create_project("Edges");
    edge_project.initialize_graph(graph_without_edge).unwrap();
    let added = edge_project.replace_graph(graph_with_edge, 0).unwrap();
    assert_eq!(added.changed_nodes().len(), 2);
    let removed = edge_project
        .replace_graph(
            Graph {
                nodes: edge_project.graph().nodes().to_vec(),
                edges: vec![],
            },
            1,
        )
        .unwrap();
    assert_eq!(removed.changed_nodes().len(), 2);
}

#[test]
fn history_contracts_reject_stale_and_missing_revisions() {
    let sdk = OpenQuartz::new(Environment::headless());
    let mut stale_history = sdk.create_project("Stale history");
    stale_history
        .create_graph_node(NodeFactoryRequest::Constant { position: None }, 0)
        .unwrap();
    assert_eq!(
        stale_history.rollback_graph(99).unwrap_err().code,
        SdkErrorCode::StaleRevision
    );
    stale_history.rollback_graph(1).unwrap();
    assert_eq!(
        stale_history.redo_graph(99).unwrap_err().code,
        SdkErrorCode::StaleRevision
    );
    let mut empty = sdk.create_project("History");
    assert_eq!(
        empty.rollback_graph(0).unwrap_err().code,
        SdkErrorCode::InvalidState
    );
    assert_eq!(
        empty.redo_graph(0).unwrap_err().code,
        SdkErrorCode::InvalidState
    );
}

#[test]
fn history_rejects_invalid_replacement_and_reconciles_shader_parse_state() {
    let sdk = OpenQuartz::new(Environment::headless());
    let mut project = Project::try_from_file(project_file(renderer_graph())).unwrap();
    let invalid = Graph {
        nodes: vec![
            node("same", NodeType::Input, vec![], vec![]),
            node("same", NodeType::Input, vec![], vec![]),
        ],
        edges: vec![],
    };
    assert_eq!(
        project.replace_graph(invalid, 0).unwrap_err().code,
        SdkErrorCode::InvalidGraph
    );

    let mut history = sdk.create_project("History");
    history
        .create_graph_node(NodeFactoryRequest::Renderer { position: None }, 0)
        .unwrap();
    history.rollback_graph(1).unwrap();
    history.redo_graph(2).unwrap();

    let (shader, _) = history
        .create_graph_node(
            NodeFactoryRequest::Shader {
                position: None,
                code: "not wgsl".to_owned(),
                label: "Parse".to_owned(),
                template_name: None,
                shader_template_id: None,
            },
            history.graph_revision(),
        )
        .unwrap();
    history
        .apply_graph_command(
            GraphCommand::UpdateShaderCode {
                node_id: NodeId::new(&shader.id),
                shader_code: "not wgsl".to_owned(),
            },
            history.graph_revision(),
        )
        .unwrap();
    history
        .apply_graph_command(
            GraphCommand::UpdateShaderCode {
                node_id: NodeId::new(&shader.id),
                shader_code: "@fragment fn main() -> @location(0) vec4f { return vec4f(1.0); }"
                    .to_owned(),
            },
            history.graph_revision(),
        )
        .unwrap();
    assert!(!history
        .graph()
        .nodes()
        .iter()
        .find(|node| node.id == shader.id)
        .unwrap()
        .data
        .extra
        .contains_key("parseError"));
}
