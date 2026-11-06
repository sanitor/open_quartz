use super::*;
use crate::outputs::presentation::Viewport;
use crate::outputs::{DataPathMode, DeliveryPolicy, OutputTransport, PresentationFit};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

fn contracts() -> BTreeMap<OutputKey, DataType> {
    BTreeMap::from([
        (OutputKey::new("math", "out"), DataType::Float),
        (OutputKey::new("math", "next"), DataType::Float),
        (OutputKey::new("other", "out"), DataType::Float),
    ])
}

fn hub() -> OutputHub {
    let mut hub = OutputHub::new(PlaybackCapabilities { data_paths: vec![] });
    hub.reconcile(1, contracts());
    hub
}

fn stamp(frame: u64) -> FrameStamp {
    FrameStamp {
        epoch: 1,
        frame,
        timeline_ns: frame * 10,
        deadline_ns: frame * 10 + 1,
    }
}

fn content(frame: u64) -> ContentStamp {
    ContentStamp {
        epoch: 1,
        timeline_ns: frame * 10,
        media_pts_ns: None,
    }
}

fn completion(frame: u64, outputs: Vec<(OutputKey, OutputPayload)>) -> AsyncCompletionEnvelope {
    AsyncCompletionEnvelope {
        node_id: "math".to_owned(),
        graph_revision: 1,
        node_generation: 1,
        input_stamp: stamp(frame),
        content_stamp: content(frame),
        outputs,
    }
}

fn subscription(id: &str, output: OutputKey, delivery: DeliveryPolicy) -> OutputSubscription {
    OutputSubscription {
        subscription_id: id.to_owned(),
        output,
        delivery,
        transport: OutputTransport::Value,
        max_width: None,
        max_height: None,
    }
}

fn subscribed_hub(capacity: usize) -> OutputHub {
    let mut hub = hub();
    hub.set_every_queue_capacity(capacity);
    for port in ["out", "next"] {
        for (name, delivery) in [
            ("every", DeliveryPolicy::Every),
            ("latest", DeliveryPolicy::Latest),
            ("change", DeliveryPolicy::OnChange),
        ] {
            hub.subscribe_output(subscription(
                &format!("{port}-{name}"),
                OutputKey::new("math", port),
                delivery,
            ))
            .unwrap();
        }
    }
    hub
}

