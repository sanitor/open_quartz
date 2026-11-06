use super::*;
use serde_json::{json, Value};

fn graph_value() -> Value {
    json!({
        "nodes": [
            {"id":"a","type":"math","position":{"x":0.0,"y":0.0},"data":{"type":"math","label":"A","shaderCode":"","mathOp":"add","inputs":[],"outputs":[{"id":"out","label":"out","dataType":"float","direction":"output"}],"uniforms":{}}},
            {"id":"b","type":"math","position":{"x":0.0,"y":0.0},"data":{"type":"math","label":"B","shaderCode":"","mathOp":"add","inputs":[{"id":"in","label":"in","dataType":"float","direction":"input"}],"outputs":[{"id":"out","label":"out","dataType":"float","direction":"output"}],"uniforms":{}}}
        ],
        "edges": [{"id":"e","source":"a","target":"b","sourceHandle":"out","targetHandle":"in"}]
    })
}

#[test]
fn state_helpers_detect_graph_node_edge_and_removal_changes() {
    let first: Graph = serde_json::from_value(graph_value()).unwrap();
    assert_eq!(changed_node_ids(None, &first), vec!["a", "b"]);
    assert!(removed_node_ids(None, &first).is_empty());
    assert_eq!(edge_signature(&first.edges[0]), ("a", "out", "in"));

    let mut positioned = graph_value();
    positioned["nodes"][0]["position"]["x"] = json!(99.0);
    let positioned: Graph = serde_json::from_value(positioned).unwrap();
    assert!(changed_node_ids(Some(&first), &positioned).is_empty());

    let mut uniform = graph_value();
    uniform["nodes"][0]["data"]["uniforms"]["gain"] = json!(0.5);
    let uniform: Graph = serde_json::from_value(uniform).unwrap();
    assert_eq!(changed_node_ids(Some(&first), &uniform), vec!["a"]);

    let mut edge = graph_value();
    edge["edges"][0]["targetHandle"] = json!("other");
    let edge: Graph = serde_json::from_value(edge).unwrap();
    assert_eq!(changed_node_ids(Some(&first), &edge), vec!["b"]);

    let empty: Graph = serde_json::from_value(json!({"nodes":[],"edges":[]})).unwrap();
    assert_eq!(removed_node_ids(Some(&first), &empty), vec!["a", "b"]);
    assert!(node_changed(Some(&first), &first, &first.nodes[0]) == false);
}

#[test]
fn engine_invalid_states_unknown_nodes_video_json_and_disposal_are_structured() {
    let mut engine = Engine::default();
    assert_eq!(engine_state_name(EngineState::Empty), "empty");
    assert_eq!(engine_state_name(EngineState::Ready), "ready");
    assert_eq!(engine_state_name(EngineState::Running), "running");
    assert_eq!(engine_state_name(EngineState::Paused), "paused");
    assert_eq!(engine_state_name(EngineState::Stopped), "stopped");
    assert_eq!(engine_state_name(EngineState::Disposed), "disposed");
    assert!(engine.pause().is_err());
    assert!(engine.resume().is_err());
    assert!(engine.stop().is_err());
    assert!(engine.mark_dirty("missing").is_err());
    assert!(engine.mark_dependents_dirty("missing").is_err());
    assert!(engine.node_generation("missing").is_err());
    assert!(engine.set_video_nodes_json("not-json").is_err());
    assert!(engine
        .run_frame(0.0, 0.0, 0, &[0.0; 4], &[0.0; 4], &[1.0; 3])
        .is_err());

    let graph = graph_value().to_string();
    engine.set_graph_json(&graph).unwrap();
    assert!(engine.execution_plan().is_some());
    assert!(engine.set_video_nodes_json("[\"missing\"]").is_err());
    engine.set_video_nodes_json("[\"a\"]").unwrap();
    engine.mark_dependents_dirty("a").unwrap();
    assert!(engine
        .run_frame(0.0, 0.0, 1, &[0.0; 4], &[0.0; 3], &[1.0; 3])
        .is_err());
    assert!(engine
        .run_frame(0.0, 0.0, 1, &[0.0; 4], &[0.0; 4], &[1.0; 2])
        .is_err());
    engine
        .run_frame(0.0, 0.0, 1, &[0.0; 4], &[0.0; 4], &[1.0; 3])
        .unwrap();
    engine.pause().unwrap();
    engine.pause().unwrap();
    engine.resume().unwrap();
    assert!(engine.resume().is_err());
    engine.stop().unwrap();
    assert!(engine
        .run_frame(0.0, 0.0, 1, &[0.0; 4], &[0.0; 4], &[1.0; 3])
        .is_err());
    engine.dispose();
    engine.dispose();
    assert!(engine.set_graph_json(&graph).is_err());
    assert!(engine.set_video_nodes_json("[]").is_err());
    assert!(engine.pause().is_err());
    assert!(engine.resume().is_err());
    assert!(engine.stop().is_err());
    assert!(engine.node_generation("a").is_err());
    assert!(engine.mark_dirty("a").is_err());
    assert!(engine.mark_dependents_dirty("a").is_err());
    assert!(engine
        .run_frame(0.0, 0.0, 1, &[0.0; 4], &[0.0; 4], &[1.0; 3])
        .is_err());
}

struct AcceptFacade;

impl crate::engine::GpuFacade for AcceptFacade {
    fn execute(
        &mut self,
        _: &crate::engine::ExecutionPlan,
        _: &[crate::engine::ExecutionCommand],
    ) -> Result<(), SdkError> {
        Ok(())
    }
}

#[test]
fn engine_internal_missing_prepared_state_paths_are_explicit() {
    let mut engine = Engine::new();
    assert!(engine.execute_gpu(&mut AcceptFacade, &[]).is_err());
    engine.node_generations.insert("ghost".to_owned(), 1);
    assert!(engine.mark_dirty("ghost").is_err());
    assert!(engine.mark_dependents_dirty("ghost").is_err());
    assert!(engine.prepared_mut().is_err());
    engine.state = EngineState::Ready;
    assert!(engine
        .run_frame(0.0, 0.0, 1, &[0.0; 4], &[0.0; 4], &[1.0; 3])
        .is_err());
}
