use super::*;
use crate::project::test_support::*;

#[test]
fn screen_saver_contracts_cover_missing_inputs_and_reserved_ids() {
    let missing_renderer = Project::try_from_file(project_file(renderer_graph())).unwrap();
    assert_eq!(
        missing_renderer
            .screen_saver_graph("missing", 1, 1)
            .unwrap_err()
            .code,
        SdkErrorCode::UnknownNode
    );
    let shader_only = Graph {
        nodes: vec![node("source", NodeType::Shader, vec![], vec![])],
        edges: vec![],
    };
    let shader_project = Project::try_from_file(project_file(shader_only)).unwrap();
    assert_eq!(
        shader_project
            .screen_saver_graph("source", 1, 1)
            .unwrap_err()
            .code,
        SdkErrorCode::UnknownNode
    );

    let renderer_without_input = Graph {
        nodes: vec![node("renderer", NodeType::Renderer, vec![], vec![])],
        edges: vec![],
    };
    let no_input = Project::try_from_file(project_file(renderer_without_input)).unwrap();
    assert_eq!(
        no_input
            .screen_saver_graph("renderer", 1, 1)
            .unwrap_err()
            .code,
        SdkErrorCode::InvalidGraph
    );

    let reserved_source = Graph {
        nodes: vec![
            node(
                SCREEN_SAVER_RESAMPLE_NODE_ID,
                NodeType::Shader,
                vec![],
                vec![port("out", DataType::Sampler2d, PortDirection::Output)],
            ),
            node(
                "renderer",
                NodeType::Renderer,
                vec![port("in", DataType::Sampler2d, PortDirection::Input)],
                vec![],
            ),
        ],
        edges: vec![Edge {
            id: "edge".to_owned(),
            source: SCREEN_SAVER_RESAMPLE_NODE_ID.to_owned(),
            source_handle: "out".to_owned(),
            target: "renderer".to_owned(),
            target_handle: "in".to_owned(),
        }],
    };
    let reserved = Project::try_from_file(project_file(reserved_source)).unwrap();
    assert_eq!(
        reserved
            .screen_saver_graph("renderer", 1, 1)
            .unwrap_err()
            .code,
        SdkErrorCode::InvalidGraph
    );
}

#[test]
fn screen_saver_inserts_resample_output() {
    let project = Project::try_from_file(project_file(renderer_graph())).unwrap();
    let saver = project.screen_saver_graph("renderer", 1920, 1080).unwrap();
    assert!(saver
        .nodes()
        .iter()
        .any(|node| node.id == SCREEN_SAVER_RESAMPLE_NODE_ID));
}
