use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use oq_inference::gpu_model::{GpuModel, ModelPlan};
use oq_inference::catalog_model::CatalogModel;
use oq_inference::catalog_gpu::{CatalogGpuSession, CatalogOptions};
use oq_inference::gpu::{GpuCounters, GpuDeviceOwner, GpuError, GpuImageSession};
use std::sync::Arc;
use oq_inference::artifact::{ArtifactDimensionValue, ArtifactInputRequirement, ArtifactShape, ArtifactValue, ArtifactValueKind};
use oq_inference::{BackendPreference, ExecutionPolicy, FallbackPolicy, InferenceSessionInfo,
    ModelArtifact, ModelMetadataInfo, OpsetImport, PlacementPolicy, ResidencyPolicy,
    SessionGeneration, TensorDimension, TensorElementType, TensorMetadata,
    ValueKind, ValueMetadata, ValueOptionality};
use super::InferenceConfig;

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
static NEXT_FRAME_ALLOCATION: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);

struct InferenceFrameAllocator(Arc<crate::resources::FrameBudget>);
impl oq_inference::gpu::GpuFrameAllocator for InferenceFrameAllocator {
    fn reserve(&self, surfaces: usize, bytes: u64)
        -> Result<Option<Arc<dyn std::any::Any + Send + Sync>>, GpuError> {
        let key=NEXT_FRAME_ALLOCATION.fetch_update(Ordering::Relaxed,Ordering::Relaxed,|id|id.checked_add(1))
            .map_err(|_|GpuError::Validation("inference frame allocation identity exhausted".into()))?;
        self.0.try_acquire((2,key),surfaces,bytes).map(|credit|credit.map(|credit|Arc::new(credit) as Arc<dyn std::any::Any+Send+Sync>))
            .map_err(GpuError::Validation)
    }
}

pub enum PreparedModel {
    Sr3(GpuModel),
    Catalog(CatalogModel),
}

impl PreparedModel {
    pub fn model_plan(&self) -> &ModelPlan {
        match self { Self::Sr3(model) => &model.plan, Self::Catalog(model) => &model.plan }
    }
}

/// CPU-side model parsing and immutable weight preparation only. No runtime or GPU is opened here.
pub struct PreparedInferenceModel {
    model: PreparedModel,
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
    use super::InferenceTask::*;
    let expected = match config.model_id.as_str() {
        "super-resolution-3x" | "realesrgan-x4" => SuperResolution,
        "u2netp" | "modnet" => BackgroundRemoval,
        "midas-small" => DepthEstimation,
        "yolov8n" => Detection,
        "yolo26n-sem" => Segmentation,
        _ => return Err("Resident inference capability requires one of the seven supported catalog model IDs".to_owned()),
    };
    if config.task != expected {
        return Err(format!("Resident inference capability for {} requires task {expected:?}, received {:?}", config.model_id, config.task));
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
        let model = if config.model_id == "super-resolution-3x" {
            PreparedModel::Sr3(GpuModel::from_onnx(bytes)?)
        } else {
            let side = match config.model_id.as_str() {
                "u2netp" => 320, "modnet" => 512, "midas-small" => 256,
                "yolov8n" | "yolo26n-sem" => 640, _ => 224,
            };
            PreparedModel::Catalog(CatalogModel::from_onnx(bytes, &config.model_id, side, side)?)
        };
        let info = resident_session_info(model.model_plan(), &config)?;
        let contract = super::image_task_contract(&config, &info)?;
        let (input_shape, output_shape) = match &model {
            PreparedModel::Sr3(_) => (vec![1, 1, 224, 224], vec![1, 1, 672, 672]),
            PreparedModel::Catalog(model) => (model.input_shape.iter().map(|&v| v as i64).collect(),
                model.output_shape.iter().map(|&v| v as i64).collect()),
        };
        contract.validate_prepared_shapes(&input_shape, &output_shape)?;
        Ok(Self { model, config, info })
    }
    pub fn info(&self) -> &InferenceSessionInfo { &self.info }
    pub fn session_info(&self) -> &InferenceSessionInfo { &self.info }
    pub fn task_contract(&self) -> Result<super::ImageTaskContract, String> {
        super::image_task_contract(&self.config, &self.info)
    }
    pub fn into_parts(self) -> (PreparedModel, InferenceConfig) { (self.model, self.config) }
    pub fn attach(self, runtime: &mut crate::player::NativePlayer, node_id: String) -> Result<InferenceSessionInfo, String> {
        let backend = runtime.backend();
        let session = ResidentInferenceSession::prepare(backend.device.clone(), backend.queue.clone(), self.model, &self.config)?;
        runtime.load_resident_inference(node_id, session, self.config)
    }
}

/// A single coordinator-facing interface for every supported resident catalog model.
pub enum ResidentInferenceSession {
    Sr3(GpuImageSession),
    Catalog(CatalogGpuSession),
}

