use super::*;
use crate::project::{DataType, Edge, NodeData, NodeType, Port, PortDirection, Position, ProjectFile, ProjectNode, PROJECT_FILE_VERSION};

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

fn node(id: &str, node_type: NodeType, inputs: Vec<Port>, outputs: Vec<Port>) -> ProjectNode {
    ProjectNode {
        id: id.to_owned(),
        node_type,
        position: Position { x: 0.0, y: 0.0 },
        data: NodeData {
            node_type,
            label: id.to_owned(),
            inputs,
            outputs,
            ..NodeData::default()
        },
    }
}

fn renderer_graph() -> Graph {
    Graph {
        nodes: vec![
            node(
                "source",
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
            id: "source-renderer".to_owned(),
            source: "source".to_owned(),
            source_handle: "out".to_owned(),
            target: "renderer".to_owned(),
            target_handle: "in".to_owned(),
        }],
    }
}

fn project_file(graph: Graph) -> ProjectFile {
    ProjectFile {
        version: PROJECT_FILE_VERSION.to_owned(),
        name: "Coverage".to_owned(),
        created_at: "created".to_owned(),
        updated_at: "updated".to_owned(),
        graph,
    }
}

#[test]
fn json_project_entry_contracts_cover_failures() {
    let sdk = OpenQuartz::new(Environment::headless());
    let invalid_json = sdk.open_project_json("{").unwrap_err();
    assert_eq!(invalid_json.code, SdkErrorCode::InvalidState);
    assert_eq!(
        sdk.normalize_project_json("{").unwrap_err().code,
        SdkErrorCode::InvalidState
    );
    assert_eq!(
        sdk.screen_saver_export_project_json("{", "renderer")
            .unwrap_err()
            .code,
        SdkErrorCode::InvalidState
    );

    let graph = renderer_graph();
    let json = serde_json::to_string(&project_file(graph.clone())).unwrap();
    let normalized = sdk.normalize_project_json(&json).unwrap();
    assert_eq!(
        sdk.open_project_json(&normalized).unwrap().name(),
        "Coverage"
    );
    let exported = sdk
        .screen_saver_export_project_json(&json, "renderer")
        .unwrap();
    assert_eq!(
        sdk.open_project_json(&exported)
            .unwrap()
            .graph()
            .nodes()
            .len(),
        2
    );
    assert_eq!(
        sdk.screen_saver_export_project_json(&json, "missing")
            .unwrap_err()
            .code,
        SdkErrorCode::UnknownNode
    );
}
