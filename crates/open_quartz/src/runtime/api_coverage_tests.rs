use super::*;
use crate::runtime::presentation::Viewport;
use crate::runtime::{
    DeliveryPolicy, OutputSubscription, OutputTransport, PresentationFit, PresentationSubscription,
};
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

fn graph() -> Graph {
    serde_json::from_value(json!({"nodes":[{"id":"math","type":"math","position":{"x":0.0,"y":0.0},"data":{"type":"math","label":"Math","shaderCode":"","mathOp":"add","inputs":[],"outputs":[{"id":"out","label":"out","dataType":"float","direction":"output"}],"uniforms":{}}}],"edges":[]})).unwrap()
}

#[test]
fn runtime_invalid_lifecycle_resources_outputs_and_video_paths_return_errors() {
    let mut runtime = Runtime::new(RuntimeCapabilities { data_paths: vec![] });
    assert!(runtime.stop().is_err());
    assert!(runtime.play(0).is_err());
    assert!(runtime.pause(0).is_err());
    assert!(runtime.resume(0).is_err());
    assert!(runtime
        .advance(&RuntimeFrameInput {
            now_ns: 0,
            date: [0.0; 4],
            mouse: [0.0; 4],
            resolution: [1.0; 3]
        })
        .is_err());
    assert!(runtime
        .register_resource(
            ResourceDescriptor {
                resource_id: "".to_owned(),
                kind: "x".to_owned()
            },
            1
        )
        .is_err());
    assert!(runtime
        .register_resource(
            ResourceDescriptor {
                resource_id: "r".to_owned(),
                kind: "".to_owned()
            },
            1
        )
        .is_err());
    assert!(runtime
        .register_resource(
            ResourceDescriptor {
                resource_id: "r".to_owned(),
                kind: "x".to_owned()
            },
            0
        )
        .is_err());
    assert!(runtime.remove_resource("missing").is_err());

    runtime.set_graph(&graph()).unwrap();
    runtime
        .register_resource(
            ResourceDescriptor {
                resource_id: "r".to_owned(),
                kind: "x".to_owned(),
            },
            7,
        )
        .unwrap();
    assert!(runtime
        .advance(&RuntimeFrameInput {
            now_ns: 1,
            date: [0.0; 4],
            mouse: [0.0; 4],
            resolution: [1.0; 3]
        })
        .is_err());
    assert!(runtime
        .register_resource(
            ResourceDescriptor {
                resource_id: "r".to_owned(),
                kind: "x".to_owned()
            },
            8
        )
        .is_err());
    assert!(runtime.node_generation("missing").is_err());
    assert!(runtime.mark_dirty("missing").is_err());
    assert!(runtime.set_video_nodes(&["missing".to_owned()]).is_err());
    runtime.set_video_nodes(&["math".to_owned()]).unwrap();

    let unknown = OutputSubscription {
        subscription_id: "s".to_owned(),
        output: OutputKey::new("missing", "out"),
        delivery: DeliveryPolicy::Latest,
        transport: OutputTransport::Value,
        max_width: None,
        max_height: None,
    };
    assert!(runtime.subscribe_output(unknown).is_err());
    assert!(runtime
        .update_output_subscription(OutputSubscription {
            subscription_id: "missing".to_owned(),
            output: OutputKey::new("math", "out"),
            delivery: DeliveryPolicy::Latest,
            transport: OutputTransport::Value,
            max_width: None,
            max_height: None
        })
        .is_err());
    assert!(runtime.unsubscribe_output("missing").is_err());
    assert!(runtime.unsubscribe_presentation("missing") == false);
    assert!(runtime.stop().is_ok());
    assert!(runtime.play(1).is_err());
    runtime.dispose().unwrap();
    assert!(runtime.set_graph(&graph()).is_err());
}

