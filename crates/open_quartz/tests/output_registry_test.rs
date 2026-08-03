use open_quartz::ffi::{EngineState, SdkErrorCode};
use open_quartz::runtime::CompositionClock;

#[test]
fn composition_clock_freezes_pause_and_resets_stop_without_deadline_drift() {
    let mut clock = CompositionClock::new(16);
    clock.start(100);
    let first = clock.tick(116).unwrap();
    assert_eq!(first.timeline_ns, 16);
    assert_eq!(first.next_deadline_ns, 100);
    clock.pause(140).unwrap();
    assert_eq!(
        clock.tick(1_000).unwrap_err().code,
        SdkErrorCode::InvalidState
    );
    clock.resume(1_000).unwrap();
    let resumed = clock.tick(1_016).unwrap();
    assert_eq!(resumed.timeline_ns, 56);
    assert_eq!(resumed.frame, 2);
    assert_eq!(resumed.next_deadline_ns, 1_000);
    clock.stop();
    assert_eq!(clock.state().epoch, 2);
    assert_eq!(clock.state().timeline_ns, 0);
    assert_eq!(clock.state().frame, 0);
}

use open_quartz::runtime::{
    ContentStamp, DataPathMode, DeliveryPolicy, FrameStamp, OutputKey, OutputPayload, OutputState,
    OutputSubscription, OutputTransport, Runtime, RuntimeCapabilities, RuntimeFrameInput,
};
use open_quartz::types::Graph;
use serde_json::json;

fn graph() -> Graph {
    serde_json::from_value(json!({
        "nodes": [{
            "id": "math-1",
            "type": "math",
            "position": { "x": 0.0, "y": 0.0 },
            "data": {
                "type": "math",
                "label": "Add",
                "shaderCode": "",
                "inputs": [],
                "outputs": [{
                    "id": "result",
                    "label": "result",
                    "dataType": "float",
                    "direction": "output"
                }],
                "uniforms": { "a": 2.0, "b": 3.0 },
                "mathOp": "add"
            }
        }],
        "edges": []
    }))
    .unwrap()
}

fn subscription(id: &str, policy: DeliveryPolicy) -> OutputSubscription {
    OutputSubscription {
        subscription_id: id.to_owned(),
        output: OutputKey::new("math-1", "result"),
        delivery: policy,
        transport: OutputTransport::Value,
        max_width: None,
        max_height: None,
    }
}

fn state(generation: u64, value: f64) -> OutputState {
    let stamp = FrameStamp {
        epoch: 1,
        frame: generation,
        timeline_ns: generation,
        deadline_ns: generation + 1,
    };
    OutputState {
        output: OutputKey::new("math-1", "result"),
        graph_revision: 1,
        output_generation: generation,
        evaluation_stamp: stamp.clone(),
        content_stamp: ContentStamp {
            epoch: 1,
            timeline_ns: stamp.timeline_ns,
            media_pts_ns: None,
        },
        payload: OutputPayload::Float(value),
    }
}

#[test]
fn latest_overwrites_pending_but_every_applies_backpressure() {
    let mut runtime = Runtime::new(RuntimeCapabilities {
        data_paths: vec![DataPathMode::CpuCopy],
    });
    runtime.set_graph(&graph()).unwrap();
    runtime
        .subscribe_output(subscription("latest", DeliveryPolicy::Latest))
        .unwrap();
    runtime
        .subscribe_output(subscription("every", DeliveryPolicy::Every))
        .unwrap();
    runtime.set_every_queue_capacity(2);

    runtime.publish_output(state(1, 1.0)).unwrap();
    runtime.publish_output(state(2, 2.0)).unwrap();
    let error = runtime.publish_output(state(3, 3.0)).unwrap_err();
    assert_eq!(error.message, "Output subscription backpressure");

    let batch = runtime.drain_deliveries();
    let latest = batch
        .deliveries
        .iter()
        .filter(|item| item.subscription_id == "latest")
        .collect::<Vec<_>>();
    let every = batch
        .deliveries
        .iter()
        .filter(|item| item.subscription_id == "every")
        .collect::<Vec<_>>();
    assert_eq!(latest.len(), 1);
    assert_eq!(latest[0].state.output_generation, 2);
    assert_eq!(
        every
            .iter()
            .map(|item| item.state.output_generation)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn graph_change_invalidates_subscriptions_to_removed_ports() {
    let mut runtime = Runtime::new(RuntimeCapabilities { data_paths: vec![] });
    runtime.set_graph(&graph()).unwrap();
    runtime
        .subscribe_output(subscription("inspector", DeliveryPolicy::OnChange))
        .unwrap();
    runtime
        .set_graph(&Graph {
            nodes: vec![],
            edges: vec![],
        })
        .unwrap();

    let batch = runtime.drain_deliveries();
    assert_eq!(batch.invalidations.len(), 1);
    assert_eq!(batch.invalidations[0].subscription_id, "inspector");
    assert_eq!(batch.invalidations[0].reason, "output-removed");
}

#[test]
fn math_result_is_published_using_its_real_port_id() {
    let mut runtime = Runtime::new(RuntimeCapabilities { data_paths: vec![] });
    runtime.set_graph(&graph()).unwrap();
    runtime
        .subscribe_output(subscription("math", DeliveryPolicy::OnChange))
        .unwrap();
    runtime
        .advance(&RuntimeFrameInput {
            time: 0.0,
            delta: 0.0,
            frame: 1,
            date: [2026.0, 8.0, 3.0, 0.0],
            mouse: [0.0; 4],
            resolution: [640.0, 360.0, 1.0],
        })
        .unwrap();

    let batch = runtime.drain_deliveries();
    assert_eq!(batch.deliveries.len(), 1);
    assert_eq!(
        batch.deliveries[0].state.output,
        OutputKey::new("math-1", "result")
    );
    assert_eq!(batch.deliveries[0].state.payload, OutputPayload::Float(5.0));
}

#[test]
fn invalid_output_and_duplicate_subscription_are_rejected() {
    let mut runtime = Runtime::new(RuntimeCapabilities { data_paths: vec![] });
    runtime.set_graph(&graph()).unwrap();
    runtime
        .subscribe_output(subscription("same", DeliveryPolicy::Latest))
        .unwrap();
    assert!(runtime
        .subscribe_output(subscription("same", DeliveryPolicy::Latest))
        .is_err());

    let mut missing = subscription("missing", DeliveryPolicy::Latest);
    missing.output.port_id = "absent".to_owned();
    assert!(runtime.subscribe_output(missing).is_err());
}
