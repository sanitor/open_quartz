use super::*;

#[test]
fn cancelled_or_replaced_load_cannot_change_a_new_project_with_the_same_node_id() {
    let (mut host, _) = pollster::block_on(NativePlayer::new()).unwrap();
    let mut runtime = InferenceRuntime::default();
    let cancelled = Arc::new(AtomicBool::new(false));
    runtime
        .requests
        .insert("model".into(), (7, cancelled.clone()));
    runtime.states.insert(
        "model".into(),
        LiveInferenceState {
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
    runtime.sync(&[], &mut host, true);
    runtime
        .requests
        .insert("model".into(), (9, Arc::new(AtomicBool::new(false))));
    runtime.states.insert(
        "model".into(),
        LiveInferenceState {
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
fn resident_texture_results_are_accepted_only_for_an_attached_session() {
    let mut runtime = InferenceRuntime::default();
    let output = InferenceOutputEvent {
        node_id: "model".into(),
        width: 32,
        height: 16,
        backend: "metal".into(),
        data: None,
    };
    runtime.output(output.clone());
    assert!(runtime.states.is_empty());
    runtime.states.insert(
        "model".into(),
        LiveInferenceState {
            attached: true,
            ..Default::default()
        },
    );
    runtime.output(output);
    let state = &runtime.states["model"];
    assert_eq!(state.status, "Inference output · 32×16");
    assert_eq!(state.result, "Resident GPU texture output · no host pixels or tensors");
    runtime.render_failed("inference failed");
    assert!(runtime.states["model"].result.is_empty());
    assert_eq!(
        runtime.states["model"].status,
        "Runtime stopped: inference failed"
    );
}

#[test]
fn model_messages_expose_their_node_identity() {
    let progress = ModelMessage::Progress {
        node_id: "progress".into(),
        ticket: 1,
        received: 2,
        total: 3,
    };
    let ready = ModelMessage::Ready {
        node_id: "ready".into(),
        ticket: 2,
        result: Err("failed".into()),
    };
    assert_eq!(progress.node_id(), "progress");
    assert_eq!(ready.node_id(), "ready");
}

#[test]
fn production_worker_rejects_host_policies_and_unsupported_tasks_before_model_io() {
    for (policy, task, model_id, expected) in [
        (ExecutionPolicy::default(), InferenceTask::SuperResolution, "super-resolution-3x", "hardware-resident"),
        (ExecutionPolicy::resident(), InferenceTask::DepthEstimation, "yolov8n", "requires task Detection"),
    ] {
        let mut runtime = InferenceRuntime::default();
        runtime.specs.insert("model".into(), ModelSpec {
            key: "model".into(), model_id: model_id.into(), task,
            path: None, target_size: 224, score: 0.5, iou: 0.45,
            policy, download: None,
        });
        let (sender, receiver) = mpsc::channel();
        runtime.load("model".into(), "/nonexistent/resident-model.onnx".into(), sender).unwrap();
        let Command::Model(ModelMessage::Ready { result, .. }) = receiver
            .recv_timeout(std::time::Duration::from_secs(10)).unwrap() else {
            panic!("worker must return model preparation result");
        };
        let error = result.err().expect("unsupported production configuration must fail");
        assert!(error.contains(expected), "{error}");
        assert!(!error.contains("ORT_DYLIB_PATH"), "{error}");
        assert!(!error.contains("Cannot read inference model"), "{error}");
    }
}
