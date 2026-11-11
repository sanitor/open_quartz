use super::*;
use crate::outputs::{AsyncCompletionEnvelope, ContentStamp, HostBackend, OutputKey, OutputPayload, OutputSubscription, PresentationSet, ResourceDescriptor};
use crate::project::Graph;
use serde_json::json;
use crate::outputs::{DeliveryPolicy, OutputTransport};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn graph() -> Graph {
    serde_json::from_value(json!({"nodes":[{"id":"math","type":"math","position":{"x":0.0,"y":0.0},"data":{"type":"math","label":"Math","shaderCode":"","mathOp":"add","inputs":[],"outputs":[{"id":"out","label":"out","dataType":"float","direction":"output"}],"uniforms":{}}}],"edges":[]})).unwrap()
}

fn frame(now_ns: u64) -> SessionFrameInput {
    SessionFrameInput {
        now_ns,
        date: [0.0; 4],
        mouse: [0.0; 4],
        resolution: [1.0; 3],
    }
}

#[test]
fn session_rejects_invalid_lifecycle_and_unknown_video_nodes() {
    let mut session = PlaybackSession::new(PlaybackCapabilities { data_paths: vec![] });
    assert!(session.stop().is_err());
    assert!(session.play(0).is_err());
    assert!(session.pause(0).is_err());
    assert!(session.resume(0).is_err());
    assert!(session.advance(&frame(0)).is_err());

    session.set_graph(&graph()).unwrap();
    assert_eq!(session.state(), EngineState::Ready);
    assert!(session.advance(&frame(1)).is_err());
    assert!(session.node_generation("missing").is_err());
    assert!(session.mark_dirty("missing").is_err());
    assert!(session.set_video_nodes(&["missing".to_owned()]).is_err());
    session.set_video_nodes(&["math".to_owned()]).unwrap();
    assert!(session.stop().is_ok());
    assert_eq!(session.state(), EngineState::Stopped);
    assert!(session.play(1).is_err());
    session.dispose().unwrap();
    assert_eq!(session.state(), EngineState::Disposed);
    assert!(session.set_graph(&graph()).is_err());
}

#[test]
fn session_coordinates_clock_graph_pause_resume_and_disposal() {
    let mut session = PlaybackSession::new(PlaybackCapabilities { data_paths: vec![] });
    assert_eq!(session.set_graph(&graph()).unwrap(), 1);
    assert!(session.execution_plan().is_some());
    session.play(100).unwrap();
    let first = session.advance(&frame(110)).unwrap();
    assert_eq!(session.state(), EngineState::Running);
    assert_eq!(first.frame, 1);
    assert_eq!(first.timeline_ns, 10);
    assert_eq!(first.previous_timeline_ns, 0);
    assert!(!session.drain_commands().is_empty());

    session.pause(120).unwrap();
    assert_eq!(session.state(), EngineState::Paused);
    assert!(session.pause(121).is_err());
    assert!(session.advance(&frame(130)).is_err());
    session.resume(200).unwrap();
    assert_eq!(session.state(), EngineState::Running);
    assert!(session.resume(201).is_err());
    let resumed = session.advance(&frame(210)).unwrap();
    assert_eq!(resumed.epoch, first.epoch);
    assert_eq!(resumed.frame, 2);
    assert_eq!(resumed.timeline_ns, 30);
    assert_eq!(resumed.previous_timeline_ns, 10);

    session.stop().unwrap();
    assert_eq!(session.state(), EngineState::Stopped);
    assert!(session.advance(&frame(220)).is_err());
    session.set_graph(&graph()).unwrap();
    session.play(300).unwrap();
    let restarted = session.advance(&frame(305)).unwrap();
    assert!(restarted.epoch > first.epoch);
    assert_eq!(restarted.frame, 1);
    assert_eq!(restarted.timeline_ns, 5);
    assert_eq!(restarted.previous_timeline_ns, 0);
    session.dispose().unwrap();
    assert_eq!(session.state(), EngineState::Disposed);
    assert!(session.advance(&frame(310)).is_err());
    assert!(session.play(310).is_err());
    assert!(session.pause(310).is_err());
    assert!(session.resume(310).is_err());
}