fn presentation(id: &str, output: OutputKey, handle: u64) -> PresentationSubscription {
    PresentationSubscription {
        subscription_id: id.to_owned(),
        output,
        group_id: "group".to_owned(),
        viewport: Viewport {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
        fit: PresentationFit::Contain,
        z_index: 0,
        resource_handle: handle,
    }
}

fn resource(id: &str) -> ResourceDescriptor {
    ResourceDescriptor {
        resource_id: id.to_owned(),
        kind: "texture".to_owned(),
    }
}

#[derive(Clone, Debug, PartialEq)]
enum BackendCall {
    Register(ResourceDescriptor, u64),
    Remove(String, u64),
    Present(PresentationSet),
}

#[derive(Default)]
struct BackendFailures {
    register: AtomicBool,
    remove_handle: AtomicU64,
    present: AtomicBool,
}

struct BackendControl {
    failures: Arc<BackendFailures>,
    calls: Receiver<BackendCall>,
}

struct TestBackend {
    failures: Arc<BackendFailures>,
    calls: Sender<BackendCall>,
}

impl HostBackend for TestBackend {
    fn capabilities(&self) -> PlaybackCapabilities {
        PlaybackCapabilities {
            data_paths: vec![DataPathMode::CpuCopy],
        }
    }

    fn register_resource(
        &mut self,
        descriptor: &ResourceDescriptor,
        handle: u64,
    ) -> Result<(), SdkError> {
        self.calls.send(BackendCall::Register(descriptor.clone(), handle)).unwrap();
        if self.failures.register.load(Ordering::Relaxed) {
            Err(SdkError::new(SdkErrorCode::InvalidResource, "register"))
        } else {
            Ok(())
        }
    }

    fn remove_resource(&mut self, resource_id: &str, handle: u64) -> Result<(), SdkError> {
        self.calls.send(BackendCall::Remove(resource_id.to_owned(), handle)).unwrap();
        if self.failures.remove_handle.load(Ordering::Relaxed) == handle {
            Err(SdkError::new(SdkErrorCode::InvalidResource, "remove"))
        } else {
            Ok(())
        }
    }

    fn present(&mut self, set: &PresentationSet) -> Result<(), SdkError> {
        self.calls.send(BackendCall::Present(set.clone())).unwrap();
        if self.failures.present.load(Ordering::Relaxed) {
            Err(SdkError::new(SdkErrorCode::InvalidResource, "present"))
        } else {
            Ok(())
        }
    }
}

fn attach_backend(hub: &mut OutputHub) -> BackendControl {
    let failures = Arc::new(BackendFailures::default());
    let (sender, calls) = channel();
    hub.attach_backend(Box::new(TestBackend {
        failures: failures.clone(),
        calls: sender,
    }));
    BackendControl { failures, calls }
}

#[test]
fn hub_validates_resources_and_output_subscription_references() {
    let mut hub = hub();
    assert!(hub.register_resource(resource(""), 1).is_err());
    assert!(hub
        .register_resource(
            ResourceDescriptor {
                resource_id: "r".to_owned(),
                kind: "".to_owned(),
            },
            1,
        )
        .is_err());
    assert!(hub.register_resource(resource("r"), 0).is_err());
    assert!(hub.remove_resource("missing").is_err());
    assert!(hub.resources.is_empty());
    hub.register_resource(resource("r"), 7).unwrap();
    assert!(hub.register_resource(resource("r"), 8).is_err());
    assert_eq!(hub.resources["r"].handle, 7);

    assert!(hub
        .subscribe_output(subscription(
            "s",
            OutputKey::new("missing", "out"),
            DeliveryPolicy::Latest,
        ))
        .is_err());
    assert!(hub
        .update_output_subscription(subscription(
            "missing",
            OutputKey::new("math", "out"),
            DeliveryPolicy::Latest,
        ))
        .is_err());
    assert!(hub.unsubscribe_output("missing").is_err());
    assert!(!hub.unsubscribe_presentation("missing"));
    assert_eq!(hub.remove_resource("r").unwrap(), 7);
    assert!(hub.resources.is_empty());
}

#[test]
fn hub_presentation_validates_updates_builds_and_dispatches_to_backend() {
    let mut hub = hub();
    let backend = attach_backend(&mut hub);
    assert_eq!(hub.capabilities().data_paths, vec![DataPathMode::CpuCopy]);
    hub.register_resource(resource("target"), 7).unwrap();
    assert!(hub
        .subscribe_presentation(presentation(
            "unknown-output",
            OutputKey::new("missing", "out"),
            7,
        ))
        .is_err());
    assert!(hub
        .subscribe_presentation(presentation(
            "unknown-handle",
            OutputKey::new("math", "out"),
            8,
        ))
        .is_err());
    let valid = presentation("valid", OutputKey::new("math", "out"), 7);
    hub.subscribe_presentation(valid.clone()).unwrap();
    assert!(hub
        .update_presentation_subscription(presentation(
            "unknown-update",
            OutputKey::new("missing", "out"),
            7,
        ))
        .is_err());
    assert!(hub.subscribe_presentation(valid.clone()).is_err());
    assert!(hub
        .update_presentation_subscription(presentation(
            "missing",
            OutputKey::new("math", "out"),
            7,
        ))
        .is_err());
    let mut updated = valid;
    updated.z_index = 2;
    hub.update_presentation_subscription(updated).unwrap();

    let stamps = BTreeMap::from([(OutputKey::new("math", "out"), content(1))]);
    let sets = hub.build_presentation(stamp(1), &stamps);
    assert_eq!(sets.len(), 1);
    assert_eq!(sets["group"].items.len(), 1);
    assert_eq!(sets["group"].items[0].z_index, 2);
    assert_eq!(sets["group"].items[0].resource_handle, 7);
    hub.publish_values(
        1,
        stamp(1),
        content(1),
        vec![(OutputKey::new("math", "out"), OutputPayload::Float(1.0))],
    )
    .unwrap();
    hub.dispatch_presentations(stamp(1)).unwrap();
    assert_eq!(
        backend.calls.try_iter().collect::<Vec<_>>(),
        vec![
            BackendCall::Register(resource("target"), 7),
            BackendCall::Present(sets["group"].clone()),
        ]
    );
}

#[test]
fn hub_propagates_backend_registration_and_presentation_errors() {
    let mut hub = hub();
    let backend = attach_backend(&mut hub);
    backend.failures.register.store(true, Ordering::Relaxed);
    assert_eq!(
        hub.register_resource(resource("r"), 1).unwrap_err().code,
        SdkErrorCode::InvalidResource
    );
    assert!(hub.resources.is_empty());
    backend.failures.register.store(false, Ordering::Relaxed);
    hub.register_resource(resource("r"), 1).unwrap();
    hub.subscribe_presentation(presentation("p", OutputKey::new("math", "out"), 1))
        .unwrap();
    backend.failures.present.store(true, Ordering::Relaxed);
    assert_eq!(
        hub.dispatch_presentations(stamp(1)).unwrap_err().code,
        SdkErrorCode::InvalidResource
    );
    assert_eq!(hub.build_presentation(stamp(1), &BTreeMap::new()).len(), 1);
    backend.failures.present.store(false, Ordering::Relaxed);
    hub.dispatch_presentations(stamp(1)).unwrap();
}

#[test]
fn hub_rejects_foreign_and_wrong_completion_payloads_and_accepts_repeated_completions() {
    let mut hub = hub();
    let base = completion(
        1,
        vec![(OutputKey::new("math", "out"), OutputPayload::Float(1.0))],
    );
    let mut foreign = base.clone();
    foreign.outputs[0].0 = OutputKey::new("other", "out");
    assert!(hub.publish_completion(&foreign).is_err());
    let mut wrong = base.clone();
    wrong.outputs[0].1 = OutputPayload::Bool(true);
    assert!(hub.publish_completion(&wrong).is_err());
    hub.publish_completion(&base).unwrap();
    hub.publish_completion(&base).unwrap();
    assert_eq!(
        hub.outputs.state(&OutputKey::new("math", "out")).unwrap().output_generation,
        2
    );
    assert_eq!(hub.drain_deliveries().deliveries.len(), 0);
}

#[test]
fn hub_rejects_completion_and_explicit_publication_against_artificially_stale_registry() {
    let mut hub = hub();
    let base = completion(
        1,
        vec![(OutputKey::new("math", "out"), OutputPayload::Float(1.0))],
    );
    hub.outputs.reconcile(99, contracts());
    assert!(hub.publish_completion(&base).is_err());
    let mut empty = base.clone();
    empty.outputs.clear();
    assert!(hub.publish_completion(&empty).is_err());
    assert!(hub
        .publish_output(OutputState {
            output: OutputKey::new("math", "out"),
            graph_revision: 1,
            output_generation: 1,
            evaluation_stamp: stamp(1),
            content_stamp: content(1),
            payload: OutputPayload::Float(1.0),
        })
        .is_err());
    assert!(hub.outputs.state(&OutputKey::new("math", "out")).is_none());
    assert_eq!(hub.drain_deliveries().deliveries.len(), 0);
}

#[test]
fn invalid_or_foreign_payload_late_in_batch_preserves_states_deliveries_and_generations() {
    let first = OutputKey::new("math", "out");
    let second = OutputKey::new("math", "next");
    for rejected in [
        (second.clone(), OutputPayload::Bool(true)),
        (OutputKey::new("other", "out"), OutputPayload::Float(40.0)),
    ] {
        let seed = completion(
            1,
            vec![
                (first.clone(), OutputPayload::Float(10.0)),
                (second.clone(), OutputPayload::Float(20.0)),
            ],
        );
        let mut hub = subscribed_hub(4);
        let mut control = subscribed_hub(4);
        hub.publish_completion(&seed).unwrap();
        control.publish_completion(&seed).unwrap();
        let first_before = hub.outputs.state(&first).cloned();
        let second_before = hub.outputs.state(&second).cloned();
        let stamps_before = hub.outputs.content_stamps();
        let failed = completion(
            2,
            vec![(first.clone(), OutputPayload::Float(30.0)), rejected],
        );
        assert_eq!(hub.publish_completion(&failed).unwrap_err().code, SdkErrorCode::InvalidResource);
        assert_eq!(hub.outputs.state(&first), first_before.as_ref());
        assert_eq!(hub.outputs.state(&second), second_before.as_ref());
        assert!(hub.outputs.state(&OutputKey::new("other", "out")).is_none());
        assert_eq!(hub.outputs.content_stamps(), stamps_before);
        assert_eq!(hub.drain_deliveries(), control.drain_deliveries());

        let retry = completion(
            2,
            vec![
                (first.clone(), OutputPayload::Float(30.0)),
                (second.clone(), OutputPayload::Float(40.0)),
            ],
        );
        hub.publish_completion(&retry).unwrap();
        control.publish_completion(&retry).unwrap();
        assert_eq!(hub.outputs.state(&first).unwrap().output_generation, 2);
        assert_eq!(hub.outputs.state(&second).unwrap().output_generation, 2);
        let delivered = hub.drain_deliveries();
        assert_eq!(delivered, control.drain_deliveries());
        assert_eq!(delivered.deliveries.len(), 6);
        assert!(delivered.deliveries.iter().all(|delivery| delivery.state.output_generation == 2));
    }
}

#[test]
fn full_every_queue_late_in_batch_prevents_any_partial_publication() {
    let first = OutputKey::new("math", "out");
    let second = OutputKey::new("math", "next");
    let seed = completion(1, vec![(second.clone(), OutputPayload::Float(20.0))]);
    let mut hub = subscribed_hub(1);
    let mut control = subscribed_hub(1);
    hub.publish_completion(&seed).unwrap();
    control.publish_completion(&seed).unwrap();
    let second_before = hub.outputs.state(&second).cloned();
    let stamps_before = hub.outputs.content_stamps();
    let batch = completion(
        2,
        vec![
            (first.clone(), OutputPayload::Float(30.0)),
            (second.clone(), OutputPayload::Float(40.0)),
        ],
    );
    let error = hub.publish_completion(&batch).unwrap_err();
    assert_eq!(error.code, SdkErrorCode::InvalidState);
    assert_eq!(error.details.as_deref(), Some("next-every"));
    assert!(hub.outputs.state(&first).is_none());
    assert_eq!(hub.outputs.state(&second), second_before.as_ref());
    assert_eq!(hub.outputs.content_stamps(), stamps_before);
    assert_eq!(hub.drain_deliveries(), control.drain_deliveries());

    hub.publish_completion(&batch).unwrap();
    control.publish_completion(&batch).unwrap();
    assert_eq!(hub.outputs.state(&first).unwrap().output_generation, 1);
    assert_eq!(hub.outputs.state(&second).unwrap().output_generation, 2);
    let delivered = hub.drain_deliveries();
    assert_eq!(delivered, control.drain_deliveries());
    assert_eq!(delivered.deliveries.len(), 6);
}

#[test]
fn repeated_output_in_batch_reserves_each_queue_slot_and_assigns_consecutive_generations() {
    let output = OutputKey::new("math", "out");
    let mut hub = subscribed_hub(2);
    let seed = completion(1, vec![(output.clone(), OutputPayload::Float(10.0))]);
    hub.publish_completion(&seed).unwrap();
    let before = hub.outputs.state(&output).cloned();
    let batch = completion(
        2,
        vec![
            (output.clone(), OutputPayload::Float(20.0)),
            (output.clone(), OutputPayload::Float(30.0)),
        ],
    );
    let error = hub.publish_completion(&batch).unwrap_err();
    assert_eq!(error.code, SdkErrorCode::InvalidState);
    assert_eq!(error.details.as_deref(), Some("out-every"));
    assert_eq!(hub.outputs.state(&output), before.as_ref());
    let pending = hub.drain_deliveries();
    assert_eq!(pending.frame_stamp, Some(stamp(1)));
    assert_eq!(pending.deliveries.len(), 3);
    assert!(pending.deliveries.iter().all(|delivery| delivery.state == *before.as_ref().unwrap()));

    hub.publish_completion(&batch).unwrap();
    let state = hub.outputs.state(&output).unwrap().clone();
    assert_eq!(state.output_generation, 3);
    assert_eq!(state.payload, OutputPayload::Float(30.0));
    let full = completion(3, vec![(output.clone(), OutputPayload::Float(40.0))]);
    assert_eq!(hub.publish_completion(&full).unwrap_err().code, SdkErrorCode::InvalidState);
    assert_eq!(hub.outputs.state(&output), Some(&state));
    let delivered = hub.drain_deliveries();
    assert_eq!(delivered.frame_stamp, Some(stamp(2)));
    assert_eq!(delivered.deliveries.len(), 4);
    let every = delivered.deliveries.iter()
        .filter(|delivery| delivery.subscription_id == "out-every")
        .map(|delivery| (delivery.state.output_generation, delivery.state.payload.clone()))
        .collect::<Vec<_>>();
    assert_eq!(every, vec![(2, OutputPayload::Float(20.0)), (3, OutputPayload::Float(30.0))]);
    for id in ["out-latest", "out-change"] {
        assert_eq!(
            delivered.deliveries.iter().find(|delivery| delivery.subscription_id == id).unwrap().state,
            state
        );
    }
    hub.publish_completion(&full).unwrap();
    assert_eq!(hub.outputs.state(&output).unwrap().output_generation, 4);
    let retry = hub.drain_deliveries();
    assert_eq!(retry.deliveries.len(), 3);
    assert!(retry.deliveries.iter().all(|delivery| delivery.state.output_generation == 4));
}

#[test]
fn failed_resource_removal_retains_ownership_and_presentation_until_retry() {
    let mut hub = hub();
    let backend = attach_backend(&mut hub);
    hub.register_resource(resource("target"), 7).unwrap();
    hub.subscribe_presentation(presentation("p", OutputKey::new("math", "out"), 7))
        .unwrap();
    let owned = hub.resources["target"].clone();
    let before = hub.build_presentation(stamp(1), &BTreeMap::new());
    backend.failures.remove_handle.store(7, Ordering::Relaxed);
    assert_eq!(hub.remove_resource("target").unwrap_err().code, SdkErrorCode::InvalidResource);
    assert_eq!(hub.resources.get("target"), Some(&owned));
    assert_eq!(hub.build_presentation(stamp(1), &BTreeMap::new()), before);
    assert!(hub.register_resource(resource("target"), 8).is_err());
    backend.failures.remove_handle.store(0, Ordering::Relaxed);
    assert_eq!(hub.remove_resource("target").unwrap(), 7);
    assert!(!hub.resources.contains_key("target"));
    assert!(hub.build_presentation(stamp(1), &BTreeMap::new()).is_empty());
    assert_eq!(
        backend.calls.try_iter().collect::<Vec<_>>(),
        vec![
            BackendCall::Register(resource("target"), 7),
            BackendCall::Remove("target".to_owned(), 7),
            BackendCall::Remove("target".to_owned(), 7),
        ]
    );
}

#[test]
fn dispose_failure_preserves_remaining_resources_and_registry_until_ordered_retry() {
    let mut hub = hub();
    let backend = attach_backend(&mut hub);
    let output = OutputKey::new("math", "out");
    for (id, handle) in [("c", 3), ("a", 1), ("b", 2)] {
        hub.register_resource(resource(id), handle).unwrap();
        hub.subscribe_presentation(presentation(id, output.clone(), handle)).unwrap();
    }
    hub.subscribe_output(subscription("keep", output.clone(), DeliveryPolicy::Latest)).unwrap();
    hub.publish_completion(&completion(1, vec![(output.clone(), OutputPayload::Float(10.0))]))
        .unwrap();
    let before = hub.outputs.state(&output).cloned();
    backend.calls.try_iter().for_each(drop);
    backend.failures.remove_handle.store(2, Ordering::Relaxed);
    assert_eq!(hub.dispose().unwrap_err().code, SdkErrorCode::InvalidResource);
    assert_eq!(
        backend.calls.try_iter().collect::<Vec<_>>(),
        vec![BackendCall::Remove("a".to_owned(), 1), BackendCall::Remove("b".to_owned(), 2)]
    );
    assert_eq!(hub.resources.keys().map(String::as_str).collect::<Vec<_>>(), vec!["b", "c"]);
    assert_eq!(hub.resources["b"].descriptor, resource("b"));
    assert_eq!(hub.resources["b"].handle, 2);
    assert_eq!(hub.resources["c"].descriptor, resource("c"));
    assert_eq!(hub.resources["c"].handle, 3);
    assert_eq!(hub.outputs.state(&output), before.as_ref());
    assert!(hub.outputs.contains_output(&output));
    let sets = hub.build_presentation(stamp(1), &BTreeMap::new());
    assert_eq!(sets["group"].items.iter().map(|item| item.resource_handle).collect::<Vec<_>>(), vec![2, 3]);
    let pending = hub.drain_deliveries();
    assert_eq!(pending.frame_stamp, Some(stamp(1)));
    assert_eq!(pending.deliveries.len(), 1);
    assert_eq!(pending.deliveries[0].subscription_id, "keep");
    assert_eq!(pending.deliveries[0].state, *before.as_ref().unwrap());
    assert!(pending.invalidations.is_empty());

    backend.failures.remove_handle.store(0, Ordering::Relaxed);
    hub.dispose().unwrap();
    assert_eq!(
        backend.calls.try_iter().collect::<Vec<_>>(),
        vec![
            BackendCall::Remove("b".to_owned(), 2),
            BackendCall::Remove("c".to_owned(), 3),
        ]
    );
    assert!(hub.resources.is_empty());
    assert!(hub.outputs.state(&output).is_none());
    assert!(!hub.outputs.contains_output(&output));
    assert!(hub.build_presentation(stamp(1), &BTreeMap::new()).is_empty());
    let empty = hub.drain_deliveries();
    assert!(empty.deliveries.is_empty());
    assert!(empty.invalidations.is_empty());
    assert_eq!(empty.frame_stamp, None);
    assert!(hub.unsubscribe_output("keep").is_err());
}

#[test]
fn completion_preflight_preserves_capacity_generations_states_and_pending_deliveries() {
    let output = OutputKey::new("math", "out");
    let seed = completion(1, vec![(output.clone(), OutputPayload::Float(10.0))]);
    let candidate = completion(2, vec![(output.clone(), OutputPayload::Float(20.0))]);
    let mut hub = subscribed_hub(2);
    let mut control = subscribed_hub(2);
    hub.publish_completion(&seed).unwrap();
    control.publish_completion(&seed).unwrap();
    let before = hub.outputs.state(&output).cloned();
    let stamps_before = hub.outputs.content_stamps();

    hub.validate_completion(&candidate).unwrap();
    hub.validate_completion(&candidate).unwrap();
    assert_eq!(hub.outputs.state(&output), before.as_ref());
    assert_eq!(hub.outputs.content_stamps(), stamps_before);
    hub.publish_completion(&candidate).unwrap();
    control.publish_completion(&candidate).unwrap();
    assert_eq!(hub.outputs.state(&output).unwrap().output_generation, 2);
    let full_state = hub.outputs.state(&output).cloned();
    let full_stamps = hub.outputs.content_stamps();

    for rejected in [
        completion(3, vec![(output.clone(), OutputPayload::Float(30.0))]),
        completion(
            3,
            vec![
                (output.clone(), OutputPayload::Float(30.0)),
                (OutputKey::new("math", "next"), OutputPayload::Bool(true)),
            ],
        ),
        completion(
            3,
            vec![
                (output.clone(), OutputPayload::Float(30.0)),
                (OutputKey::new("other", "out"), OutputPayload::Float(40.0)),
            ],
        ),
    ] {
        let validated = hub.validate_completion(&rejected).unwrap_err();
        let submitted = hub.publish_completion(&rejected).unwrap_err();
        assert_eq!(validated.to_json(), submitted.to_json());
        assert_eq!(hub.outputs.state(&output), full_state.as_ref());
        assert_eq!(hub.outputs.content_stamps(), full_stamps);
    }
    assert_eq!(hub.drain_deliveries(), control.drain_deliveries());

    let retry = completion(3, vec![(output.clone(), OutputPayload::Float(30.0))]);
    hub.validate_completion(&retry).unwrap();
    hub.validate_completion(&retry).unwrap();
    assert_eq!(hub.outputs.state(&output), full_state.as_ref());
    hub.publish_completion(&retry).unwrap();
    control.publish_completion(&retry).unwrap();
    assert_eq!(hub.outputs.state(&output).unwrap().output_generation, 3);
    assert_eq!(hub.drain_deliveries(), control.drain_deliveries());
}
