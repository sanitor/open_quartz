use super::*;
use crate::inference::{InferenceTask, PlacementPolicy, ResidencyPolicy};

fn config() -> InferenceConfig {
    InferenceConfig { model_id: "super-resolution-3x".to_owned(), task: InferenceTask::SuperResolution,
        target_size: 224, policy: ExecutionPolicy::resident(), ..Default::default() }
}
#[test]
fn unsupported_policy_wins_over_missing_model_before_io() {
    let missing = Path::new("missing-inference-model-for-policy-test.onnx");
    for policy in [ExecutionPolicy::default(),
        ExecutionPolicy { residency: ResidencyPolicy::RequireResident, ..ExecutionPolicy::default() },
        ExecutionPolicy { placement: PlacementPolicy::AcceleratorOnly, ..ExecutionPolicy::default() },
        ExecutionPolicy { fallback: FallbackPolicy::AllowCpu, ..ExecutionPolicy::resident() }] {
        let mut config = config(); config.policy = policy;
        let error = PreparedInferenceModel::open_with_policy(missing, config).err().unwrap();
        assert!(error.contains("hardware-resident"), "{error}");
        assert!(!error.contains("Cannot read"), "{error}");
    }
    let error = PreparedInferenceModel::open(missing, config(), true).err().unwrap();
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
    assert_eq!(info.inputs[0].dimensions[0], TensorDimension::Symbol("batch_size".to_owned()));
    assert_eq!(info.source_interface, Some(oq_inference::inspect_onnx(&bytes).unwrap()));
    assert_eq!(info.source_interface.as_ref().unwrap().inputs.len(), 9);
    assert_eq!(info.overridable_initializers.len(), 8);
    assert_eq!(info.artifact.byte_length, bytes.len() as u64);
    assert_eq!(info.artifact.sha256.len(), 64);
    assert_eq!(info.model.producer_name.as_deref(), Some("open-quartz-resident-test"));
    assert_eq!(info.plan.provider_placement, oq_inference::ProviderPlacement::Gpu);
    assert_eq!(info.plan.transfer_path, oq_inference::TransferPath::Device);
    assert_eq!(prepared.task_contract().unwrap().adapter, crate::inference::ImageTaskAdapter::LuminanceSuperResolution3x);
    let (model, owned_config) = prepared.into_parts();
    assert_eq!(model.model_plan().source_byte_length, bytes.len() as u64);
    assert_eq!(owned_config.policy, ExecutionPolicy::resident());
}
#[test]
fn unsupported_tasks_and_invalid_graphs_never_become_reference_fallbacks() {
    let missing = Path::new("missing.onnx");
    let mut unsupported = config(); unsupported.model_id = "unknown-model".to_owned();
    let error = PreparedInferenceModel::open_with_policy(missing, unsupported).err().unwrap();
    assert!(error.contains("seven supported"), "{error}");
    assert!(PreparedInferenceModel::from_memory(b"not ONNX", config()).is_err());
    assert!(PreparedInferenceModel::from_memory(include_bytes!("../../tests/data/image_identity.onnx"), config()).is_err());
}

#[test]
fn resident_catalog_requires_exact_model_task_pairing() {
    for (id, task) in [
        ("super-resolution-3x", InferenceTask::SuperResolution),
        ("realesrgan-x4", InferenceTask::SuperResolution), ("u2netp", InferenceTask::BackgroundRemoval),
        ("modnet", InferenceTask::BackgroundRemoval), ("midas-small", InferenceTask::DepthEstimation),
        ("yolov8n", InferenceTask::Detection), ("yolo26n-sem", InferenceTask::Segmentation),
    ] {
        let mut config = config(); config.model_id = id.to_owned(); config.task = task;
        validate_resident_config(&config).unwrap();
        config.task = InferenceTask::Generic;
        assert!(validate_resident_config(&config).unwrap_err().contains("requires task"));
    }
}
