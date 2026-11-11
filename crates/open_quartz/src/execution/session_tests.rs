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

#[test]
fn rejected_completions_preserve_values_generations_and_every_capacity_until_valid_retry() {
    let mut session =
        FrameExecutionSession::new_native(PlaybackCapabilities { data_paths: vec![] });
    session.set_graph(&graph()).unwrap();
    session.set_every_queue_capacity(1);
    session
        .subscribe_output(subscription("every", DeliveryPolicy::Every))
        .unwrap();
    session.evaluate(&frame(7, 1)).unwrap();
    session.drain_commands();
    let completion = AsyncCompletionEnvelope {
        node_id: "math".into(),
        graph_revision: session.revision(),
        node_generation: session.node_generation("math").unwrap(),
        input_stamp: frame(7, 2).stamp,
        content_stamp: ContentStamp {
            epoch: 7,
            timeline_ns: 150,
            media_pts_ns: Some(125),
        },
        outputs: vec![(OutputKey::new("math", "out"), OutputPayload::Float(9.0))],
    };
    assert!(session.validate_completion(&completion).is_err());
    assert!(session.submit_completion(completion.clone()).is_err());
    let first = session.drain_deliveries().deliveries;
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].state.payload, OutputPayload::Float(5.0));
    let generation = first[0].state.output_generation;
    for kind in 0..5 {
        let mut invalid = completion.clone();
        match kind {
            0 => invalid.graph_revision -= 1,
            1 => invalid.node_generation += 1,
            2 => invalid.node_id = "missing".into(),
            3 => invalid.outputs[0].0.port_id = "missing".into(),
            _ => invalid.input_stamp.epoch = 6,
        }
        assert!(session.validate_completion(&invalid).is_err());
        assert!(session.submit_completion(invalid).is_err());
        assert!(session.drain_deliveries().deliveries.is_empty());
        assert_eq!(session.last_evaluation(), Some(&frame(7, 1).stamp));
        assert!(session.drain_commands().is_empty());
    }
    session.validate_completion(&completion).unwrap();
    session.submit_completion(completion.clone()).unwrap();
    let delivered = session.drain_deliveries().deliveries;
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].state.payload, OutputPayload::Float(9.0));
    assert_eq!(delivered[0].state.output_generation, generation + 1);
    assert_eq!(delivered[0].state.evaluation_stamp, completion.input_stamp);
    assert_eq!(delivered[0].state.content_stamp, completion.content_stamp);
    let mut changed = graph();
    changed.nodes[0]
        .data
        .uniforms
        .insert("a".into(), json!(10.0));
    session.set_graph(&changed).unwrap();
    assert!(session.submit_completion(completion).is_err());
    session.evaluate(&frame(8, 1)).unwrap();
    assert_eq!(
        session.drain_deliveries().deliveries[0].state.payload,
        OutputPayload::Float(13.0)
    );
}

#[derive(Default)]
struct BackendFailures {
    present: std::sync::atomic::AtomicBool,
    remove: std::sync::atomic::AtomicBool,
}

struct PresentationBackend {
    resources: BTreeMap<String, u64>,
    failures: std::sync::Arc<BackendFailures>,
    presented: std::sync::mpsc::Sender<PresentationSet>,
    removed: std::sync::mpsc::Sender<String>,
}

impl HostBackend for PresentationBackend {
    fn capabilities(&self) -> PlaybackCapabilities {
        PlaybackCapabilities {
            data_paths: vec![crate::outputs::DataPathMode::CpuCopy],
        }
    }

    fn register_resource(
        &mut self,
        descriptor: &ResourceDescriptor,
        handle: u64,
    ) -> Result<(), SdkError> {
        self.resources
            .insert(descriptor.resource_id.clone(), handle);
        Ok(())
    }

    fn remove_resource(&mut self, id: &str, _handle: u64) -> Result<(), SdkError> {
        if self
            .failures
            .remove
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err(SdkError::new(
                SdkErrorCode::InvalidResource,
                "target is in use",
            ));
        }
        self.resources.remove(id);
        self.removed.send(id.to_owned()).unwrap();
        Ok(())
    }

    fn present(&mut self, set: &PresentationSet) -> Result<(), SdkError> {
        if self
            .failures
            .present
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err(SdkError::new(
                SdkErrorCode::InvalidResource,
                "target is unavailable",
            ));
        }
        assert!(set.items.iter().all(|item| self
            .resources
            .values()
            .any(|handle| *handle == item.resource_handle)));
        self.presented.send(set.clone()).unwrap();
        Ok(())
    }
}

