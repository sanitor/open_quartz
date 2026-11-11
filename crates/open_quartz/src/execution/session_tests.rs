use super::*;
use crate::outputs::{
    AsyncCompletionEnvelope, DeliveryPolicy, OutputKey, OutputPayload, OutputSubscription,
    OutputTransport,
};
use serde_json::json;

fn graph() -> Graph {
    serde_json::from_value(json!({"nodes":[{
        "id":"math","type":"math","position":{"x":0,"y":0},"data":{
            "type":"math","label":"sum","shaderCode":"","mathOp":"add","inputs":[],
            "outputs":[{"id":"out","label":"out","dataType":"float","direction":"output"}],
            "uniforms":{"a":2.0,"b":3.0}}}],"edges":[]}))
    .unwrap()
}
fn frame(epoch: u64, number: u64) -> FrameEvaluationInput {
    FrameEvaluationInput {
        stamp: FrameStamp {
            epoch,
            frame: number,
            timeline_ns: number * 100,
            deadline_ns: 9000,
        },
        previous_timeline_ns: number.saturating_sub(1) * 100,
        date: [0.0; 4],
        mouse: [0.0; 4],
        resolution: [2.0, 2.0, 1.0],
    }
}
fn subscription(id: &str, delivery: DeliveryPolicy) -> OutputSubscription {
    OutputSubscription {
        subscription_id: id.into(),
        output: OutputKey::new("math", "out"),
        delivery,
        transport: OutputTransport::Value,
        max_width: None,
        max_height: None,
    }
}
#[test]
fn no_clock_evaluation_and_every_rejection_preserve_graph_until_retry() {
    let mut session =
        FrameExecutionSession::new_native(PlaybackCapabilities { data_paths: vec![] });
    session.set_graph(&graph()).unwrap();
    session.set_every_queue_capacity(1);
    session
        .subscribe_output(subscription("all", DeliveryPolicy::Every))
        .unwrap();
    session
        .subscribe_output(subscription("latest", DeliveryPolicy::Latest))
        .unwrap();
    session.evaluate(&frame(7, 1)).unwrap();
    let first = session.drain_commands();
    assert_eq!(first[0].scalar_output, Some(5.0));
    let before = session.last_evaluation().cloned();
    assert!(session
        .evaluate_with_dirty(&frame(7, 2), &["math"])
        .is_err());
    assert_eq!(session.last_evaluation(), before.as_ref());
    assert!(session.drain_commands().is_empty());
    let deliveries = session.drain_deliveries();
    assert_eq!(deliveries.deliveries.len(), 2);
    assert!(deliveries
        .deliveries
        .iter()
        .all(|d| d.state.content_stamp.timeline_ns == 100
            && d.state.payload == OutputPayload::Float(5.0)));
    session
        .evaluate_with_dirty(&frame(7, 2), &["math"])
        .unwrap();
    assert_eq!(session.drain_commands()[0].scalar_output, Some(5.0));
    let delivered = session.drain_deliveries();
    assert!(delivered
        .deliveries
        .iter()
        .all(|d| d.state.evaluation_stamp == frame(7, 2).stamp));
    session
        .evaluate_with_dirty(&frame(7, 3), &["math"])
        .unwrap();
    session.unsubscribe_output("all").unwrap();
    session
        .evaluate_with_dirty(&frame(7, 4), &["math"])
        .unwrap();
    let latest = session.drain_deliveries().deliveries;
    assert_eq!(latest.len(), 1);
    assert_eq!(latest[0].state.evaluation_stamp.frame, 4);
}
#[test]
fn malformed_and_stale_explicit_frames_do_not_mutate_last_result() {
    let mut session =
        FrameExecutionSession::new_native(PlaybackCapabilities { data_paths: vec![] });
    assert!(session.evaluate(&frame(1, 1)).is_err());
    session.set_graph(&graph()).unwrap();
    session.evaluate(&frame(4, 1)).unwrap();
    session.drain_commands();
    for mut bad in [
        frame(3, 2),
        frame(4, 0),
        frame(4, 2),
        frame(4, 3),
        frame(4, 4),
    ] {
        if bad.stamp.frame == 2 && bad.stamp.epoch == 4 {
            bad.previous_timeline_ns = bad.stamp.timeline_ns + 1;
        }
        if bad.stamp.frame == 3 {
            bad.resolution[0] = 0.0;
        }
        if bad.stamp.frame == 4 {
            bad.mouse[0] = f32::NAN;
        }
        assert!(session.evaluate(&bad).is_err());
        assert_eq!(session.last_evaluation(), Some(&frame(4, 1).stamp));
        assert!(session.drain_commands().is_empty());
    }
    session.pause().unwrap();
    assert!(session.evaluate(&frame(4, 5)).is_err());
    session.resume().unwrap();
    session.evaluate(&frame(4, 5)).unwrap();
    session.dispose().unwrap();
    assert!(session.evaluate(&frame(4, 6)).is_err());
}
#[test]
fn completion_epoch_is_checked_before_publication_and_dirty_changes() {
    let mut session =
        FrameExecutionSession::new_native(PlaybackCapabilities { data_paths: vec![] });
    session.set_graph(&graph()).unwrap();
    session
        .subscribe_output(subscription("value", DeliveryPolicy::Latest))
        .unwrap();
    session.evaluate(&frame(5, 1)).unwrap();
    session.drain_commands();
    session.drain_deliveries();
    let completion = AsyncCompletionEnvelope {
        node_id: "math".into(),
        graph_revision: session.revision(),
        node_generation: session.node_generation("math").unwrap(),
        input_stamp: frame(4, 1).stamp,
        content_stamp: ContentStamp {
            epoch: 4,
            timeline_ns: 100,
            media_pts_ns: None,
        },
        outputs: vec![(OutputKey::new("math", "out"), OutputPayload::Float(9.0))],
    };
    assert!(session.submit_completion(completion.clone()).is_err());
    assert!(session.drain_deliveries().deliveries.is_empty());
    let mut current = completion;
    current.input_stamp.epoch = 5;
    session.submit_completion(current).unwrap();
    assert_eq!(
        session.drain_deliveries().deliveries[0].state.payload,
        OutputPayload::Float(9.0)
    );
}
#[test]
fn new_epoch_resets_feedback_but_same_epoch_preserves_history() {
    let graph:Graph=serde_json::from_value(json!({"nodes":[{"id":"feedback","type":"shader","position":{"x":0,"y":0},"data":{
        "type":"shader","label":"history","shaderCode":"@fragment fn main(@location(0) uv:vec2f)->@location(0) vec4f{return textureSample(previousFrame,previousFrameSampler,uv);}",
        "inputs":[],"outputs":[],"uniforms":{},"autoSize":false,"width":2,"height":2}}],"edges":[]})).unwrap();
    let mut session =
        FrameExecutionSession::new_native(PlaybackCapabilities { data_paths: vec![] });
    session.set_graph(&graph).unwrap();
    session.evaluate(&frame(1, 1)).unwrap();
    assert!(session.drain_commands()[0].clear_feedback);
    session.evaluate(&frame(1, 2)).unwrap();
    assert!(!session.drain_commands()[0].clear_feedback);
    session.evaluate(&frame(2, 1)).unwrap();
    assert!(session.drain_commands()[0].clear_feedback);
    session.stop().unwrap();
    session.set_graph(&graph).unwrap();
    session.evaluate(&frame(3, 1)).unwrap();
    assert!(session.drain_commands()[0].clear_feedback);
}
