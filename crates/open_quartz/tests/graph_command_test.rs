use open_quartz::{
    DataType, Graph, GraphCommand, InputMode, NodeData, NodeFactoryRequest, NodeId, NodeType,
    OpenQuartz, Port, PortDirection, PortId, PortKey, Position, ProjectNode, SdkErrorCode,
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

fn node(
    id: &str,
    node_type: NodeType,
    inputs: Vec<Port>,
    outputs: Vec<Port>,
) -> ProjectNode {
    ProjectNode {
        id: id.to_owned(),
        node_type,
        position: Position { x: 0.0, y: 0.0 },
        data: NodeData {
            node_type,
            label: id.to_owned(),
            shader_code: String::new(),
            inputs,
            outputs,
            uniforms: Map::<String, Value>::new(),
            ..NodeData::default()
        },
    }
}

fn empty_project() -> open_quartz::Project {
    OpenQuartz::new(open_quartz::Environment::headless()).create_project("commands")
}

#[test]
fn rust_factory_creates_typed_nodes_and_owns_ids() {
    let mut project = empty_project();
    let (shader, first_change) = project
        .create_graph_node(
            NodeFactoryRequest::Shader {
                position: Some(Position { x: 12.0, y: 24.0 }),
                code: "@group(0) @binding(0) var<uniform> value: f32;\n\
                       @fragment fn main(@location(0) uv: vec2f) -> @location(0) vec4f { \
                         return vec4f(value); \
                       }"
                    .to_owned(),
                label: "Custom".to_owned(),
                template_name: Some("Custom".to_owned()),
                shader_template_id: None,
            },
            0,
        )
        .unwrap();

    assert_eq!(shader.id, "shader_1");
    assert_eq!(shader.data.label, "custom_1");
    assert_eq!(shader.position, Position { x: 12.0, y: 24.0 });
    assert_eq!(shader.data.inputs[0].data_type, DataType::Float);
    assert_eq!(first_change.changed_nodes().len(), 1);

    let (math, _) = project
        .create_graph_node(
            NodeFactoryRequest::Math {
                position: None,
                op: "add".to_owned(),
            },
            project.graph_revision(),
        )
        .unwrap();
    assert_eq!(math.id, "math_1");
    assert_eq!(math.data.inputs.len(), 2);
    assert_eq!(math.data.inputs[0].data_type, DataType::Auto);

    let (renderer, _) = project
        .create_graph_node(
            NodeFactoryRequest::Renderer { position: None },
            project.graph_revision(),
        )
        .unwrap();
    assert_eq!(renderer.id, "renderer_1");
    assert_eq!(
        renderer.data.extra.get("expanded"),
        Some(&Value::Bool(true))
    );
}

#[test]
fn typed_commands_connect_disconnect_and_remove_cascade() {
    let mut graph = Graph {
        nodes: vec![
            node(
                "source",
                NodeType::Input,
                vec![],
                vec![port("out", DataType::Float, PortDirection::Output)],
            ),
            node(
                "target",
                NodeType::Math,
                vec![port("in", DataType::Float, PortDirection::Input)],
                vec![],
            ),
        ],
        edges: vec![],
    };

    let change = graph
        .apply_command(GraphCommand::Connect {
            source: PortKey::new(NodeId::new("source"), PortId::new("out")),
            target: PortKey::new(NodeId::new("target"), PortId::new("in")),
        })
        .unwrap();
    assert_eq!(graph.edges().len(), 1);
    assert_eq!(change.changed_nodes().len(), 2);

    let edge_id = graph.edges()[0].id.clone();
    graph
        .apply_command(GraphCommand::Disconnect { edge_id })
        .unwrap();
    assert!(graph.edges().is_empty());

    graph
        .apply_command(GraphCommand::Connect {
            source: PortKey::new(NodeId::new("source"), PortId::new("out")),
            target: PortKey::new(NodeId::new("target"), PortId::new("in")),
        })
        .unwrap();
    graph
        .apply_command(GraphCommand::RemoveNode {
            node_id: NodeId::new("source"),
        })
        .unwrap();
    assert!(graph.nodes().iter().all(|node| node.id != "source"));
    assert!(graph.edges().is_empty());
}

#[test]
fn typed_commands_reject_incompatible_types_and_cycles_atomically() {
    let mut graph = Graph {
        nodes: vec![
            node(
                "a",
                NodeType::Input,
                vec![port("in", DataType::Float, PortDirection::Input)],
                vec![port("out", DataType::Float, PortDirection::Output)],
            ),
            node(
                "b",
                NodeType::Input,
                vec![port("in", DataType::Vec4, PortDirection::Input)],
                vec![port("out", DataType::Vec4, PortDirection::Output)],
            ),
            node(
                "c",
                NodeType::Input,
                vec![port("in", DataType::Float, PortDirection::Input)],
                vec![port("out", DataType::Float, PortDirection::Output)],
            ),
        ],
        edges: vec![],
    };

    let mismatch = graph.apply_command(GraphCommand::Connect {
        source: PortKey::new(NodeId::new("a"), PortId::new("out")),
        target: PortKey::new(NodeId::new("b"), PortId::new("in")),
    });
    assert_eq!(mismatch.unwrap_err().code, SdkErrorCode::InvalidGraph);
    assert!(graph.edges().is_empty());

    graph
        .apply_command(GraphCommand::Connect {
            source: PortKey::new(NodeId::new("a"), PortId::new("out")),
            target: PortKey::new(NodeId::new("c"), PortId::new("in")),
        })
        .unwrap();
    let cycle = graph.apply_command(GraphCommand::Connect {
        source: PortKey::new(NodeId::new("c"), PortId::new("out")),
        target: PortKey::new(NodeId::new("a"), PortId::new("in")),
    });
    assert!(cycle.is_err());
    assert_eq!(graph.edges().len(), 1);
}

#[test]
fn project_shader_port_updates_emit_revision_and_preserve_previous_graph_on_failure() {
    let mut project = empty_project();
    let (node, _) = project
        .create_graph_node(
            NodeFactoryRequest::Shader {
                position: None,
                code: "@fragment fn main(@location(0) uv: vec2f) -> @location(0) vec4f { return vec4f(1.0); }"
                    .to_owned(),
                label: "Shader".to_owned(),
                template_name: None,
                shader_template_id: None,
            },
            0,
        )
        .unwrap();
    let revision = project.graph_revision();

    let change = project
        .apply_graph_command(
            GraphCommand::UpdateShaderCode {
                node_id: NodeId::new(node.id.clone()),
                shader_code: "@group(0) @binding(0) var<uniform> gain: f32;\n\
                              @fragment fn main(@location(0) uv: vec2f) -> @location(0) vec4f { \
                                return vec4f(gain); \
                              }"
                    .to_owned(),
            },
            revision,
        )
        .unwrap();
    assert_eq!(change.changed_nodes(), &[NodeId::new(node.id.clone())]);
    assert_eq!(project.graph_revision(), revision + 1);
    assert_eq!(project.graph().nodes()[0].data.inputs[0].label, "gain");

    let stale = project.apply_graph_command(
        GraphCommand::UpdateShaderCode {
            node_id: NodeId::new(node.id),
            shader_code: String::new(),
        },
        revision,
    );
    assert_eq!(stale.unwrap_err().code, SdkErrorCode::StaleRevision);
    assert_eq!(project.graph_revision(), revision + 1);
}

#[test]
fn project_undo_and_redo_are_rust_owned() {
    let mut project = empty_project();
    let (node, _) = project
        .create_graph_node(
            NodeFactoryRequest::Input {
                position: None,
                data_type: DataType::Float,
                input_mode: Some(InputMode::System),
            },
            0,
        )
        .unwrap();
    let revision = project.graph_revision();

    project
        .apply_graph_command(
            GraphCommand::RemoveNode {
                node_id: NodeId::new(node.id.clone()),
            },
            revision,
        )
        .unwrap();
    assert!(project.graph().nodes().is_empty());

    project.rollback_graph(project.graph_revision()).unwrap();
    assert_eq!(project.graph().nodes()[0].id, node.id);
    project.redo_graph(project.graph_revision()).unwrap();
    assert!(project.graph().nodes().is_empty());
}
