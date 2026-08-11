use open_quartz::engine::{ExecutionCommand, ExecutionPlan, GpuFacade};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

struct RecordingBackend {
    registered: Arc<AtomicU64>,
    removed: Arc<AtomicU64>,
}

impl open_quartz::runtime::HostBackend for RecordingBackend {
    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities {
            data_paths: vec![DataPathMode::NativePresent],
        }
    }

    fn register_resource(
        &mut self,
        _descriptor: &ResourceDescriptor,
        handle: u64,
    ) -> Result<(), open_quartz::SdkError> {
        self.registered.store(handle, Ordering::Release);
        Ok(())
    }

    fn remove_resource(
        &mut self,
        _resource_id: &str,
        handle: u64,
    ) -> Result<(), open_quartz::SdkError> {
        self.removed.store(handle, Ordering::Release);
        Ok(())
    }

    fn present(&mut self, _set: &PresentationSet) -> Result<(), open_quartz::SdkError> {
        Ok(())
    }
}

use open_quartz::runtime::{
    public_surface_manifest, AsyncCompletionEnvelope, ContentStamp, DataPathMode, DeliveryPolicy,
    FrameStamp, HostBackend, OutputDelivery, OutputDeliveryBatch, OutputKey, OutputPayload,
    OutputState, OutputSubscription, OutputTransport, PresentationFit, PresentationItem,
    PresentationSet, ResourceDescriptor, Runtime, RuntimeCapabilities, RuntimeFrameInput, Viewport,
};
use open_quartz::types::Graph;
use open_quartz::EngineState;
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
        deliveries: vec![OutputDelivery {
            subscription_id: "inspector".to_owned(),
            state: state.clone(),
        }],
        invalidations: vec![],
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
    runtime.play(100).unwrap();
    runtime
        .advance(&RuntimeFrameInput {
            now_ns: 116,
            date: [2026.0, 8.0, 3.0, 0.0],
            mouse: [0.0; 4],
            resolution: [640.0, 360.0, 1.0],
        })
        .unwrap();
    assert_eq!(runtime.state(), EngineState::Running);
    runtime.pause(140).unwrap();
    assert_eq!(runtime.state(), EngineState::Paused);
    runtime.resume(1_000).unwrap();
    runtime.stop().unwrap();
    assert_eq!(runtime.remove_resource("image-1").unwrap(), 7);
    runtime.dispose().unwrap();
    assert_eq!(runtime.state(), EngineState::Disposed);
}

#[test]
fn direct_runtime_owns_resource_policy_and_forwards_only_handles_to_backend() {
    let registered = Arc::new(AtomicU64::new(0));
    let removed = Arc::new(AtomicU64::new(0));
    let mut runtime = Runtime::new(RuntimeCapabilities { data_paths: vec![] });
    runtime.attach_backend(Box::new(RecordingBackend {
        registered: registered.clone(),
        removed: removed.clone(),
    }));
    runtime
        .register_resource(
            ResourceDescriptor {
                resource_id: "video".to_owned(),
                kind: "video".to_owned(),
            },
            99,
        )
        .unwrap();
    assert_eq!(
        runtime.capabilities().data_paths,
        vec![DataPathMode::NativePresent]
    );
    assert_eq!(registered.load(Ordering::Acquire), 99);
    assert_eq!(runtime.remove_resource("video").unwrap(), 99);
    assert_eq!(removed.load(Ordering::Acquire), 99);
}