#[test]
fn backend_errors_preserve_committed_evaluation_and_failed_disposal_keeps_session_usable() {
    use crate::outputs::{PresentationFit, Viewport};
    use std::sync::atomic::Ordering::Relaxed;
    let mut session =
        FrameExecutionSession::new_native(PlaybackCapabilities { data_paths: vec![] });
    let failures = std::sync::Arc::new(BackendFailures::default());
    let (presented_tx, presented) = std::sync::mpsc::channel();
    let (removed_tx, removed) = std::sync::mpsc::channel();
    session.attach_backend(Box::new(PresentationBackend {
        resources: BTreeMap::new(),
        failures: failures.clone(),
        presented: presented_tx,
        removed: removed_tx,
    }));
    session.set_graph(&graph()).unwrap();
    session
        .register_resource(
            ResourceDescriptor {
                resource_id: "target".into(),
                kind: "value".into(),
            },
            42,
        )
        .unwrap();
    let mut presentation = PresentationSubscription {
        subscription_id: "view".into(),
        output: OutputKey::new("math", "out"),
        group_id: "main".into(),
        viewport: Viewport {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
        fit: PresentationFit::Contain,
        z_index: 0,
        resource_handle: 42,
    };
    session
        .subscribe_presentation(presentation.clone())
        .unwrap();
    session
        .subscribe_output(subscription("values", DeliveryPolicy::Latest))
        .unwrap();
    failures.present.store(true, Relaxed);
    assert!(session.evaluate(&frame(2, 1)).is_err());
    assert_eq!(session.last_evaluation(), Some(&frame(2, 1).stamp));
    assert_eq!(session.drain_commands()[0].scalar_output, Some(5.0));
    let values = session.drain_deliveries().deliveries;
    assert_eq!(values[0].state.payload, OutputPayload::Float(5.0));
    assert!(presented.try_recv().is_err());
    failures.present.store(false, Relaxed);
    presentation.viewport.width = 0.5;
    presentation.fit = PresentationFit::Cover;
    session
        .update_presentation_subscription(presentation.clone())
        .unwrap();
    session
        .update_output_subscription(subscription("values", DeliveryPolicy::Every))
        .unwrap();
    session
        .evaluate_with_dirty(&frame(2, 2), &["math"])
        .unwrap();
    let delivered = session.drain_deliveries().deliveries;
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].state.payload, OutputPayload::Float(5.0));
    let expected = session.build_presentation(
        frame(2, 2).stamp,
        &BTreeMap::from([(
            OutputKey::new("math", "out"),
            delivered[0].state.content_stamp.clone(),
        )]),
    );
    assert_eq!(presented.try_recv().unwrap(), expected["main"]);
    assert_eq!(expected["main"].items[0].viewport.width, 0.5);
    assert_eq!(expected["main"].items[0].fit, PresentationFit::Cover);
    failures.remove.store(true, Relaxed);
    assert!(session.dispose().is_err());
    assert_ne!(session.state(), EngineState::Disposed);
    session
        .evaluate_with_dirty(&frame(2, 3), &["math"])
        .unwrap();
    assert_eq!(
        session.drain_deliveries().deliveries[0]
            .state
            .evaluation_stamp,
        frame(2, 3).stamp
    );
    assert_eq!(presented.try_recv().unwrap().frame_stamp, frame(2, 3).stamp);
    assert!(removed.try_recv().is_err());
    assert!(session.unsubscribe_presentation("view"));
    session
        .evaluate_with_dirty(&frame(2, 4), &["math"])
        .unwrap();
    assert!(presented.try_recv().is_err());
    failures.remove.store(false, Relaxed);
    session.dispose().unwrap();
    assert_eq!(removed.try_recv().unwrap(), "target");
    assert_eq!(session.state(), EngineState::Disposed);
    assert!(session.evaluate(&frame(2, 5)).is_err());
}