#[test]
fn session_completion_validates_graph_revision_node_generation_and_node_identity() {
    let mut session = PlaybackSession::new(PlaybackCapabilities { data_paths: vec![] });
    session.set_graph(&graph()).unwrap();
    let base = AsyncCompletionEnvelope {
        node_id: "math".to_owned(),
        graph_revision: session.revision(),
        node_generation: session.node_generation("math").unwrap(),
        input_stamp: FrameStamp {
            epoch: 1,
            frame: 1,
            timeline_ns: 1,
            deadline_ns: 2,
        },
        content_stamp: ContentStamp {
            epoch: 1,
            timeline_ns: 1,
            media_pts_ns: None,
        },
        outputs: vec![],
    };
    let mut stale_revision = base.clone();
    stale_revision.graph_revision = 99;
    let validated = session.validate_completion(&stale_revision).unwrap_err();
    let submitted = session.submit_completion(stale_revision).unwrap_err();
    assert_eq!(validated.to_json(), submitted.to_json());
    let mut stale_generation = base.clone();
    stale_generation.node_generation = 99;
    let validated = session.validate_completion(&stale_generation).unwrap_err();
    let submitted = session.submit_completion(stale_generation).unwrap_err();
    assert_eq!(validated.to_json(), submitted.to_json());
    let mut unknown = base.clone();
    unknown.node_id = "missing".to_owned();
    let validated = session.validate_completion(&unknown).unwrap_err();
    let submitted = session.submit_completion(unknown).unwrap_err();
    assert_eq!(validated.to_json(), submitted.to_json());

    session.submit_completion(base.clone()).unwrap();
    session.submit_completion(base.clone()).unwrap();
    session.set_graph(&graph()).unwrap();
    assert!(session.submit_completion(base).is_err());
    assert!(session.drain_work().is_ok());
    assert!(!session.drain_events().is_empty());
}

#[test]
fn session_marks_dependents_only_after_the_entire_completion_is_accepted() {
    let mut graph = graph();
    let mut dependent = graph.nodes[0].clone();
    dependent.id = "dependent".to_owned();
    dependent.data.inputs = serde_json::from_value(json!([
        {"id":"a","label":"a","dataType":"float","direction":"input"}
    ]))
    .unwrap();
    graph.nodes.push(dependent);
    graph.edges = serde_json::from_value(json!([
        {"id":"math-dependent","source":"math","sourceHandle":"out",
         "target":"dependent","targetHandle":"a"}
    ]))
    .unwrap();
    let mut session = PlaybackSession::new(PlaybackCapabilities { data_paths: vec![] });
    session.set_graph(&graph).unwrap();
    session.play(0).unwrap();
    session.advance(&frame(1)).unwrap();
    let initial = session.drain_commands();
    assert_eq!(
        initial.iter().map(|command| command.node_id.as_str()).collect::<Vec<_>>(),
        vec!["math", "dependent"]
    );
    session.advance(&frame(2)).unwrap();
    assert!(session.drain_commands().is_empty());

    let output = OutputKey::new("math", "out");
    session.set_every_queue_capacity(1);
    session.subscribe_output(OutputSubscription {
        subscription_id: "source".to_owned(),
        output: output.clone(),
        delivery: DeliveryPolicy::Every,
        transport: OutputTransport::Value,
        max_width: None,
        max_height: None,
    })
    .unwrap();
    let completion = AsyncCompletionEnvelope {
        node_id: "math".to_owned(),
        graph_revision: session.revision(),
        node_generation: session.node_generation("math").unwrap(),
        input_stamp: FrameStamp {
            epoch: 1,
            frame: 2,
            timeline_ns: 2,
            deadline_ns: 3,
        },
        content_stamp: ContentStamp {
            epoch: 1,
            timeline_ns: 2,
            media_pts_ns: None,
        },
        outputs: vec![(output.clone(), OutputPayload::Float(1.0))],
    };
    let clock_before = session.clock.state();
    session.drain_events();
    session.validate_completion(&completion).unwrap();
    session.validate_completion(&completion).unwrap();
    assert_eq!(session.clock.state(), clock_before);
    assert!(session.drain_events().is_empty());
    assert!(session.drain_deliveries().deliveries.is_empty());
    session.advance(&frame(3)).unwrap();
    assert!(session.drain_commands().is_empty());
    session.submit_completion(completion.clone()).unwrap();
    session.advance(&frame(4)).unwrap();
    let accepted = session.drain_commands();
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0].node_id, "dependent");

    let mut invalid = completion.clone();
    invalid.outputs.push((output, OutputPayload::Bool(true)));
    let validated = session.validate_completion(&invalid).unwrap_err();
    let submitted = session.submit_completion(invalid).unwrap_err();
    assert_eq!(validated.code, SdkErrorCode::InvalidResource);
    assert_eq!(validated.to_json(), submitted.to_json());
    session.advance(&frame(5)).unwrap();
    assert!(session.drain_commands().is_empty());
    let validated = session.validate_completion(&completion).unwrap_err();
    let submitted = session.submit_completion(completion.clone()).unwrap_err();
    assert_eq!(validated.code, SdkErrorCode::InvalidState);
    assert_eq!(validated.to_json(), submitted.to_json());
    session.advance(&frame(6)).unwrap();
    assert!(session.drain_commands().is_empty());

    let pending = session.drain_deliveries();
    assert_eq!(pending.deliveries.len(), 1);
    assert_eq!(pending.deliveries[0].state.output_generation, 2);
    session.validate_completion(&completion).unwrap();
    session.validate_completion(&completion).unwrap();
    session.submit_completion(completion).unwrap();
    let published = session.drain_deliveries();
    assert_eq!(published.deliveries.len(), 1);
    assert_eq!(published.deliveries[0].state.output_generation, 3);
    session.advance(&frame(7)).unwrap();
    let retried = session.drain_commands();
    assert_eq!(retried.len(), 1);
    assert_eq!(retried[0].node_id, "dependent");
}