#[test]
fn runtime_rejects_stale_async_completions_and_preserves_launch_stamp() {
    let mut runtime = Runtime::new(RuntimeCapabilities { data_paths: vec![] });
    runtime.set_graph(&serde_json::from_value(json!({
        "nodes": [
            {
                "id": "onnx-1",
                "type": "onnx",
                "position": { "x": 0.0, "y": 0.0 },
                "data": {
                    "type": "onnx", "label": "ONNX", "shaderCode": "", "inputs": [],
                    "outputs": [{ "id": "result", "label": "result", "dataType": "json", "direction": "output" }],
                    "uniforms": {}
                }
            },
            {
                "id": "renderer",
                "type": "renderer",
                "position": { "x": 100.0, "y": 0.0 },
                "data": {
                    "type": "renderer", "label": "Renderer", "shaderCode": "",
                    "inputs": [{ "id": "input", "label": "input", "dataType": "json", "direction": "input" }],
                    "outputs": [], "uniforms": {}
                }
            }
        ],
        "edges": [{
            "id": "onnx-renderer", "source": "onnx-1", "sourceHandle": "result",
            "target": "renderer", "targetHandle": "input"
        }]
    })).unwrap()).unwrap();
    runtime
        .subscribe_output(OutputSubscription {
            subscription_id: "result".to_owned(),
            output: OutputKey::new("onnx-1", "result"),
            delivery: DeliveryPolicy::Latest,
            transport: OutputTransport::Value,
            max_width: None,
            max_height: None,
        })
        .unwrap();
    runtime.play(0).unwrap();
    runtime
        .advance(&RuntimeFrameInput {
            now_ns: 16,
            date: [0.0; 4],
            mouse: [0.0; 4],
            resolution: [640.0, 360.0, 1.0],
        })
        .unwrap();
    runtime.drain_commands();
    let completion = AsyncCompletionEnvelope {
        node_id: "onnx-1".to_owned(),
        graph_revision: runtime.revision(),
        node_generation: runtime.node_generation("onnx-1").unwrap(),
        input_stamp: stamp(4),
        content_stamp: ContentStamp {
            epoch: 3,
            timeline_ns: 60,
            media_pts_ns: Some(55),
        },
        outputs: vec![(
            OutputKey::new("onnx-1", "result"),
            OutputPayload::Json(json!({ "ok": true })),
        )],
    };
    runtime.submit_completion(completion.clone()).unwrap();
    let delivered = runtime.drain_deliveries().deliveries.remove(0).state;
    assert_eq!(delivered.evaluation_stamp, completion.input_stamp);
    assert_eq!(delivered.content_stamp, completion.content_stamp);
    runtime
        .advance(&RuntimeFrameInput {
            now_ns: 32,
            date: [0.0; 4],
            mouse: [0.0; 4],
            resolution: [640.0, 360.0, 1.0],
        })
        .unwrap();
    assert_eq!(
        runtime
            .drain_commands()
            .iter()
            .map(|command| command.node_id.as_str())
            .collect::<Vec<_>>(),
        vec!["renderer"]
    );
    let mut stale = completion;
    stale.graph_revision = 0;
    assert!(runtime.submit_completion(stale).is_err());
}

struct FailingRemoveBackend;

impl HostBackend for FailingRemoveBackend {
    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities { data_paths: vec![] }
    }
    fn register_resource(
        &mut self,
        _: &ResourceDescriptor,
        _: u64,
    ) -> Result<(), open_quartz::SdkError> {
        Ok(())
    }
    fn remove_resource(&mut self, _: &str, _: u64) -> Result<(), open_quartz::SdkError> {
        Err(open_quartz::SdkError::new(
            open_quartz::SdkErrorCode::InvalidResource,
            "release failed",
        ))
    }
    fn present(&mut self, _: &PresentationSet) -> Result<(), open_quartz::SdkError> {
        Ok(())
    }
}

#[test]
fn failed_backend_release_keeps_runtime_resource_owned() {
    let mut runtime = Runtime::new(RuntimeCapabilities { data_paths: vec![] });
    runtime.attach_backend(Box::new(FailingRemoveBackend));
    runtime
        .register_resource(
            ResourceDescriptor {
                resource_id: "owned".to_owned(),
                kind: "texture".to_owned(),
            },
            7,
        )
        .unwrap();
    assert!(runtime.remove_resource("owned").is_err());
    assert!(runtime.remove_resource("owned").is_err());
    assert!(runtime.dispose().is_err());
}

