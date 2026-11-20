use super::*;
use crate::player::{Environment, OpenQuartz};
use crate::project::test_support::*;

#[test]
fn node_factory_covers_every_executable_variant_and_math_descriptor() {
    let sdk = OpenQuartz::new(Environment::headless());
    let mut project = sdk.create_project("Factories");
    project
        .graph_mut()
        .nodes
        .push(node("legacy", NodeType::Shader, vec![], vec![]));
    project
        .graph_mut()
        .nodes
        .push(node("shader_bad", NodeType::Shader, vec![], vec![]));
    project
        .graph_mut()
        .nodes
        .push(node("shader_9", NodeType::Shader, vec![], vec![]));
    project
        .create_graph_node(
            NodeFactoryRequest::Shader {
                position: None,
                code: "@fragment fn main() -> @location(0) vec4f { return vec4f(1.0); }".to_owned(),
                label: "Factory Shader".to_owned(),
                template_name: None,
                shader_template_id: None,
            },
            0,
        )
        .unwrap();

    let (sampler, _) = project
        .create_graph_node(
            NodeFactoryRequest::Input {
                position: None,
                data_type: DataType::Sampler2d,
                input_mode: None,
            },
            project.graph_revision(),
        )
        .unwrap();
    assert_eq!(sampler.data.input_mode, Some(InputMode::Image));
    let (scalar, _) = project
        .create_graph_node(
            NodeFactoryRequest::Input {
                position: None,
                data_type: DataType::Float,
                input_mode: None,
            },
            project.graph_revision(),
        )
        .unwrap();
    assert_eq!(scalar.data.input_mode, None);

    for source in [
        SystemSource::Time,
        SystemSource::TimeDelta,
        SystemSource::Frame,
        SystemSource::Mouse,
        SystemSource::Resolution,
    ] {
        let (system, _) = project
            .create_graph_node(
                NodeFactoryRequest::System {
                    position: None,
                    source,
                },
                project.graph_revision(),
            )
            .unwrap();
        assert_eq!(system.data.system_source, Some(source));
    }

    project
        .create_graph_node(
            NodeFactoryRequest::Constant { position: None },
            project.graph_revision(),
        )
        .unwrap();
    project
        .create_graph_node(
            NodeFactoryRequest::Inference {
                position: None,
                label: "Detector".to_owned(),
                template_name: None,
                model_id: Some("model".to_owned()),
                catalog_id: Some("catalog".to_owned()),
                inputs: vec![port("image", DataType::Sampler2d, PortDirection::Input)],
                outputs: vec![port("boxes", DataType::Roi, PortDirection::Output)],
            },
            project.graph_revision(),
        )
        .unwrap();
    project
        .create_graph_node(
            NodeFactoryRequest::CustomInference { position: None },
            project.graph_revision(),
        )
        .unwrap();

    let math_ops = [
        "add",
        "subtract",
        "multiply",
        "divide",
        "negate",
        "modulo",
        "min",
        "max",
        "clamp",
        "saturate",
        "step",
        "smoothstep",
        "abs",
        "sign",
        "sin",
        "cos",
        "tan",
        "asin",
        "acos",
        "atan",
        "pow",
        "sqrt",
        "exp",
        "log",
        "mix",
        "floor",
        "ceil",
        "round",
        "fract",
    ];
    for op in math_ops {
        let (math, _) = project
            .create_graph_node(
                NodeFactoryRequest::Math {
                    position: None,
                    op: op.to_owned(),
                },
                project.graph_revision(),
            )
            .unwrap();
        assert_eq!(math.data.math_op.as_deref(), Some(op));
    }
    assert_eq!(
        project
            .create_graph_node(
                NodeFactoryRequest::Math {
                    position: None,
                    op: "unknown".to_owned()
                },
                project.graph_revision(),
            )
            .unwrap_err()
            .code,
        SdkErrorCode::InvalidGraph
    );

    let mut invalid_project = sdk.create_project("Invalid internal graph");
    invalid_project.graph_mut().nodes.extend([
        node("duplicate", NodeType::Input, vec![], vec![]),
        node("duplicate", NodeType::Input, vec![], vec![]),
    ]);
    assert_eq!(
        invalid_project
            .create_graph_node(NodeFactoryRequest::Constant { position: None }, 0)
            .unwrap_err()
            .code,
        SdkErrorCode::InvalidGraph
    );

    assert_eq!(numeric_suffix("legacy"), "1");
    assert_eq!(normalize_label(" Two Words "), "two_words");
    let extra_ports = math_ports("math", 4, PortDirection::Input);
    assert_eq!(extra_ports[3].label, "value");
}

