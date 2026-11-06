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
            NodeFactoryRequest::Onnx {
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
            NodeFactoryRequest::CustomOnnx { position: None },
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
