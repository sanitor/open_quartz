use std::collections::HashSet;

use open_quartz::engine::{ExecutionEngine, FrameInputs};
use open_quartz::types::{Edge, ProjectNode};
use serde_json::json;

fn parse_nodes(value: serde_json::Value) -> Vec<ProjectNode> {
    serde_json::from_value(value).unwrap()
}

fn parse_edges(value: serde_json::Value) -> Vec<Edge> {
    serde_json::from_value(value).unwrap()
}

fn frame(frame: u64) -> FrameInputs {
    FrameInputs {
        time: frame as f64 / 60.0,
        delta: 1.0 / 60.0,
        frame,
        date: [2026.0, 7.0, 29.0, 0.0],
        mouse: [1.0, 2.0, 0.0, 0.0],
        resolution: [640.0, 360.0, 1.0],
        video_nodes: Vec::new(),
    }
}

#[test]
fn builds_plan_with_default_size_upstream_and_renderer_output() {
    let nodes = parse_nodes(json!([
        {
            "id": "image", "type": "input", "position": {"x": 0.0, "y": 0.0},
            "data": {"type": "input", "label": "Image", "shaderCode": "", "inputs": [],
                "outputs": [{"id": "image_out", "label": "output", "dataType": "sampler2D", "direction": "output"}],
                "uniforms": {}, "inputMode": "image", "inputDataType": "sampler2D", "imageWidth": 640, "imageHeight": 360}
        },
        {
            "id": "shader", "type": "shader", "position": {"x": 1.0, "y": 0.0},
            "data": {"type": "shader", "label": "Shader",
                "shaderCode": "@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f { return textureSample(inputImage, inputImageSampler, v_uv); }",
                "inputs": [{"id": "shader_in", "label": "inputImage", "dataType": "sampler2D", "direction": "input"}],
                "outputs": [{"id": "shader_out", "label": "fragColor", "dataType": "vec4", "direction": "output"}], "uniforms": {}}
        },
        {
            "id": "renderer", "type": "renderer", "position": {"x": 2.0, "y": 0.0},
            "data": {"type": "renderer", "label": "Renderer", "shaderCode": "",
                "inputs": [{"id": "renderer_in", "label": "inputImage", "dataType": "sampler2D", "direction": "input"}],
                "outputs": [], "uniforms": {}}
        }
    ]));
    let edges = parse_edges(json!([
        {"id": "e1", "source": "image", "sourceHandle": "image_out", "target": "shader", "targetHandle": "shader_in"},
        {"id": "e2", "source": "shader", "sourceHandle": "shader_out", "target": "renderer", "targetHandle": "renderer_in"}
    ]));

    let engine = ExecutionEngine::prepare(nodes, edges);
    let plan = engine.plan();
    assert_eq!(plan.sorted_ids, ["image", "shader", "renderer"]);
    assert_eq!((plan.default_width, plan.default_height), (640, 360));
    assert_eq!(plan.output_nodes, ["renderer"]);
    let shader = plan.nodes.iter().find(|node| node.id == "shader").unwrap();
    assert_eq!(shader.upstream["inputImage"], "image");
    assert_eq!(
        shader
            .target
            .as_ref()
            .map(|target| (target.width, target.height)),
        Some((640, 360))
    );
    assert!(shader.validation_errors.is_empty());
}

#[test]
fn static_graph_executes_once_then_stays_clean() {
    let nodes = parse_nodes(json!([{
        "id": "shader", "type": "shader", "position": {"x": 0.0, "y": 0.0},
        "data": {"type": "shader", "label": "Static",
            "shaderCode": "@fragment fn main() -> @location(0) vec4f { return vec4f(gain); }",
            "inputs": [{"id": "gain", "label": "gain", "dataType": "float", "direction": "input"}],
            "outputs": [], "uniforms": {"gain": 0.5}}
    }]));
    let mut engine = ExecutionEngine::prepare(nodes, Vec::new());

    let first = engine.run_frame(&frame(1));
    assert_eq!(first.commands.len(), 1);
    assert_eq!(first.commands[0].uniforms["gain"], [0.5]);
    assert!(engine.run_frame(&frame(2)).commands.is_empty());
}

