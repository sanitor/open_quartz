use super::*;
fn setup() -> (InferenceTasks, FrameExecutionSession) {
    let graph: Graph = serde_json::from_value(serde_json::json!({"nodes":[{
        "id":"model","type":"inference","position":{"x":0,"y":0},"data":{
            "type":"inference","label":"Model","shaderCode":"","inputs":[],"uniforms":{},
            "outputs":[{"id":"out","label":"image","dataType":"sampler2D","direction":"output"}]}
    }],"edges":[]})).unwrap();
    let mut playback = FrameExecutionSession::new_native(crate::outputs::PlaybackCapabilities { data_paths: vec![] });
    playback.set_graph(&graph).unwrap();
    let mut tasks = InferenceTasks::new(); tasks.sync_graph(&graph, &mut playback).unwrap();
    (tasks, playback)
}
fn pending(tasks: &mut InferenceTasks, playback: &FrameExecutionSession, id: u64) -> Completion {
    let active = Active { task_id: id, session_generation: SessionGeneration(7), revision: playback.revision(),
        generation: playback.node_generation("model").unwrap() };
    tasks.active.insert("model".to_owned(), active);
    Completion { node_id: "model".to_owned(), task_id: id,
        stamp: RequestStamp { request_id: id, session_generation: active.session_generation },
        revision: active.revision, generation: active.generation,
        input_stamp: FrameStamp { epoch: 1, frame: 9001, timeline_ns: 456, deadline_ns: 0 },
        result: Err("must not surface obsolete failure".to_owned()) }
}
#[test]
fn stale_request_revision_node_and_session_generations_never_clear_current_slot() {
    let (mut tasks, playback) = setup();
    for alter in 0..5 {
        let mut completion = pending(&mut tasks, &playback, 72);
        match alter { 0 => completion.task_id = 71, 1 => completion.stamp.request_id = 71,
            2 => completion.stamp.session_generation = SessionGeneration(6),
            3 => completion.revision = completion.revision.wrapping_add(1),
            _ => completion.generation = completion.generation.wrapping_add(1) }
        tasks.completions.push_back(completion);
        assert!(tasks.next_completion(&playback).unwrap().is_none());
        assert_eq!(tasks.active.get("model").unwrap().task_id, 72);
        assert!(tasks.take_output_events().is_empty());
    }
    assert_eq!(tasks.metrics().stale, 5);
    assert_eq!(tasks.metrics().failed, 0);
}
#[test]
fn cancel_unload_stop_and_clear_release_slots_without_restoring_old_work() {
    let (mut tasks, mut playback) = setup();
    let old = pending(&mut tasks, &playback, 1);
    playback.stop().unwrap(); tasks.cancel_pending();
    assert!(!tasks.has_pending());
    tasks.completions.push_back(old);
    assert!(tasks.next_completion(&playback).unwrap().is_none());
    let old = pending(&mut tasks, &playback, 2);
    tasks.unload("model"); tasks.completions.push_back(old);
    assert!(tasks.next_completion(&playback).unwrap().is_none());
    let old = pending(&mut tasks, &playback, 3);
    playback.dispose().unwrap(); tasks.clear(); tasks.completions.push_back(old);
    assert!(tasks.next_completion(&playback).unwrap().is_none());
    assert!(tasks.active.is_empty()); assert!(tasks.resources.is_empty());
    assert!(tasks.outputs.is_empty()); assert!(tasks.event_subscriptions.is_empty());
    assert!(tasks.take_output_events().is_empty());
    assert_eq!(tasks.metrics().cancelled, 3);
    assert_eq!(tasks.metrics().readback_bytes, 0); assert_eq!(tasks.metrics().upload_bytes, 0);
}
#[test]
fn current_failure_releases_slot_and_coalesces_only_its_node() {
    let (mut tasks, playback) = setup();
    let completion = pending(&mut tasks, &playback, 9);
    tasks.queued_nodes.insert("model".to_owned()); tasks.completions.push_back(completion);
    assert!(tasks.next_completion(&playback).err().unwrap().contains("Inference model failed"));
    assert!(!tasks.has_pending()); assert_eq!(tasks.take_ready_nodes(), ["model"]);
    assert!(tasks.take_ready_nodes().is_empty()); assert_eq!(tasks.metrics().failed, 1);
}