struct RemovalBackend(Arc<AtomicBool>);

impl HostBackend for RemovalBackend {
    fn capabilities(&self) -> PlaybackCapabilities {
        PlaybackCapabilities { data_paths: vec![] }
    }

    fn register_resource(&mut self, _: &ResourceDescriptor, _: u64) -> Result<(), SdkError> {
        Ok(())
    }

    fn remove_resource(&mut self, _: &str, _: u64) -> Result<(), SdkError> {
        if self.0.load(Ordering::Relaxed) {
            Err(SdkError::new(SdkErrorCode::InvalidResource, "remove"))
        } else {
            Ok(())
        }
    }

    fn present(&mut self, _: &PresentationSet) -> Result<(), SdkError> {
        Ok(())
    }
}

#[test]
fn session_disposes_graph_and_clock_only_after_hub_disposal_succeeds() {
    let fail_remove = Arc::new(AtomicBool::new(true));
    let mut session = PlaybackSession::new(PlaybackCapabilities { data_paths: vec![] });
    session.attach_backend(Box::new(RemovalBackend(fail_remove.clone())));
    session.set_graph(&graph()).unwrap();
    session.register_resource(ResourceDescriptor {
        resource_id: "target".to_owned(),
        kind: "texture".to_owned(),
    }, 7)
    .unwrap();
    session.play(100).unwrap();
    let first = session.advance(&frame(110)).unwrap();
    let clock_before = session.clock.state();
    assert_eq!(session.dispose().unwrap_err().code, SdkErrorCode::InvalidResource);
    assert_eq!(session.state(), EngineState::Running);
    assert_eq!(session.clock.state(), clock_before);
    let continued = session.advance(&frame(120)).unwrap();
    assert_eq!(continued.epoch, first.epoch);
    assert_eq!(continued.frame, 2);
    assert_eq!(continued.timeline_ns, 20);

    fail_remove.store(false, Ordering::Relaxed);
    session.dispose().unwrap();
    assert_eq!(session.state(), EngineState::Disposed);
    assert_eq!(session.clock.state().frame, 0);
    assert_eq!(session.clock.state().timeline_ns, 0);
    assert!(session.clock.state().epoch > first.epoch);
    assert!(session.advance(&frame(130)).is_err());
}

#[test]
fn decode_engine_error_accepts_structured_and_wraps_plain_errors() {
    let structured = SdkError::new(SdkErrorCode::InvalidGraph, "bad");
    let decoded = decode_engine_error(serde_json::to_string(&structured).unwrap());
    assert_eq!(decoded.code, SdkErrorCode::InvalidGraph);
    let fallback = decode_engine_error("plain".to_owned());
    assert_eq!(fallback.code, SdkErrorCode::InvalidState);
    assert_eq!(fallback.details.as_deref(), Some("plain"));
}

