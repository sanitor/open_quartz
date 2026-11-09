use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use oq_inference::gpu_model::{GpuModel, ModelPlan};
use oq_inference::artifact::{ArtifactDimensionValue, ArtifactInputRequirement, ArtifactShape, ArtifactValue, ArtifactValueKind};
use oq_inference::{BackendPreference, ExecutionPolicy, FallbackPolicy, InferenceSessionInfo,
    ModelArtifact, ModelMetadataInfo, OpsetImport, PlacementPolicy, ResidencyPolicy,
    SessionGeneration, TensorDimension, TensorElementType, TensorMetadata,
    ValueKind, ValueMetadata, ValueOptionality};
use super::InferenceConfig;

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

/// CPU-side model parsing and immutable weight preparation only. No runtime or GPU is opened here.
pub struct PreparedInferenceModel {
    model: GpuModel,
    config: InferenceConfig,
    info: InferenceSessionInfo,
}

pub(crate) fn validate_resident_policy(policy: &ExecutionPolicy) -> Result<(), String> {
    if !matches!(policy.backend, BackendPreference::MetalGpu | BackendPreference::Auto)
        || policy.fallback != FallbackPolicy::Forbid
        || policy.residency != ResidencyPolicy::RequireResident
        || policy.placement != PlacementPolicy::AcceleratorOnly {
        return Err("Production inference requires hardware-resident Metal GPU execution, accelerator-only placement and no CPU fallback or host transfer".to_owned());
    }
    Ok(())
}

pub(crate) fn validate_resident_config(config: &InferenceConfig) -> Result<(), String> {
    validate_resident_policy(&config.policy)?;
    if config.task != super::InferenceTask::SuperResolution || config.model_id != "super-resolution-3x" {
        return Err("Resident inference capability supports only task super-resolution with model super-resolution-3x (Conv/ReLU/PixelShuffle)".to_owned());
    }
    Ok(())
}

impl PreparedInferenceModel {
    /// The historical DirectML option is incompatible with device-resident inference.
    pub fn open(path: &Path, config: InferenceConfig, prefer_direct_ml: bool) -> Result<Self, String> {
        if prefer_direct_ml {
            return Err("Legacy DirectML host-transfer inference cannot satisfy hardware residency; select Metal GPU".to_owned());
        }
        Self::open_with_policy(path, config)
    }
    pub fn open_with_policy(path: &Path, config: InferenceConfig) -> Result<Self, String> {
        validate_resident_config(&config)?;
        if !cfg!(target_os = "macos") {
            return Err("Metal GPU resident inference backend unavailable on this platform; CPU fallback forbidden".to_owned());
        }
        let bytes = fs::read(path)
            .map_err(|error| format!("Cannot read inference model {}: {error}", path.display()))?;
        Self::from_memory(&bytes, config)
    }
    pub fn from_memory(bytes: &[u8], config: InferenceConfig) -> Result<Self, String> {
        validate_resident_config(&config)?;
        let model = GpuModel::from_onnx(bytes)?;
        let info = resident_session_info(&model.plan, &config)?;
        super::image_task_contract(&config, &info)?;
        Ok(Self { model, config, info })
    }
    pub fn info(&self) -> &InferenceSessionInfo { &self.info }
    pub fn session_info(&self) -> &InferenceSessionInfo { &self.info }
    pub fn task_contract(&self) -> Result<super::ImageTaskContract, String> {
        super::image_task_contract(&self.config, &self.info)
    }
    pub fn into_parts(self) -> (GpuModel, InferenceConfig) { (self.model, self.config) }
    pub fn attach(self, runtime: &mut crate::player::NativePlayer, node_id: String) -> Result<InferenceSessionInfo, String> {
        let backend = runtime.backend();
        let session = oq_inference::gpu::GpuImageSession::prepare(backend.device.clone(), backend.queue.clone(), self.model)
            .map_err(|error| error.to_string())?;
        runtime.load_resident_inference(node_id, session, self.config)
    }
}

