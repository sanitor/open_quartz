use super::*;

#[test]
fn cancelled_or_replaced_load_cannot_change_a_new_project_with_the_same_node_id() {
    let (mut host, _) = pollster::block_on(NativeHost::new()).unwrap();
    let mut runtime = OnnxRuntime::default();
    let cancelled = Arc::new(AtomicBool::new(false));
    runtime
        .requests
        .insert("model".into(), (7, cancelled.clone()));
    runtime.states.insert(
        "model".into(),
        LiveOnnxState {
            loading: true,
            status: "Loading model…".into(),
            ..Default::default()
        },
    );
    runtime.unload("model", &mut host);
    assert!(cancelled.load(Ordering::Acquire));
    runtime.receive(
        ModelMessage::Ready {
            node_id: "model".into(),
            ticket: 7,
            result: Err("late error".into()),
        },
        &mut host,
    );
    assert_eq!(
        runtime.states["model"].status,
        "Unloaded · no session attached"
    );
    assert!(!runtime.states["model"].loading);

    runtime
        .requests
        .insert("model".into(), (8, Arc::new(AtomicBool::new(false))));
    runtime.states.get_mut("model").unwrap().loading = true;
    runtime.sync(&Graph::default(), &mut host, true).unwrap();
    runtime
        .requests
        .insert("model".into(), (9, Arc::new(AtomicBool::new(false))));
    runtime.states.insert(
        "model".into(),
        LiveOnnxState {
            loading: true,
            status: "new load".into(),
            ..Default::default()
        },
    );
    runtime.receive(
        ModelMessage::Progress {
            node_id: "model".into(),
            ticket: 8,
            received: 100,
            total: 100,
        },
        &mut host,
    );
    runtime.receive(
        ModelMessage::Ready {
            node_id: "model".into(),
            ticket: 8,
            result: Err("old project error".into()),
        },
        &mut host,
    );
    assert_eq!(runtime.states["model"].status, "new load");
    assert!(runtime.states["model"].loading);
    assert_eq!(runtime.requests["model"].0, 9);
}

#[test]
fn live_structured_results_are_accepted_only_for_an_attached_session() {
    let mut runtime = OnnxRuntime::default();
    let output = NativeOutputEvent {
        node_id: "model".into(),
        width: 32,
        height: 16,
        backend: "cpu".into(),
        data: Some(serde_json::json!([{ "score": 0.9, "classId": 2 }])),
    };
    runtime.output(output.clone());
    assert!(runtime.states.is_empty());
    runtime.states.insert(
        "model".into(),
        LiveOnnxState {
            attached: true,
            ..Default::default()
        },
    );
    runtime.output(output);
    let state = &runtime.states["model"];
    assert_eq!(state.status, "Inference output · 32×16");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&state.result).unwrap()[0]["classId"],
        2
    );
    runtime.render_failed("inference failed");
    assert!(runtime.states["model"].result.is_empty());
    assert_eq!(
        runtime.states["model"].status,
        "Runtime stopped: inference failed"
    );
}