impl ResidentInferenceSession {
    fn prepare(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>, model: PreparedModel,
        config: &InferenceConfig) -> Result<Self, String> {
        match model {
            PreparedModel::Sr3(model) => GpuImageSession::prepare(device, queue, model).map(Self::Sr3),
            PreparedModel::Catalog(model) => CatalogGpuSession::prepare(device, queue, model,
                CatalogOptions { score_threshold: config.score_threshold, iou_threshold: config.iou_threshold }).map(Self::Catalog),
        }.map_err(|error| error.to_string())
    }
    pub fn device_owner(&self) -> &GpuDeviceOwner {
        match self { Self::Sr3(session) => session.device_owner(), Self::Catalog(session) => session.device_owner() }
    }
    pub fn model_plan(&self) -> &ModelPlan {
        match self { Self::Sr3(session) => session.model_plan(), Self::Catalog(session) => session.model_plan() }
    }
    pub fn counters(&self) -> GpuCounters {
        match self { Self::Sr3(session) => session.counters(), Self::Catalog(session) => session.counters() }
    }
    pub(crate) fn submit_texture(&mut self, texture: &wgpu::Texture, width: u32, height: u32)
        -> Result<super::tasks::ResidentImage, GpuError> {
        match self {
            Self::Sr3(session) => session.submit_texture(texture, width, height).map(super::tasks::ResidentImage::Gpu),
            Self::Catalog(session) => session.submit_texture(texture, width, height).map(super::tasks::ResidentImage::Catalog),
        }
    }
    /// Install renderer-owned accounting for application-owned resident allocations.
    pub fn set_frame_allocator(&mut self, allocator: Option<Arc<dyn oq_inference::gpu::GpuFrameAllocator>>) -> Result<(), String> {
        match self { Self::Sr3(session) => session.set_frame_allocator(allocator),
            Self::Catalog(session) => session.set_frame_allocator(allocator) }.map_err(|e|e.to_string())
    }
    pub fn set_frame_budget(&mut self, budget: Arc<crate::resources::FrameBudget>) -> Result<(), String> {
        self.set_frame_allocator(Some(Arc::new(InferenceFrameAllocator(budget.clone()))))?;
        let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(move || budget.notify_waiters());
        match self { Self::Sr3(session) => session.set_output_waker(waker),
            Self::Catalog(session) => session.set_output_waker(waker) }
        Ok(())
    }
    /// Reserve the inference output working set before source decoding.
    pub fn prepare_output_frame(&mut self, width: u32, height: u32) -> Result<(), GpuError> {
        match self { Self::Sr3(session) => session.prepare_outputs(width,height,2),
            Self::Catalog(session) => session.prepare_outputs(width,height,2) }
    }
}

fn source_tensor(value: &ArtifactValue) -> Result<TensorMetadata, String> {
    let Some(ty) = &value.value_type else { return Err("resident model has an untyped value".into()); };
    let ArtifactValueKind::Tensor { element_type: Some(dtype), shape: ArtifactShape::KnownRank(shape) } = &ty.kind else {
        return Err("resident model requires source tensors with declared dtype and rank".into());
    };
    let element_type = match dtype {
        1 => TensorElementType::Float32,
        2 => TensorElementType::Uint8,
        _ => return Err(format!("resident model has unsupported source tensor dtype {dtype}")),
    };
    let dimensions = shape.iter().map(|dimension| match &dimension.value {
        ArtifactDimensionValue::Static(value) => u64::try_from(*value).map(TensorDimension::Static)
            .map_err(|_| "resident model declares a negative dimension".to_owned()),
        ArtifactDimensionValue::Symbol(value) => Ok(TensorDimension::Symbol(value.clone())),
        ArtifactDimensionValue::Dynamic => Ok(TensorDimension::Dynamic),
    }).collect::<Result<_, _>>()?;
    Ok(TensorMetadata { name: value.name.clone(), element_type, dimensions })
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
            expected_input_layout: match config.model_id.as_str() {
                "super-resolution-3x" => "NCHW FP32 BT.601 luminance [1,1,224,224]",
                "u2netp" => "NCHW FP32 RGB [0,1] [1,3,320,320]",
                "modnet" => "NCHW FP32 RGB [0,1] [1,3,512,512]",
                "midas-small" => "NCHW FP32 RGB [0,1] [1,3,256,256]; normalization embedded in graph",
                "yolov8n" | "yolo26n-sem" => "NCHW FP32 RGB [0,1] [1,3,640,640]; letterbox",
                _ => "NCHW FP32 RGB [0,1] [1,3,H,W]; source dimensions",
            }.to_owned(),
            output_format: match config.model_id.as_str() {
                "super-resolution-3x" => "NCHW FP32 luminance [1,1,672,672]; device RGBA at source dimensions ×3",
                "realesrgan-x4" => "NCHW FP32 RGB [1,3,4H,4W]; device RGBA at source dimensions ×4",
                "u2netp" => "FP32 primary alpha mask output 1959; all seven original outputs retained in metadata",
                "modnet" => "NCHW FP32 alpha mask [1,1,512,512]; device RGBA at source dimensions",
                "midas-small" => "FP32 depth [1,256,256]; device grayscale RGBA at source dimensions",
                "yolov8n" => "FP32 COCO detections [1,84,8400]; device overlay and structured detection buffer",
                _ => "UInt8 class map [1,640,640]; device segmentation overlay at source dimensions",
            }.to_owned(),
            issues: Vec::new() },
    })
}

#[cfg(test)]
#[path = "prepared_coverage_tests.rs"]
mod tests;
