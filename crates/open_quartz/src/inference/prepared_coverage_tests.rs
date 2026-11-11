use super::*;
use crate::inference::{InferenceTask, PlacementPolicy, ResidencyPolicy};

fn config() -> InferenceConfig {
    InferenceConfig {
        model_id: "super-resolution-3x".to_owned(),
        task: InferenceTask::SuperResolution,
        target_size: 224,
        policy: ExecutionPolicy::resident(),
        ..Default::default()
    }
}
#[test]
fn unsupported_policy_wins_over_missing_model_before_io() {
    let missing = Path::new("missing-inference-model-for-policy-test.onnx");
    for policy in [
        ExecutionPolicy::default(),
        ExecutionPolicy {
            residency: ResidencyPolicy::RequireResident,
            ..ExecutionPolicy::default()
        },
        ExecutionPolicy {
            placement: PlacementPolicy::AcceleratorOnly,
            ..ExecutionPolicy::default()
        },
        ExecutionPolicy {
            fallback: FallbackPolicy::AllowCpu,
            ..ExecutionPolicy::resident()
        },
    ] {
        let mut config = config();
        config.policy = policy;
        let error = PreparedInferenceModel::open_with_policy(missing, config)
            .err()
            .unwrap();
        assert!(error.contains("hardware-resident"), "{error}");
        assert!(!error.contains("Cannot read"), "{error}");
    }
    let error = PreparedInferenceModel::open(missing, config(), true)
        .err()
        .unwrap();
    assert!(error.contains("DirectML"), "{error}");
    assert!(!error.contains("Cannot read"), "{error}");
}
#[test]
fn model_preparation_preserves_source_facts_without_runtime_or_gpu() {
    let bytes = crate::inference::resident_test_model::sr3_model();
    let prepared = PreparedInferenceModel::from_memory(&bytes, config()).unwrap();
    let info = prepared.info();
    assert_eq!(info.backend, "metal");
    assert_eq!(info.input_names, ["input"]);
    assert_eq!(info.output_names, ["output"]);
    assert_eq!(
        info.inputs[0].dimensions[0],
        TensorDimension::Symbol("batch_size".to_owned())
    );
    assert_eq!(
        info.source_interface,
        Some(oq_inference::inspect_onnx(&bytes).unwrap())
    );
    assert_eq!(info.source_interface.as_ref().unwrap().inputs.len(), 9);
    assert_eq!(info.overridable_initializers.len(), 8);
    assert_eq!(info.artifact.byte_length, bytes.len() as u64);
    assert_eq!(info.artifact.sha256.len(), 64);
    assert_eq!(
        info.model.producer_name.as_deref(),
        Some("open-quartz-resident-test")
    );
    assert_eq!(
        info.plan.provider_placement,
        oq_inference::ProviderPlacement::Gpu
    );
    assert_eq!(info.plan.transfer_path, oq_inference::TransferPath::Device);
    assert_eq!(
        prepared.task_contract().unwrap().adapter,
        crate::inference::ImageTaskAdapter::LuminanceSuperResolution3x
    );
    assert_eq!(info.input_values[0].name, "input");
    assert_eq!(
        info.input_values[0].kind,
        ValueKind::Tensor {
            element_type: TensorElementType::Float32,
            dimensions: info.inputs[0].dimensions.clone(),
        }
    );
    assert_eq!(info.input_values[0].optionality, ValueOptionality::Unknown);
    assert_eq!(
        info.overridable_initializers[0].optionality,
        ValueOptionality::Optional
    );
    assert!(info
        .overridable_initializers
        .iter()
        .all(|initializer| initializer.overridable_initializer));
    assert_eq!(
        info.image_interface.expected_input_layout,
        "NCHW FP32 BT.601 luminance [1,1,224,224]"
    );
    assert!(info.image_interface.runnable);
    assert!(info.image_interface.issues.is_empty());
    assert_eq!(info.opset_imports.as_ref().unwrap()[0].version, 10);
    let (model, owned_config) = prepared.into_parts();
    assert_eq!(model.model_plan().source_byte_length, bytes.len() as u64);
    assert_eq!(owned_config.policy, ExecutionPolicy::resident());
}
#[test]
fn unsupported_tasks_and_invalid_graphs_never_become_reference_fallbacks() {
    let missing = Path::new("missing.onnx");
    let mut unsupported = config();
    unsupported.model_id = "unknown-model".to_owned();
    let error = PreparedInferenceModel::open_with_policy(missing, unsupported)
        .err()
        .unwrap();
    assert!(error.contains("seven supported"), "{error}");
    assert!(PreparedInferenceModel::from_memory(b"not ONNX", config()).is_err());
    assert!(PreparedInferenceModel::from_memory(
        include_bytes!("../../tests/data/image_identity.onnx"),
        config()
    )
    .is_err());
}

