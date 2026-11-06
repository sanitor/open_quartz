use super::*;

fn subscription(id: &str) -> OutputSubscription {
    OutputSubscription {
        subscription_id: id.to_owned(),
        output: OutputKey::new("node", "port"),
        delivery: DeliveryPolicy::Latest,
        transport: OutputTransport::Value,
        max_width: None,
        max_height: None,
    }
}

#[test]
fn payload_matching_covers_every_data_type_family() {
    let values = [
        (DataType::Bool, OutputPayload::Bool(true)),
        (DataType::Int, OutputPayload::Int(-1)),
        (DataType::Uint, OutputPayload::Uint(1)),
        (DataType::Float, OutputPayload::Float(1.0)),
        (DataType::Vec2, OutputPayload::FloatArray(vec![1.0, 2.0])),
        (DataType::Vec3, OutputPayload::FloatArray(vec![])),
        (DataType::Vec4, OutputPayload::FloatArray(vec![])),
        (DataType::Ivec2, OutputPayload::FloatArray(vec![])),
        (DataType::Ivec3, OutputPayload::FloatArray(vec![])),
        (DataType::Ivec4, OutputPayload::FloatArray(vec![])),
        (DataType::Uvec2, OutputPayload::FloatArray(vec![])),
        (DataType::Uvec3, OutputPayload::FloatArray(vec![])),
        (DataType::Uvec4, OutputPayload::FloatArray(vec![])),
        (DataType::Bvec2, OutputPayload::FloatArray(vec![])),
        (DataType::Bvec3, OutputPayload::FloatArray(vec![])),
        (DataType::Bvec4, OutputPayload::FloatArray(vec![])),
        (DataType::Mat2, OutputPayload::FloatArray(vec![])),
        (DataType::Mat3, OutputPayload::FloatArray(vec![])),
        (DataType::Mat4, OutputPayload::FloatArray(vec![])),
        (DataType::Sampler2d, OutputPayload::Resource { handle: 1 }),
        (DataType::SamplerCube, OutputPayload::Resource { handle: 1 }),
        (DataType::Roi, OutputPayload::Json(serde_json::json!({}))),
        (DataType::Mesh, OutputPayload::Json(serde_json::json!({}))),
        (DataType::Json, OutputPayload::Json(serde_json::json!({}))),
        (DataType::Auto, OutputPayload::Bytes(vec![])),
    ];
    for (ty, payload) in values {
        assert!(payload_matches(ty, &payload), "{ty:?}");
        if ty != DataType::Auto {
            assert!(!payload_matches(
                ty,
                &OutputPayload::Tensor {
                    handle: 1,
                    dtype: "f32".to_owned(),
                    shape: vec![]
                }
            ));
        }
    }
}

#[test]
fn subscription_validation_covers_empty_ids_zero_dimensions_and_registry_failures() {
    let mut valid = subscription("s");
    assert!(validate_subscription(&valid).is_ok());
    valid.subscription_id.clear();
    assert!(validate_subscription(&valid).is_err());
    valid = subscription("s");
    valid.output.node_id.clear();
    assert!(validate_subscription(&valid).is_err());
    valid = subscription("s");
    valid.output.port_id.clear();
    assert!(validate_subscription(&valid).is_err());
    valid = subscription("s");
    valid.max_width = Some(0);
    assert!(validate_subscription(&valid).is_err());
    valid.max_width = None;
    valid.max_height = Some(0);
    assert!(validate_subscription(&valid).is_err());

    let mut registry = OutputRegistry::default();
    registry.set_every_queue_capacity(0);
    assert!(registry.subscribe(subscription("s")).is_err());
    assert!(registry.update(subscription("missing")).is_err());
    assert!(registry.unsubscribe("missing").is_err());
    assert!(registry
        .validate_contract(&OutputKey::new("none", "none"), &OutputPayload::Float(1.0))
        .is_err());
    assert!(registry.state(&OutputKey::new("none", "none")).is_none());
    assert!(registry.content_stamps().is_empty());
}

fn state(output: &OutputKey, revision: u32, generation: u64) -> OutputState {
    OutputState {
        output: output.clone(),
        graph_revision: revision,
        output_generation: generation,
        evaluation_stamp: FrameStamp {
            epoch: 1,
            frame: generation,
            timeline_ns: generation,
            deadline_ns: generation + 1,
        },
        content_stamp: crate::runtime::ContentStamp {
            epoch: 1,
            timeline_ns: generation,
            media_pts_ns: None,
        },
        payload: OutputPayload::Float(generation as f64),
    }
}

#[test]
fn registry_update_reconcile_publish_generation_and_pending_cleanup_paths() {
    let output = OutputKey::new("node", "port");
    let mut registry = OutputRegistry::default();
    registry.reconcile(1, BTreeMap::from([(output.clone(), DataType::Float)]));
    assert!(registry.subscribe(subscription("")).is_err());

    let mut first = subscription("first");
    first.delivery = DeliveryPolicy::Every;
    let mut second = subscription("second");
    second.delivery = DeliveryPolicy::Every;
    registry.subscribe(first.clone()).unwrap();
    let mut invalid_update = first.clone();
    invalid_update.max_width = Some(0);
    assert!(registry.update(invalid_update).is_err());
    let mut incompatible_update = first.clone();
    incompatible_update.transport = OutputTransport::Preview;
    assert!(registry.update(incompatible_update).is_err());
    registry.subscribe(second.clone()).unwrap();
    registry.reconcile(2, BTreeMap::from([(output.clone(), DataType::Float)]));
    registry.publish(state(&output, 2, 1)).unwrap();

    first.delivery = DeliveryPolicy::Latest;
    registry.update(first).unwrap();
    registry.unsubscribe("second").unwrap();
    assert!(registry.publish(state(&output, 1, 2)).is_err());
    registry.publish(state(&output, 2, 2)).unwrap();
    assert!(registry.publish(state(&output, 2, 2)).is_err());

    let mut on_change = subscription("change");
    on_change.delivery = DeliveryPolicy::OnChange;
    registry.subscribe(on_change).unwrap();
    registry
        .last_enqueued_generation
        .insert("change".to_owned(), 99);
    registry.publish(state(&output, 2, 3)).unwrap();
    assert!(!registry.pending_latest.contains_key("change"));
}