#[test]
fn canonical_runtime_is_send_and_selects_native_video_bindings() {
    fn assert_send<T: Send>() {}
    assert_send::<Runtime>();

    let graph: Graph = serde_json::from_value(json!({
        "nodes": [
            {
                "id": "video",
                "type": "input",
                "position": { "x": 0.0, "y": 0.0 },
                "data": {
                    "type": "input",
                    "label": "Video",
                    "shaderCode": "",
                    "inputs": [],
                    "outputs": [{
                        "id": "video_out",
                        "label": "output",
                        "dataType": "sampler2D",
                        "direction": "output"
                    }],
                    "uniforms": {},
                    "inputMode": "video",
                    "inputDataType": "sampler2D"
                }
            },
            {
                "id": "shader",
                "type": "shader",
                "position": { "x": 1.0, "y": 0.0 },
                "data": {
                    "type": "shader",
                    "label": "Copy",
                    "shaderCode": "@fragment fn main(@location(0) uv: vec2f) -> @location(0) vec4f { return textureSample(inputImage, inputImageSampler, uv); }",
                    "inputs": [{
                        "id": "shader_in",
                        "label": "inputImage",
                        "dataType": "sampler2D",
                        "direction": "input"
                    }],
                    "outputs": [],
                    "uniforms": {}
                }
            }
        ],
        "edges": [{
            "id": "video-to-shader",
            "source": "video",
            "sourceHandle": "video_out",
            "target": "shader",
            "targetHandle": "shader_in"
        }]
    }))
    .unwrap();

    let mut browser = Runtime::new(RuntimeCapabilities { data_paths: vec![] });
    browser.set_graph(&graph).unwrap();
    let browser_shader = browser
        .execution_plan()
        .unwrap()
        .nodes
        .iter()
        .find(|node| node.id == "shader")
        .unwrap()
        .shader
        .as_ref()
        .unwrap();
    assert!(browser_shader
        .external_texture_bindings
        .contains_key("inputImage"));

    let mut native = Runtime::new_native(RuntimeCapabilities { data_paths: vec![] });
    native.set_graph(&graph).unwrap();
    let native_shader = native
        .execution_plan()
        .unwrap()
        .nodes
        .iter()
        .find(|node| node.id == "shader")
        .unwrap()
        .shader
        .as_ref()
        .unwrap();
    assert!(native_shader.external_texture_bindings.is_empty());
    assert!(native_shader.texture_bindings.contains_key("inputImage"));
}

struct RecordingGpuFacade(Arc<AtomicU64>);

impl GpuFacade for RecordingGpuFacade {
    fn execute(
        &mut self,
        _plan: &ExecutionPlan,
        commands: &[ExecutionCommand],
    ) -> Result<(), open_quartz::SdkError> {
        self.0.store(commands.len() as u64, Ordering::Release);
        Ok(())
    }
}

#[test]
fn runtime_dispatches_engine_work_through_the_gpu_facade() {
    let graph: Graph = serde_json::from_value(json!({
        "nodes": [{
            "id": "shader",
            "type": "shader",
            "position": { "x": 0.0, "y": 0.0 },
            "data": {
                "type": "shader",
                "label": "Color",
                "shaderCode": "@fragment fn main() -> @location(0) vec4f { return vec4f(1.0); }",
                "inputs": [],
                "outputs": [],
                "uniforms": {}
            }
        }],
        "edges": []
    }))
    .unwrap();
    let mut runtime = Runtime::new(RuntimeCapabilities { data_paths: vec![] });
    runtime.set_graph(&graph).unwrap();
    runtime.play(100).unwrap();
    runtime
        .advance(&RuntimeFrameInput {
            now_ns: 116,
            date: [0.0; 4],
            mouse: [0.0; 4],
            resolution: [640.0, 360.0, 1.0],
        })
        .unwrap();
    let commands = runtime.drain_commands();
    let observed = Arc::new(AtomicU64::new(0));
    let mut facade = RecordingGpuFacade(observed.clone());

    runtime.execute_gpu(&mut facade, &commands).unwrap();

    assert_eq!(observed.load(Ordering::Acquire), commands.len() as u64);
    assert!(!commands.is_empty());
}