#[test]
fn corrector_factory_edits_history_and_persistence_keep_executable_semantics() {
    use crate::operators::{evaluate_color_corrector, ColorDomain};
    let sdk = OpenQuartz::new(Environment::headless());
    let mut project = sdk.create_project("Corrector");
    let (node, _) = project.create_graph_node(NodeFactoryRequest::ColorCorrector { position: None }, 0).unwrap();
    assert_eq!(node.node_type, NodeType::ColorCorrector);
    assert!(node.data.shader_code.is_empty());
    let mut data = node.data.clone();
    data.color_corrector.as_mut().unwrap().primaries.exposure = 1.0;
    project.apply_graph_command(GraphCommand::UpdateNodeData { node_id: NodeId::new(&node.id), data }, project.graph_revision()).unwrap();
    let saved = serde_json::to_value(project.to_file()).unwrap();
    let reopened = Project::try_from_file(serde_json::from_value(saved.clone()).unwrap()).unwrap();
    let config = reopened.graph().nodes()[0].data.color_corrector.as_ref().unwrap();
    let result = evaluate_color_corrector(config, ColorDomain::AcesCgLinear, [-0.5,0.25,2.0,0.4]).unwrap();
    for (actual,expected) in result.into_iter().zip([-1.0,0.5,4.0,0.4]) { assert!((actual-expected).abs()<1e-5); }
    project.rollback_graph(project.graph_revision()).unwrap();
    assert!(project.graph().nodes()[0].data.color_corrector.as_ref().unwrap().is_identity());
    project.redo_graph(project.graph_revision()).unwrap();
    assert_eq!(project.graph().nodes()[0].data.color_corrector.as_ref().unwrap().primaries.exposure,1.0);

    let before = project.to_file();
    let revision = project.graph_revision();
    let mut invalid = project.graph().nodes()[0].data.clone();
    invalid.color_corrector.as_mut().unwrap().primaries.gamma[0] = 0.0;
    assert!(project.apply_graph_command(GraphCommand::UpdateNodeData { node_id: NodeId::new(&node.id), data: invalid },revision).is_err());
    assert_eq!(project.graph_revision(), revision);
    assert_eq!(project.to_file(), before);
    let mut malformed = saved;
    malformed["graph"]["nodes"][0]["data"]["colorCorrector"]["algorithmVersion"] = serde_json::json!(900);
    assert!(serde_json::from_value::<ProjectFile>(malformed).is_err());
}

#[test]
fn corrector_rejects_payload_type_mismatch_shader_disguise_and_port_mutation() {
    let sdk = OpenQuartz::new(Environment::headless());
    let mut project = sdk.create_project("Corrector validation");
    let (node, _) = project.create_graph_node(NodeFactoryRequest::ColorCorrector { position: None }, 0).unwrap();
    for data in [
        NodeData { color_corrector: None, ..node.data.clone() },
        NodeData { node_type: NodeType::Shader, ..node.data.clone() },
        NodeData { shader_code: "@fragment fn main() {}".to_owned(), ..node.data.clone() },
        NodeData { inputs: vec![], ..node.data.clone() },
        NodeData { width: Some(512), ..node.data.clone() },
        NodeData { feedback_enabled: Some(true), ..node.data.clone() },
    ] {
        assert!(project.apply_graph_command(GraphCommand::UpdateNodeData { node_id: NodeId::new(&node.id), data }, project.graph_revision()).is_err());
    }
    let mut detached = node.data;
    detached.node_type = NodeType::Shader;
    assert!(serde_json::from_value::<NodeData>(serde_json::to_value(detached).unwrap()).is_err());
}