#[test]
fn resident_catalog_requires_exact_model_task_pairing() {
    for (id, task) in [
        ("super-resolution-3x", InferenceTask::SuperResolution),
        ("realesrgan-x4", InferenceTask::SuperResolution),
        ("u2netp", InferenceTask::BackgroundRemoval),
        ("modnet", InferenceTask::BackgroundRemoval),
        ("midas-small", InferenceTask::DepthEstimation),
        ("yolov8n", InferenceTask::Detection),
        ("yolo26n-sem", InferenceTask::Segmentation),
    ] {
        let mut config = config();
        config.model_id = id.to_owned();
        config.task = task;
        validate_resident_config(&config).unwrap();
        config.task = InferenceTask::Generic;
        assert!(validate_resident_config(&config)
            .unwrap_err()
            .contains("requires task"));
    }
}

#[cfg(target_os = "macos")]
#[test]
fn prepared_catalog_attaches_to_native_player_and_retains_alpha_pixels_after_close() {
    use serde_json::json;
    use std::time::{Duration, Instant};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mask.onnx");
    std::fs::write(&path, crate::inference::resident_test_model::mask_model()).unwrap();
    let config = InferenceConfig {
        model_id: "modnet".into(),
        task: InferenceTask::BackgroundRemoval,
        policy: ExecutionPolicy::resident(),
        target_size: 512,
        ..Default::default()
    };
    assert!(PreparedInferenceModel::open_with_policy(
        &directory.path().join("absent.onnx"),
        config.clone()
    )
    .is_err());
    let graph: crate::Graph = serde_json::from_value(json!({"nodes":[
        {"id":"source","type":"input","position":{"x":0,"y":0},"data":{"type":"input","label":"source","shaderCode":"","inputs":[],"outputs":[{"id":"out","label":"image","dataType":"sampler2D","direction":"output"}],"uniforms":{},"inputMode":"image","inputDataType":"sampler2D"}},
        {"id":"model","type":"inference","position":{"x":0,"y":0},"data":{"type":"inference","label":"mask","shaderCode":"","inputs":[{"id":"in","label":"image","dataType":"sampler2D","direction":"input"}],"outputs":[{"id":"out","label":"mask","dataType":"sampler2D","direction":"output"}],"uniforms":{},"inferenceModelId":"modnet","inferenceTask":"background-removal"}},
        {"id":"renderer","type":"renderer","position":{"x":0,"y":0},"data":{"type":"renderer","label":"output","shaderCode":"","inputs":[{"id":"in","label":"image","dataType":"sampler2D","direction":"input"}],"outputs":[],"uniforms":{}}}
    ],"edges":[{"id":"a","source":"source","sourceHandle":"out","target":"model","targetHandle":"in"},{"id":"b","source":"model","sourceHandle":"out","target":"renderer","targetHandle":"in"}]})).unwrap();
    let (mut player, _) = pollster::block_on(crate::player::NativePlayer::new()).unwrap();
    player.set_graph_snapshot(&graph).unwrap();
    player
        .upload_image("source", &[255, 0, 0, 255].repeat(4), 2, 2)
        .unwrap();
    let prepared = PreparedInferenceModel::open(&path, config, false).unwrap();
    prepared.attach(&mut player, "model".into()).unwrap();
    player.start_playback().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while player.render_next().unwrap().width == 0 {
        assert!(Instant::now() < deadline);
    }
    let held = player.try_stable_preview("renderer").unwrap().unwrap();
    held.ready.wait(Duration::from_secs(5)).unwrap();
    let backend = player.backend();
    assert_eq!(
        pollster::block_on(backend.read_texture_rgba(&held.output().texture, 2, 2)).unwrap(),
        [255, 0, 0, 128].repeat(4)
    );
    let metrics = player.inference_metrics();
    assert_eq!(metrics.accepted, 1);
    assert_eq!(
        metrics.readback_bytes
            + metrics.upload_bytes
            + metrics.input_cpu_bytes
            + metrics.output_cpu_bytes,
        0
    );
    player.close().unwrap();
    assert_eq!(
        pollster::block_on(backend.read_texture_rgba(&held.output().texture, 2, 2)).unwrap(),
        [255, 0, 0, 128].repeat(4)
    );
}
