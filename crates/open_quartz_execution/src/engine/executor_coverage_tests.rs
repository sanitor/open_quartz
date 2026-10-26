use super::*;
use crate::engine::TargetSpec;
use serde_json::json;

fn node(id: &str, kind: &str) -> ProjectNode {
    serde_json::from_value(json!({
        "id": id, "type": kind, "position": {"x": 0.0, "y": 0.0},
        "data": {"type": kind, "label": id, "shaderCode": if kind == "shader" || kind == "constant" { "@fragment fn main() -> @location(0) vec4f { return vec4f(1.0); }" } else { "" }, "inputs": [], "outputs": [], "uniforms": {}, "mathOp": "add"}
    })).unwrap()
}

fn inputs() -> FrameInputs {
    FrameInputs {
        time: 1.5,
        delta: 0.25,
        frame: 7,
        date: [1.0, 2.0, 3.0, 4.0],
        mouse: [5.0, 6.0, 7.0, 8.0],
        resolution: [9.0, 10.0, 1.0],
        video_nodes: vec![],
    }
}

#[test]
fn executor_replace_preserve_and_defensive_missing_plan_node_paths() {
    let math = node("math", "math");
    let mut engine = ExecutionEngine::prepare(vec![math.clone()], vec![]);
    engine.math_values.insert("math".to_owned(), 3.0);
    engine.feedback_first_frame.insert("math".to_owned());
    engine.replace_graph_preserving_state(vec![math.clone()], vec![], &HashSet::new());
    assert_eq!(engine.math_values["math"], 3.0);
    engine.replace_graph(vec![node("constant", "constant")], vec![]);
    assert_eq!(engine.run_frame(&inputs()).commands[0].kind, "constant");

    let mut missing_plan = ExecutionEngine::prepare(vec![math.clone()], vec![]);
    missing_plan.plan.nodes.clear();
    assert!(missing_plan.run_frame(&inputs()).commands.is_empty());
    let mut missing_node = ExecutionEngine::prepare(vec![math], vec![]);
    missing_node.nodes.clear();
    assert!(missing_node.run_frame(&inputs()).commands.is_empty());
    let mut input_engine = ExecutionEngine::prepare(vec![node("input", "input")], vec![]);
    assert!(input_engine.run_frame(&inputs()).commands.is_empty());
}

#[test]
fn executor_private_uniform_system_builtin_and_json_helpers_cover_variants() {
    let mut systems = Vec::new();
    for (id, source) in [
        ("time", SystemSource::Time),
        ("delta", SystemSource::TimeDelta),
        ("frame", SystemSource::Frame),
        ("mouse", SystemSource::Mouse),
        ("resolution", SystemSource::Resolution),
    ] {
        let mut current = node(id, "input");
        current.data.system_source = Some(source);
        systems.push(current);
    }
    let mut literal = node("literal", "input");
    literal.data.inputs = serde_json::from_value(
        json!([{ "id":"value", "label":"value", "dataType":"float", "direction":"input" }]),
    )
    .unwrap();
    literal.data.uniforms.insert("value".to_owned(), json!(4.0));
    systems.push(node("bare-source", "input"));
    systems.push(literal);
    let mut engine = ExecutionEngine::prepare(systems, vec![]);
    engine.math_values.insert("math".to_owned(), 6.0);
    let frame = inputs();
    assert_eq!(engine.upstream_scalar("math", &frame), Some(vec![6.0]));
    assert_eq!(engine.upstream_scalar("time", &frame), Some(vec![1.5]));
    assert_eq!(engine.upstream_scalar("delta", &frame), Some(vec![0.25]));
    assert_eq!(engine.upstream_scalar("frame", &frame), Some(vec![7.0]));
    assert_eq!(
        engine.upstream_scalar("mouse", &frame),
        Some(frame.mouse.to_vec())
    );
    assert_eq!(
        engine.upstream_scalar("resolution", &frame),
        Some(frame.resolution.to_vec())
    );
    assert_eq!(engine.upstream_scalar("literal", &frame), Some(vec![4.0]));
    assert_eq!(engine.upstream_scalar("missing", &frame), None);
    assert_eq!(engine.upstream_scalar("bare-source", &frame), None);

    let bare_plan = NodeExecutionPlan {
        id: "bare".to_owned(),
        node_type: NodeType::Math,
        upstream: BTreeMap::new(),
        builtin_ports: vec![],
        target: None,
        shader: None,
        validation_errors: vec![],
        feedback: false,
    };
    assert!(engine
        .resolve_uniforms(&bare_plan, &node("bare", "math"), &frame)
        .is_empty());
    for (name, expected) in [
        ("iTime", vec![1.5]),
        ("iTimeDelta", vec![0.25]),
        ("iFrame", vec![7.0]),
        ("iDate", frame.date.to_vec()),
        ("iMouse", frame.mouse.to_vec()),
        ("iResolution", frame.resolution.to_vec()),
        ("unknown", vec![0.0]),
    ] {
        assert_eq!(builtin_value(name, &bare_plan, &frame), expected);
    }
    let target_plan = NodeExecutionPlan {
        target: Some(TargetSpec {
            width: 20,
            height: 30,
            float: false,
        }),
        ..bare_plan.clone()
    };

    let mut shader_node = node("uniforms", "shader");
    shader_node.data.inputs = serde_json::from_value(json!([
        {"id":"missing-upstream","label":"missingUpstream","dataType":"float","direction":"input"},
        {"id":"missing-literal","label":"missingLiteral","dataType":"float","direction":"input"}
    ]))
    .unwrap();
    shader_node.data.shader_code = "@fragment fn main() -> @location(0) vec4f { return vec4f(missingUpstream + missingLiteral); }".to_owned();
    let uniform_engine = ExecutionEngine::prepare(vec![shader_node.clone()], vec![]);
    let mut shader_plan = uniform_engine.plan.nodes[0].clone();
    shader_plan
        .upstream
        .insert("missingUpstream".to_owned(), "absent".to_owned());
    let uniforms = uniform_engine.resolve_uniforms(&shader_plan, &shader_node, &frame);
    assert_eq!(uniforms["missingUpstream"], vec![0.0]);
    assert_eq!(uniforms["missingLiteral"], vec![0.0]);
    assert_eq!(
        builtin_value("iResolution", &target_plan, &frame),
        vec![20.0, 30.0, 1.0]
    );

    assert_eq!(json_value_to_f32(&json!([1.0, "x"])), vec![1.0, 0.0]);
    assert_eq!(json_value_to_f32(&json!(true)), vec![1.0]);
    assert_eq!(json_value_to_f32(&json!(2.0)), vec![2.0]);
    assert_eq!(json_value_to_f32(&json!("3.0")), vec![3.0]);
    assert_eq!(json_value_to_f32(&Value::Null), vec![0.0]);

    let command = command_for(&bare_plan, "math", None, BTreeMap::new(), None, false, None);
    assert!(command.texture_inputs.is_empty());
}
