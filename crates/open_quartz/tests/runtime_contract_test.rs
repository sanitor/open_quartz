use open_quartz::ffi::EngineState;
use open_quartz::runtime::{
    public_surface_manifest, AsyncCompletionEnvelope, ContentStamp, DataPathMode, DeliveryPolicy,
    FrameStamp, OutputDeliveryBatch, OutputKey, OutputPayload, OutputState, OutputSubscription,
    OutputTransport, PresentationFit, PresentationItem, PresentationSet, ResourceDescriptor,
    Runtime, RuntimeCapabilities, RuntimeFrameInput, Viewport,
};
use open_quartz::types::Graph;
use serde_json::json;

fn stamp(frame: u64) -> FrameStamp {
    FrameStamp {
        epoch: 3,
        frame,
        timeline_ns: frame * 16_666_667,
        deadline_ns: (frame + 1) * 16_666_667,
    }
}

#[test]
fn runtime_contract_round_trips_canonical_wire_schema() {
    let output = OutputKey::new("math-1", "result");
    let state = OutputState {
        output: output.clone(),
        graph_revision: 7,
        output_generation: 11,
        evaluation_stamp: stamp(5),
        content_stamp: ContentStamp {
            epoch: 3,
            timeline_ns: 66_666_668,
            media_pts_ns: Some(64_000_000),
        },
        payload: OutputPayload::Float(0.75),
    };
    let subscription = OutputSubscription {
        subscription_id: "inspector".to_owned(),
        output: output.clone(),
        delivery: DeliveryPolicy::OnChange,
        transport: OutputTransport::Value,
        max_width: None,
        max_height: None,
    };
    let batch = OutputDeliveryBatch {
        frame_stamp: Some(stamp(5)),
        deliveries: vec![state.clone()],
    };

    assert_eq!(
        serde_json::from_value::<OutputState>(serde_json::to_value(&state).unwrap()).unwrap(),
        state
    );
    assert_eq!(
        serde_json::from_value::<OutputSubscription>(serde_json::to_value(&subscription).unwrap())
            .unwrap(),
        subscription
    );
    assert_eq!(
        serde_json::from_value::<OutputDeliveryBatch>(serde_json::to_value(&batch).unwrap())
            .unwrap(),
        batch
    );
    assert_eq!(
        serde_json::to_value(DeliveryPolicy::OnChange).unwrap(),
        json!("on-change")
    );
    assert_eq!(
        serde_json::to_value(OutputTransport::NativePresent).unwrap(),
        json!("native-present")
    );
}

#[test]
fn presentation_and_async_completion_keep_original_stamps() {
    let output = OutputKey::new("onnx-1", "overlay");
    let content_stamp = ContentStamp {
        epoch: 2,
        timeline_ns: 100,
        media_pts_ns: Some(90),
    };
    let item = PresentationItem {
        output: output.clone(),
        resource_handle: 42,
        viewport: Viewport {
            x: 0.0,
            y: 0.0,
            width: 640.0,
            height: 360.0,
        },
        fit: PresentationFit::Contain,
        z_index: 0,
        evaluation_stamp: stamp(9),
        content_stamp: content_stamp.clone(),
    };
    let set = PresentationSet {
        group_id: "main".to_owned(),
        frame_stamp: stamp(9),
        items: vec![item],
    };
    let completion = AsyncCompletionEnvelope {
        node_id: "onnx-1".to_owned(),
        graph_revision: 4,
        node_generation: 8,
        input_stamp: stamp(4),
        content_stamp,
        outputs: vec![(output, OutputPayload::Resource { handle: 42 })],
    };

    assert_eq!(
        serde_json::from_value::<PresentationSet>(serde_json::to_value(&set).unwrap()).unwrap(),
        set
    );
    assert_eq!(
        serde_json::from_value::<AsyncCompletionEnvelope>(
            serde_json::to_value(&completion).unwrap()
        )
        .unwrap(),
        completion
    );
}

#[test]
fn capabilities_name_copy_boundaries_instead_of_claiming_zero_copy() {
    let capabilities = RuntimeCapabilities {
        data_paths: vec![
            DataPathMode::CpuCopy,
            DataPathMode::ExternalFrameNoCpuReadback,
            DataPathMode::SharedGpu,
            DataPathMode::NativePresent,
        ],
    };
    let value = serde_json::to_value(capabilities).unwrap();
    assert_eq!(
        value["dataPaths"],
        json!([
            "cpu-copy",
            "external-frame/no-cpu-readback",
            "shared-gpu",
            "native-present"
        ])
    );
}

#[test]
fn public_surface_is_host_neutral_and_versioned() {
    let manifest = public_surface_manifest();
    assert!(manifest.api_version > 0);
    assert!(manifest.methods.contains(&"set_graph"));
    assert!(manifest.methods.contains(&"subscribe_output"));
    assert!(manifest.methods.contains(&"update_presentation"));
    assert!(manifest.methods.contains(&"drain_deliveries"));
    assert!(manifest.methods.iter().all(|name| !name.starts_with("web_")
        && !name.starts_with("tauri_")
        && !name.starts_with("native_")));
}

#[test]
fn direct_runtime_client_drives_the_same_public_lifecycle() {
    let mut runtime = Runtime::new(RuntimeCapabilities {
        data_paths: vec![DataPathMode::CpuCopy],
    });
    assert_eq!(runtime.state(), EngineState::Empty);
    assert_eq!(
        runtime
            .set_graph(&Graph {
                nodes: vec![],
                edges: vec![]
            })
            .unwrap(),
        1
    );
    runtime
        .register_resource(
            ResourceDescriptor {
                resource_id: "image-1".to_owned(),
                kind: "image".to_owned(),
            },
            7,
        )
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
    assert_eq!(runtime.state(), EngineState::Running);
    runtime.pause().unwrap();
    assert_eq!(runtime.state(), EngineState::Paused);
    runtime.resume().unwrap();
    runtime.stop().unwrap();
    assert_eq!(runtime.remove_resource("image-1").unwrap(), 7);
    runtime.dispose();
    assert_eq!(runtime.state(), EngineState::Disposed);
}