#[test]
fn runtime_completion_rejects_generation_foreign_output_and_wrong_payload() {
    let mut runtime = Runtime::new(RuntimeCapabilities { data_paths: vec![] });
    runtime.set_graph(&graph()).unwrap();
    let base = AsyncCompletionEnvelope {
        node_id: "math".to_owned(),
        graph_revision: runtime.revision(),
        node_generation: runtime.node_generation("math").unwrap(),
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
        outputs: vec![(OutputKey::new("math", "out"), OutputPayload::Float(1.0))],
    };
    let mut stale_generation = base.clone();
    stale_generation.node_generation = 99;
    assert!(runtime.submit_completion(stale_generation).is_err());
    let mut foreign = base.clone();
    foreign.outputs[0].0.node_id = "other".to_owned();
    assert!(runtime.submit_completion(foreign).is_err());
    let mut wrong = base.clone();
    wrong.outputs[0].1 = OutputPayload::Bool(true);
    assert!(runtime.submit_completion(wrong).is_err());
    runtime.submit_completion(base.clone()).unwrap();
    runtime.submit_completion(base.clone()).unwrap();
    let mut unknown = base.clone();
    unknown.node_id = "missing".to_owned();
    unknown.outputs.clear();
    assert!(runtime.submit_completion(unknown).is_err());
    runtime
        .outputs
        .reconcile(99, runtime.output_contracts.clone());
    assert!(runtime.submit_completion(base).is_err());
    assert_eq!(runtime.drain_deliveries().deliveries.len(), 0);
    assert!(runtime.drain_work().is_ok());
    assert!(runtime.drain_events().len() >= 1);
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

struct PresentBackend(Arc<AtomicU64>);

impl HostBackend for PresentBackend {
    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities { data_paths: vec![] }
    }

    fn register_resource(&mut self, _: &ResourceDescriptor, _: u64) -> Result<(), SdkError> {
        Ok(())
    }

    fn remove_resource(&mut self, _: &str, _: u64) -> Result<(), SdkError> {
        Ok(())
    }

    fn present(&mut self, _: &PresentationSet) -> Result<(), SdkError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
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

#[test]
fn runtime_presentation_validates_updates_builds_and_dispatches_to_backend() {
    let presented = Arc::new(AtomicU64::new(0));
    let mut runtime = Runtime::new(RuntimeCapabilities { data_paths: vec![] });
    runtime.attach_backend(Box::new(PresentBackend(presented.clone())));
    runtime.set_graph(&graph()).unwrap();
    runtime
        .register_resource(
            ResourceDescriptor {
                resource_id: "target".to_owned(),
                kind: "texture".to_owned(),
            },
            7,
        )
        .unwrap();

    assert!(runtime
        .subscribe_presentation(presentation(
            "unknown-output",
            OutputKey::new("missing", "out"),
            7
        ))
        .is_err());
    assert!(runtime
        .subscribe_presentation(presentation(
            "unknown-handle",
            OutputKey::new("math", "out"),
            8
        ))
        .is_err());
    let valid = presentation("valid", OutputKey::new("math", "out"), 7);
    runtime.subscribe_presentation(valid.clone()).unwrap();
    assert!(runtime
        .update_presentation_subscription(presentation(
            "unknown-update",
            OutputKey::new("missing", "out"),
            7
        ))
        .is_err());
    assert!(runtime.subscribe_presentation(valid.clone()).is_err());
    assert!(runtime
        .update_presentation_subscription(presentation("missing", OutputKey::new("math", "out"), 7))
        .is_err());
    let mut updated = valid;
    updated.z_index = 2;
    runtime.update_presentation_subscription(updated).unwrap();

    let stamp = FrameStamp {
        epoch: 1,
        frame: 1,
        timeline_ns: 1,
        deadline_ns: 2,
    };
    let content = BTreeMap::from([(
        OutputKey::new("math", "out"),
        ContentStamp {
            epoch: 1,
            timeline_ns: 1,
            media_pts_ns: None,
        },
    )]);
    assert_eq!(runtime.build_presentation(stamp, &content).len(), 1);
    runtime.play(0).unwrap();
    runtime
        .advance(&RuntimeFrameInput {
            now_ns: 1,
            date: [0.0; 4],
            mouse: [0.0; 4],
            resolution: [1.0; 3],
        })
        .unwrap();
    assert_eq!(presented.load(Ordering::Relaxed), 1);
}

struct FailingBackend {
    fail_register: bool,
    fail_present: bool,
}

impl HostBackend for FailingBackend {
    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities { data_paths: vec![] }
    }
    fn register_resource(&mut self, _: &ResourceDescriptor, _: u64) -> Result<(), SdkError> {
        if self.fail_register {
            Err(SdkError::new(SdkErrorCode::InvalidResource, "register"))
        } else {
            Ok(())
        }
    }
    fn remove_resource(&mut self, _: &str, _: u64) -> Result<(), SdkError> {
        Ok(())
    }
    fn present(&mut self, _: &PresentationSet) -> Result<(), SdkError> {
        if self.fail_present {
            Err(SdkError::new(SdkErrorCode::InvalidResource, "present"))
        } else {
            Ok(())
        }
    }
}

#[test]
fn runtime_propagates_backend_registration_presentation_and_output_publication_errors() {
    let mut registration = Runtime::new(RuntimeCapabilities { data_paths: vec![] });
    registration.attach_backend(Box::new(FailingBackend {
        fail_register: true,
        fail_present: false,
    }));
    assert!(registration
        .register_resource(
            ResourceDescriptor {
                resource_id: "r".to_owned(),
                kind: "texture".to_owned()
            },
            1
        )
        .is_err());

    let mut runtime = Runtime::new(RuntimeCapabilities { data_paths: vec![] });
    runtime.attach_backend(Box::new(FailingBackend {
        fail_register: false,
        fail_present: true,
    }));
    runtime.set_graph(&graph()).unwrap();
    runtime
        .register_resource(
            ResourceDescriptor {
                resource_id: "r".to_owned(),
                kind: "texture".to_owned(),
            },
            1,
        )
        .unwrap();
    runtime
        .subscribe_presentation(presentation("p", OutputKey::new("math", "out"), 1))
        .unwrap();
    runtime.play(0).unwrap();
    assert!(runtime
        .advance(&RuntimeFrameInput {
            now_ns: 1,
            date: [0.0; 4],
            mouse: [0.0; 4],
            resolution: [1.0; 3]
        })
        .is_err());

    let mut stale_outputs = Runtime::new(RuntimeCapabilities { data_paths: vec![] });
    stale_outputs.set_graph(&graph()).unwrap();
    stale_outputs
        .outputs
        .reconcile(99, stale_outputs.output_contracts.clone());
    stale_outputs.play(0).unwrap();
    assert!(stale_outputs
        .advance(&RuntimeFrameInput {
            now_ns: 1,
            date: [0.0; 4],
            mouse: [0.0; 4],
            resolution: [1.0; 3]
        })
        .is_err());
}
