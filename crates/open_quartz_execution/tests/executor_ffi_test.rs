use open_quartz_execution::engine::{api_version, capabilities_json, Engine, SDK_API_VERSION};
use serde_json::{json, Value};

fn graph_json() -> String {
    json!({
        "nodes": [{
            "id": "shader", "type": "shader", "position": {"x": 0.0, "y": 0.0},
            "data": {"type": "shader", "label": "Static", "shaderCode": "@fragment fn main() -> @location(0) vec4f { return vec4f(gain); }",
                "inputs": [{"id": "gain", "label": "gain", "dataType": "float", "direction": "input"}],
                "outputs": [], "uniforms": {"gain": 0.25}}
        }],
        "edges": []
    })
    .to_string()
}

#[test]
fn engine_contract_versions_capabilities_and_graph_lifecycle() {
    assert_eq!(api_version(), SDK_API_VERSION);
    let capabilities: Value = serde_json::from_str(&capabilities_json()).unwrap();
    assert_eq!(capabilities["structuredEngine"], true);
    assert_eq!(capabilities["graphPlanning"], true);
    assert_eq!(capabilities["gpuExecution"], false);

    let mut engine = Engine::new();
    assert_eq!(engine.engine_state(), "empty");
    assert_eq!(engine.set_graph_json(&graph_json()).unwrap(), 1);
    assert_eq!(engine.revision(), 1);
    assert_eq!(engine.engine_state(), "ready");
    engine.mark_dirty("shader").unwrap();

    let events: Value = serde_json::from_str(&engine.drain_events_json()).unwrap();
    assert_eq!(events[0], json!({"type": "graph-ready", "revision": 1}));
    assert_eq!(
        events[1],
        json!({"type": "resource-invalidated", "nodeId": "shader", "generation": 1})
    );
    assert_eq!(events[2], json!({"type": "state", "state": "ready"}));
    assert_eq!(engine.drain_events_json(), "[]");

    engine.dispose();
    assert_eq!(engine.engine_state(), "disposed");
    assert_eq!(
        serde_json::from_str::<Value>(&engine.mark_dirty("shader").unwrap_err()).unwrap()["code"],
        "disposed"
    );
}

#[test]
fn engine_rejects_invalid_graph_with_structured_error() {
    let mut engine = Engine::new();
    let error: Value = serde_json::from_str(&engine.set_graph_json("{}").unwrap_err()).unwrap();
    assert_eq!(error["code"], "invalid-graph");
    assert_eq!(error["message"], "Invalid graph JSON");
    assert!(error["details"].as_str().unwrap().contains("missing field"));
}

#[test]
fn typed_run_frame_keeps_commands_internal_and_emits_small_events() {
    let mut engine = Engine::new();
    engine.set_graph_json(&graph_json()).unwrap();
    engine.drain_events_json();

    engine
        .run_frame(
            1.0,
            1.0 / 60.0,
            42,
            &[2026.0, 7.0, 29.0, 0.0],
            &[1.0, 2.0, 0.0, 0.0],
            &[640.0, 360.0, 1.0],
        )
        .unwrap();

    assert_eq!(engine.engine_state(), "running");
    assert_eq!(engine.last_frame(), Some(42));
    assert_eq!(engine.pending_command_count(), 1);
    let events: Value = serde_json::from_str(&engine.drain_events_json()).unwrap();
    assert_eq!(events[0], json!({"type": "state", "state": "running"}));
    assert_eq!(
        events[1],
        json!({
            "type": "frame-planned", "frame": 42, "revision": 1,
            "commandCount": 1, "dirtyNodeCount": 1
        })
    );

    engine
        .run_frame(2.0, 1.0 / 60.0, 43, &[0.0; 4], &[0.0; 4], &[1.0; 3])
        .unwrap();
    assert_eq!(engine.pending_command_count(), 0);
    let events: Value = serde_json::from_str(&engine.drain_events_json()).unwrap();
    assert_eq!(events[0]["commandCount"], 0);

    let error = engine
        .run_frame(0.0, 0.0, 44, &[0.0; 3], &[0.0; 4], &[1.0; 3])
        .unwrap_err();
    assert_eq!(
        serde_json::from_str::<Value>(&error).unwrap()["code"],
        "invalid-frame"
    );
}

#[test]
fn lifecycle_and_resource_generations_track_semantic_graph_changes() {
    let mut engine = Engine::new();
    let graph = graph_json();
    engine.set_graph_json(&graph).unwrap();
    engine.drain_events_json();
    assert_eq!(engine.node_generation("shader").unwrap(), 1);

    let mut position_only: Value = serde_json::from_str(&graph).unwrap();
    position_only["nodes"][0]["position"]["x"] = json!(100.0);
    engine.set_graph_json(&position_only.to_string()).unwrap();
    assert_eq!(engine.node_generation("shader").unwrap(), 1);
    assert_eq!(
        serde_json::from_str::<Value>(&engine.drain_events_json()).unwrap(),
        json!([{"type": "graph-ready", "revision": 2}])
    );

    position_only["nodes"][0]["data"]["uniforms"]["gain"] = json!(0.75);
    engine.set_graph_json(&position_only.to_string()).unwrap();
    assert_eq!(engine.node_generation("shader").unwrap(), 2);
    let events: Value = serde_json::from_str(&engine.drain_events_json()).unwrap();
    assert_eq!(events[1]["type"], "resource-invalidated");
    assert_eq!(events[1]["generation"], 2);

    engine.set_graph_json(r#"{"nodes":[],"edges":[]}"#).unwrap();
    let events: Value = serde_json::from_str(&engine.drain_events_json()).unwrap();
    assert_eq!(events[1]["type"], "resource-released");
    assert_eq!(events[1]["generation"], 2);

    engine.set_graph_json(&graph).unwrap();
    engine.drain_events_json();
    engine
        .run_frame(0.0, 0.0, 1, &[0.0; 4], &[0.0; 4], &[1.0; 3])
        .unwrap();
    engine.pause().unwrap();
    assert_eq!(engine.engine_state(), "paused");
    assert_eq!(
        serde_json::from_str::<Value>(
            &engine
                .run_frame(0.0, 0.0, 2, &[0.0; 4], &[0.0; 4], &[1.0; 3])
                .unwrap_err()
        )
        .unwrap()["code"],
        "invalid-state"
    );
    engine.resume().unwrap();
    engine.stop().unwrap();
    assert_eq!(engine.engine_state(), "stopped");
}