fn source_tensor(value: &ArtifactValue) -> Result<TensorMetadata, String> {
    let Some(ty) = &value.value_type else { return Err("resident model has an untyped value".into()); };
    let ArtifactValueKind::Tensor { element_type: Some(1), shape: ArtifactShape::KnownRank(shape) } = &ty.kind else {
        return Err("resident model requires source FP32 tensors with declared rank".into());
    };
    let dimensions = shape.iter().map(|dimension| match &dimension.value {
        ArtifactDimensionValue::Static(value) => u64::try_from(*value).map(TensorDimension::Static)
            .map_err(|_| "resident model declares a negative dimension".to_owned()),
        ArtifactDimensionValue::Symbol(value) => Ok(TensorDimension::Symbol(value.clone())),
        ArtifactDimensionValue::Dynamic => Ok(TensorDimension::Dynamic),
    }).collect::<Result<_, _>>()?;
    Ok(TensorMetadata { name: value.name.clone(), element_type: TensorElementType::Float32, dimensions })
}
fn value_metadata(tensor: &TensorMetadata, initializer: bool) -> ValueMetadata {
    ValueMetadata { name: tensor.name.clone(), kind: ValueKind::Tensor {
        element_type: tensor.element_type.clone(), dimensions: tensor.dimensions.clone() },
        optionality: if initializer { ValueOptionality::Optional } else { ValueOptionality::Unknown },
        overridable_initializer: initializer }
}

pub(crate) fn resident_session_info(plan: &ModelPlan, config: &InferenceConfig) -> Result<InferenceSessionInfo, String> {
    validate_resident_config(config)?;
    let source = &plan.source_interface;
    let inputs = source.inputs.iter().filter(|input| input.requirement != ArtifactInputRequirement::OverridableInitializer)
        .map(|input| source_tensor(&input.value)).collect::<Result<Vec<_>, _>>()?;
    let outputs = source.outputs.iter().map(source_tensor).collect::<Result<Vec<_>, _>>()?;
    let overridable_initializers = source.inputs.iter()
        .filter(|input| input.requirement == ArtifactInputRequirement::OverridableInitializer)
        .map(|input| source_tensor(&input.value).map(|tensor| value_metadata(&tensor, true)))
        .collect::<Result<Vec<_>, _>>()?;
    let opset_imports = source.opset_imports.iter().map(|opset| opset.version.map(|version| OpsetImport {
        domain: opset.domain.clone().unwrap_or_default(), version })).collect::<Option<Vec<_>>>();
    let opset_unavailable_reason = opset_imports.is_none()
        .then(|| "source model contains an opset import without a version".to_owned());
    Ok(InferenceSessionInfo {
        input_names: inputs.iter().map(|tensor| tensor.name.clone()).collect(),
        output_names: outputs.iter().map(|tensor| tensor.name.clone()).collect(),
        backend: "metal".to_owned(),
        input_values: inputs.iter().map(|tensor| value_metadata(tensor, false)).collect(),
        output_values: outputs.iter().map(|tensor| value_metadata(tensor, false)).collect(),
        inputs, outputs, overridable_initializers,
        plan: oq_inference::ExecutionPlan { requested_policy: config.policy.clone(),
            configured_backend: BackendPreference::MetalGpu, configured_providers: vec![oq_inference::InferenceProvider::Metal],
            provider_placement: oq_inference::ProviderPlacement::Gpu, transfer_path: oq_inference::TransferPath::Device,
            fallback_reason: None },
        generation: SessionGeneration(NEXT_GENERATION.fetch_add(1, Ordering::Relaxed)),
        artifact: ModelArtifact { sha256: plan.source_sha256.clone(), byte_length: plan.source_byte_length, format: "onnx".to_owned() },
        model: ModelMetadataInfo { producer_name: source.producer_name.clone(), producer_version: source.producer_version.clone(),
            name: source.graph_name.clone(), domain: source.domain.clone(), version: source.model_version,
            description: source.doc_string.clone(), graph_description: source.graph_doc_string.clone(), custom_metadata: source.metadata.clone() },
        opset_imports, opset_unavailable_reason, source_interface: Some(source.clone()),
        image_interface: oq_inference::ImageInterfaceReport { runnable: true,
            expected_input_layout: "NCHW FP32 BT.601 luminance [1,1,224,224]".to_owned(),
            output_format: "NCHW FP32 luminance [1,1,672,672]; device RGBA output at source dimensions ×3".to_owned(),
            issues: Vec::new() },
    })
}

#[cfg(test)]
#[path = "prepared_coverage_tests.rs"]
mod tests;