#[test]
fn numeric_string_uniform_reaches_native_shader_command() {
    let nodes = parse_nodes(json!([{
        "id": "hue", "type": "shader", "position": {"x": 0.0, "y": 0.0},
        "data": {"type": "shader", "label": "Hue Rotate",
            "shaderCode": "@fragment fn main() -> @location(0) vec4f { return vec4f(angle); }",
            "inputs": [{"id": "angle", "label": "angle", "dataType": "float", "direction": "input"}],
            "outputs": [], "uniforms": {"angle": "2.094395"}}
    }]));
    let mut engine = ExecutionEngine::prepare(nodes, Vec::new());

    let work = engine.run_frame(&frame(1));

    assert_eq!(work.commands[0].uniforms["angle"], [2.094395]);
}

#[test]
fn dynamic_builtin_reruns_each_frame_and_resolves_uniform() {
    let nodes = parse_nodes(json!([{
        "id": "shader", "type": "shader", "position": {"x": 0.0, "y": 0.0},
        "data": {"type": "shader", "label": "Dynamic",
            "shaderCode": "@fragment fn main() -> @location(0) vec4f { return vec4f(iTime); }",
            "inputs": [{"id": "time", "label": "iTime", "dataType": "float", "direction": "input"}],
            "outputs": [], "uniforms": {}}
    }]));
    let mut engine = ExecutionEngine::prepare(nodes, Vec::new());

    let first = engine.run_frame(&frame(1));
    let second = engine.run_frame(&frame(2));
    assert_eq!(first.commands[0].uniforms["iTime"], [1.0 / 60.0]);
    assert_eq!(second.commands[0].uniforms["iTime"], [2.0 / 60.0]);
}

#[test]
fn connected_time_source_updates_shader_uniform_each_frame() {
    let nodes = parse_nodes(json!([
        {
            "id": "time", "type": "input", "position": {"x": 0.0, "y": 0.0},
            "data": {"type": "input", "label": "Time", "shaderCode": "",
                "inputs": [],
                "outputs": [{"id": "time_out", "label": "value", "dataType": "float", "direction": "output"}],
                "uniforms": {}, "inputMode": "system", "inputDataType": "float", "systemSource": "time"}
        },
        {
            "id": "hue", "type": "shader", "position": {"x": 1.0, "y": 0.0},
            "data": {"type": "shader", "label": "Hue Rotate",
                "shaderCode": "@fragment fn main() -> @location(0) vec4f { return vec4f(angle); }",
                "inputs": [{"id": "angle", "label": "angle", "dataType": "float", "direction": "input"}],
                "outputs": [], "uniforms": {}}
        }
    ]));
    let edges = parse_edges(json!([{
        "id": "time_to_hue", "source": "time", "sourceHandle": "time_out",
        "target": "hue", "targetHandle": "angle"
    }]));
    let mut engine = ExecutionEngine::prepare(nodes, edges);

    let first = engine.run_frame(&frame(120));
    let second = engine.run_frame(&frame(180));

    assert_eq!(first.commands[0].uniforms["angle"], [2.0]);
    assert_eq!(second.commands[0].uniforms["angle"], [3.0]);
}

#[test]
fn feedback_swaps_read_write_targets_and_only_clears_first_frame() {
    let nodes = parse_nodes(json!([{
        "id": "feedback", "type": "shader", "position": {"x": 0.0, "y": 0.0},
        "data": {"type": "shader", "label": "Feedback",
            "shaderCode": "@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f { return textureSample(previousFrame, previousFrameSampler, v_uv); }",
            "inputs": [], "outputs": [], "uniforms": {}}
    }]));
    let mut engine = ExecutionEngine::prepare(nodes, Vec::new());

    let first = engine.run_frame(&frame(1));
    let second = engine.run_frame(&frame(2));
    assert_eq!(
        (
            first.commands[0].feedback_read_index,
            first.commands[0].feedback_write_index
        ),
        (Some(0), Some(1))
    );
    assert!(first.commands[0].clear_feedback);
    assert_eq!(
        (
            second.commands[0].feedback_read_index,
            second.commands[0].feedback_write_index
        ),
        (Some(1), Some(0))
    );
    assert!(!second.commands[0].clear_feedback);
}

