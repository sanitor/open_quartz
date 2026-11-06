use super::*;

pub(super) fn port(id: &str, data_type: DataType, direction: PortDirection) -> Port {
    Port {
        id: id.to_owned(),
        label: id.to_owned(),
        data_type,
        direction,
        default_value: None,
        description: None,
    }
}

pub(super) fn node(id: &str, node_type: NodeType, inputs: Vec<Port>, outputs: Vec<Port>) -> ProjectNode {
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

pub(super) fn renderer_graph() -> Graph {
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

pub(super) fn project_file(graph: Graph) -> ProjectFile {
    ProjectFile {
        version: PROJECT_FILE_VERSION.to_owned(),
        name: "Coverage".to_owned(),
        created_at: "created".to_owned(),
        updated_at: "updated".to_owned(),
        graph,
    }
}