#[test]
fn rejected_scalar_frame_preserves_pending_work_clock_and_deliveries_until_same_input_retry() {
    fn prepared_session() -> PlaybackSession {
        let mut graph = graph();
        let mut first = graph.nodes[0].clone();
        first.id = "first".to_owned();
        first.data.uniforms = serde_json::from_value(json!({"a": 2.0, "b": 3.0})).unwrap();
        let mut late = graph.nodes[0].clone();
        late.id = "late".to_owned();
        late.data.uniforms = serde_json::from_value(json!({"a": 7.0, "b": 11.0})).unwrap();
        graph.nodes = vec![first, late];
        let mut session = PlaybackSession::new(PlaybackCapabilities { data_paths: vec![] });
        session.set_graph(&graph).unwrap();
        session.set_every_queue_capacity(1);
        for (id, node, delivery) in [
            ("first-latest", "first", DeliveryPolicy::Latest),
            ("first-change", "first", DeliveryPolicy::OnChange),
            ("late-every", "late", DeliveryPolicy::Every),
        ] {
            session.subscribe_output(OutputSubscription {
                subscription_id: id.to_owned(),
                output: OutputKey::new(node, "out"),
                delivery,
                transport: OutputTransport::Value,
                max_width: None,
                max_height: None,
            })
            .unwrap();
        }
        session.play(100).unwrap();
        session.advance(&frame(110)).unwrap();
        assert_eq!(
            session.graph.pending_commands().iter()
                .map(|command| (command.node_id.as_str(), command.scalar_output))
                .collect::<Vec<_>>(),
            vec![("first", Some(5.0)), ("late", Some(18.0))]
        );
        session.mark_dirty("first").unwrap();
        session.advance(&frame(120)).unwrap();
        assert_eq!(session.graph.pending_commands().len(), 1);
        assert_eq!(session.graph.pending_commands()[0].node_id, "first");
        session.mark_dirty("first").unwrap();
        session.mark_dirty("late").unwrap();
        session.drain_events();
        session
    }

    let mut session = prepared_session();
    let mut control = prepared_session();
    let clock_before = session.clock.state();
    let commands_before = serde_json::to_value(session.graph.pending_commands()).unwrap();
    let generations_before = [
        session.node_generation("first").unwrap(),
        session.node_generation("late").unwrap(),
    ];
    let error = session.advance(&frame(130)).unwrap_err();
    assert_eq!(error.code, SdkErrorCode::InvalidState);
    assert_eq!(error.details.as_deref(), Some("late-every"));
    assert_eq!(session.state(), EngineState::Running);
    assert_eq!(session.clock.state(), clock_before);
    assert_eq!(session.revision(), control.revision());
    assert_eq!(
        [
            session.node_generation("first").unwrap(),
            session.node_generation("late").unwrap(),
        ],
        generations_before
    );
    assert_eq!(
        serde_json::to_value(session.graph.pending_commands()).unwrap(),
        commands_before
    );
    assert!(session.drain_events().is_empty());
    let queued = session.drain_deliveries();
    assert_eq!(queued, control.drain_deliveries());
    assert_eq!(queued.deliveries.len(), 3);
    for delivery in &queued.deliveries {
        if delivery.state.output.node_id == "first" {
            assert_eq!(delivery.state.output_generation, 2);
            assert_eq!(delivery.state.payload, OutputPayload::Float(5.0));
            assert_eq!(delivery.state.evaluation_stamp.frame, 2);
        } else {
            assert_eq!(delivery.state.output.node_id, "late");
            assert_eq!(delivery.state.output_generation, 1);
            assert_eq!(delivery.state.payload, OutputPayload::Float(18.0));
            assert_eq!(delivery.state.evaluation_stamp.frame, 1);
        }
    }

    let retried = session.advance(&frame(130)).unwrap();
    assert_eq!(retried, control.advance(&frame(130)).unwrap());
    assert_eq!(retried.frame, 3);
    assert_eq!(retried.timeline_ns, 30);
    assert_eq!(retried.previous_timeline_ns, 20);
    assert_eq!(
        serde_json::to_value(session.graph.pending_commands()).unwrap(),
        serde_json::to_value(control.graph.pending_commands()).unwrap()
    );
    assert_eq!(
        session.graph.pending_commands().iter()
            .map(|command| (command.node_id.as_str(), command.scalar_output))
            .collect::<Vec<_>>(),
        vec![("first", Some(5.0)), ("late", Some(18.0))]
    );
    let delivered = session.drain_deliveries();
    assert_eq!(delivered, control.drain_deliveries());
    assert_eq!(delivered.deliveries.len(), 3);
    for delivery in &delivered.deliveries {
        assert_eq!(delivery.state.evaluation_stamp.frame, 3);
        assert_eq!(delivery.state.evaluation_stamp.timeline_ns, 30);
        if delivery.state.output.node_id == "first" {
            assert_eq!(delivery.state.output_generation, 3);
            assert_eq!(delivery.state.payload, OutputPayload::Float(5.0));
        } else {
            assert_eq!(delivery.state.output.node_id, "late");
            assert_eq!(delivery.state.output_generation, 2);
            assert_eq!(delivery.state.payload, OutputPayload::Float(18.0));
        }
    }
    session.advance(&frame(140)).unwrap();
    assert!(session.drain_commands().is_empty());
}