#[test]
fn position_only_graph_update_preserves_feedback_state() {
    let nodes = parse_nodes(json!([{
        "id": "feedback", "type": "shader", "position": {"x": 0.0, "y": 0.0},
        "data": {"type": "shader", "label": "Feedback",
            "shaderCode": "@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f { return textureSample(previousFrame, previousFrameSampler, v_uv); }",
            "inputs": [], "outputs": [], "uniforms": {}}
    }]));
    let mut engine = ExecutionEngine::prepare(nodes.clone(), Vec::new());
    let first = engine.run_frame(&frame(1));
    assert_eq!(first.commands[0].feedback_write_index, Some(1));

    let mut moved = nodes;
    moved[0].position.x = 100.0;
    engine.replace_graph_preserving_state(moved, Vec::new(), &HashSet::new());
    let next = engine.run_frame(&frame(2));
    assert_eq!(next.commands[0].feedback_read_index, Some(1));
    assert_eq!(next.commands[0].feedback_write_index, Some(0));
    assert!(!next.commands[0].clear_feedback);
}

#[test]
fn math_nodes_compute_scalar_outputs() {
    let nodes = parse_nodes(json!([
        {"id": "input", "type": "input", "position": {"x": 0.0, "y": 0.0},
            "data": {"type": "input", "label": "Input", "shaderCode": "",
                "inputs": [{"id": "value", "label": "value", "dataType": "float", "direction": "input"}],
                "outputs": [], "uniforms": {"value": 2.0}}},
        {"id": "math", "type": "math", "position": {"x": 1.0, "y": 0.0},
            "data": {"type": "math", "label": "Add", "shaderCode": "",
                "inputs": [
                    {"id": "math_a", "label": "a", "dataType": "auto", "direction": "input"},
                    {"id": "math_b", "label": "b", "dataType": "auto", "direction": "input"}
                ], "outputs": [], "uniforms": {"b": 3.0}, "mathOp": "add"}}
    ]));
    let edges = parse_edges(json!([{
        "id": "e1", "source": "input", "sourceHandle": "value", "target": "math", "targetHandle": "math_a"
    }]));
    let mut engine = ExecutionEngine::prepare(nodes, edges);
    let result = engine.run_frame(&frame(1));
    let math = result
        .commands
        .iter()
        .find(|command| command.node_id == "math")
        .unwrap();
    assert_eq!(math.scalar_output, Some(5.0));
}

#[test]
fn native_video_plan_uses_sampled_texture_while_browser_uses_external_texture() {
    let nodes = parse_nodes(json!([
        {
            "id": "video", "type": "input", "position": {"x": 0.0, "y": 0.0},
            "data": {
                "type": "input", "label": "Video", "shaderCode": "", "inputs": [],
                "outputs": [{"id": "video_out", "label": "output", "dataType": "sampler2D", "direction": "output"}],
                "uniforms": {}, "inputMode": "video", "inputDataType": "sampler2D"
            }
        },
        {
            "id": "shader", "type": "shader", "position": {"x": 1.0, "y": 0.0},
            "data": {
                "type": "shader", "label": "Copy",
                "shaderCode": "@fragment fn main(@location(0) uv: vec2f) -> @location(0) vec4f { return textureSample(inputImage, inputImageSampler, uv); }",
                "inputs": [{"id": "shader_in", "label": "inputImage", "dataType": "sampler2D", "direction": "input"}],
                "outputs": [], "uniforms": {}
            }
        }
    ]));
    let edges = parse_edges(json!([{
        "id": "e1", "source": "video", "sourceHandle": "video_out",
        "target": "shader", "targetHandle": "shader_in"
    }]));

    let browser = ExecutionEngine::prepare(nodes.clone(), edges.clone());
    let browser_shader = browser
        .plan()
        .nodes
        .iter()
        .find(|node| node.id == "shader")
        .unwrap()
        .shader
        .as_ref()
        .unwrap();
    assert!(browser_shader
        .external_texture_bindings
        .contains_key("inputImage"));

    let native = ExecutionEngine::prepare_with_options(nodes, edges, false);
    let native_shader = native
        .plan()
        .nodes
        .iter()
        .find(|node| node.id == "shader")
        .unwrap()
        .shader
        .as_ref()
        .unwrap();
    assert!(native_shader.external_texture_bindings.is_empty());
    assert!(native_shader.texture_bindings.contains_key("inputImage"));
}
