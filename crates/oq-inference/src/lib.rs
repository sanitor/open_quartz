//! Device-independent inference contracts. Enable `ort` for real ONNX Runtime execution.
pub mod artifact;
pub use artifact::{inspect_onnx, ArtifactError, ArtifactInterface};
#[cfg(feature = "gpu")]
pub mod gpu_model;
#[cfg(feature = "gpu")]
pub mod gpu;
#[cfg(feature = "gpu")]
pub mod catalog_model;
#[cfg(feature = "gpu")]
pub mod catalog_gpu;
#[cfg(feature = "gpu")]
mod catalog_detect;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[cfg(any(test, all(feature = "ort", not(target_arch = "wasm32"))))]
use std::collections::BTreeSet;
use std::fmt;
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackendPreference {
    Auto,
    #[default]
    OrtCpu,
    OrtDirectMl,
    OrtCoreMl,
    MetalGpu,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FallbackPolicy {
    #[default]
    AllowCpu,
    Forbid,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResidencyPolicy {
    #[default]
    CpuTransfersAllowed,
    RequireResident,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlacementPolicy {
    #[default]
    Any,
    HardwareAccelerated,
    AcceleratorOnly,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ExecutionPolicy {
    pub backend: BackendPreference,
    pub fallback: FallbackPolicy,
    pub residency: ResidencyPolicy,
    pub placement: PlacementPolicy,
}
impl ExecutionPolicy {
    /// The application-resident compute path has no permitted host payload fallback.
    pub const fn resident() -> Self {
        Self { backend: BackendPreference::MetalGpu, fallback: FallbackPolicy::Forbid,
            residency: ResidencyPolicy::RequireResident, placement: PlacementPolicy::AcceleratorOnly }
    }

    pub fn validate(&self) -> Result<(), InferenceError> {
        if self.backend == BackendPreference::MetalGpu
            || (self.backend == BackendPreference::Auto && self.residency == ResidencyPolicy::RequireResident
                && self.placement == PlacementPolicy::AcceleratorOnly) {
            if self.fallback != FallbackPolicy::Forbid {
                return Err(InferenceError::InvalidModelInterface("hardware-resident inference forbids CPU fallback".into()));
            }
            return Ok(());
        }
        if self.residency != ResidencyPolicy::CpuTransfersAllowed {
            return Err(InferenceError::UnsupportedResidency(self.residency));
        }
        if self.placement != PlacementPolicy::Any {
            return Err(InferenceError::UnsupportedPlacement(self.placement));
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InferenceProvider {
    #[default]
    Cpu,
    DirectMl,
    CoreMl,
    Metal,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InferenceOptions {
    pub provider: InferenceProvider,
    pub allow_cpu_fallback: bool,
}
impl Default for InferenceOptions {
    fn default() -> Self {
        Self {
            provider: InferenceProvider::Cpu,
            allow_cpu_fallback: true,
        }
    }
}
impl From<InferenceOptions> for ExecutionPolicy {
    fn from(value: InferenceOptions) -> Self {
        Self {
            backend: match value.provider {
                InferenceProvider::Cpu => BackendPreference::OrtCpu,
                InferenceProvider::DirectMl => BackendPreference::OrtDirectMl,
                InferenceProvider::CoreMl => BackendPreference::OrtCoreMl,
                InferenceProvider::Metal => BackendPreference::MetalGpu,
            },
            fallback: if value.allow_cpu_fallback {
                FallbackPolicy::AllowCpu
            } else {
                FallbackPolicy::Forbid
            },
            ..Self::default()
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceCapabilities {
    pub cpu: bool,
    pub direct_ml: bool,
    pub core_ml: bool,
    pub shared_wgpu_device: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderPlacement {
    Unknown,
    Gpu,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransferPath {
    Cpu,
    Device,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionPlan {
    pub requested_policy: ExecutionPolicy,
    pub configured_backend: BackendPreference,
    pub configured_providers: Vec<InferenceProvider>,
    pub provider_placement: ProviderPlacement,
    pub transfer_path: TransferPath,
    pub fallback_reason: Option<String>,
}
/// Select a plan without creating a session or a device. Session construction may
/// explicitly rebuild this as a CPU plan if allowed provider activation fails.
pub fn select_execution_plan(
    policy: &ExecutionPolicy,
    capabilities: &InferenceCapabilities,
) -> Result<ExecutionPlan, InferenceError> {
    policy.validate()?;
    if policy.backend == BackendPreference::MetalGpu || policy.residency == ResidencyPolicy::RequireResident {
        return Err(InferenceError::InvalidModelInterface(
            "Metal GPU execution requires GpuImageSession on the render device, not a host tensor session".into(),
        ));
    }
    let wants_dml = match policy.backend {
        BackendPreference::Auto => capabilities.direct_ml,
        BackendPreference::OrtDirectMl => true,
        BackendPreference::OrtCpu | BackendPreference::OrtCoreMl | BackendPreference::MetalGpu => false,
    };
    let wants_core_ml = match policy.backend {
        BackendPreference::Auto => !wants_dml && capabilities.core_ml,
        BackendPreference::OrtCoreMl => true,
        BackendPreference::OrtCpu | BackendPreference::OrtDirectMl | BackendPreference::MetalGpu => false,
    };
    let mut fallback_reason = None;
    let backend = if wants_dml && capabilities.direct_ml {
        BackendPreference::OrtDirectMl
    } else if wants_dml {
        if policy.fallback == FallbackPolicy::Forbid {
            return Err(InferenceError::ProviderUnavailable(
                InferenceProvider::DirectMl,
            ));
        }
        fallback_reason = Some("DirectML is unavailable; configured CPU explicitly".into());
        BackendPreference::OrtCpu
    } else if wants_core_ml && capabilities.core_ml {
        BackendPreference::OrtCoreMl
    } else if wants_core_ml {
        if policy.fallback == FallbackPolicy::Forbid {
            return Err(InferenceError::ProviderUnavailable(
                InferenceProvider::CoreMl,
            ));
        }
        fallback_reason = Some("CoreML is unavailable; configured CPU explicitly".into());
        BackendPreference::OrtCpu
    } else {
        BackendPreference::OrtCpu
    };
    if backend == BackendPreference::OrtCpu && !capabilities.cpu {
        return Err(InferenceError::ProviderUnavailable(InferenceProvider::Cpu));
    }
    let configured_providers = match backend {
        BackendPreference::OrtDirectMl => {
            if policy.fallback == FallbackPolicy::AllowCpu {
                vec![InferenceProvider::DirectMl, InferenceProvider::Cpu]
            } else {
                vec![InferenceProvider::DirectMl]
            }
        }
        BackendPreference::OrtCoreMl => {
            if policy.fallback == FallbackPolicy::AllowCpu {
                vec![InferenceProvider::CoreMl, InferenceProvider::Cpu]
            } else {
                vec![InferenceProvider::CoreMl]
            }
        }
        BackendPreference::Auto | BackendPreference::OrtCpu | BackendPreference::MetalGpu => vec![InferenceProvider::Cpu],
    };
    Ok(ExecutionPlan {
        requested_policy: *policy,
        configured_backend: backend,
        configured_providers,
        provider_placement: ProviderPlacement::Unknown,
        transfer_path: TransferPath::Cpu,
        fallback_reason,
    })
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionGeneration(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestStamp {
    pub request_id: u64,
    pub session_generation: SessionGeneration,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TensorElementType {
    Float16,
    #[serde(rename = "bfloat16")]
    BFloat16,
    Float32,
    Float64,
    Int8,
    Int16,
    Int32,
    Int64,
    Uint8,
    Uint16,
    Uint32,
    Uint64,
    Bool,
    String,
    Complex64,
    Complex128,
    Unsupported(String),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ValueOptionality {
    Optional,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueMetadata {
    pub name: String,
    pub kind: ValueKind,
    pub optionality: ValueOptionality,
    pub overridable_initializer: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ValueKind {
    Tensor {
        element_type: TensorElementType,
        dimensions: Vec<TensorDimension>,
    },
    Sequence {
        element: Box<ValueKind>,
    },
    Map {
        key_type: TensorElementType,
        value_type: TensorElementType,
    },
    Optional {
        element: Box<ValueKind>,
    },
    Unknown {
        description: String,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TensorMetadata {
    pub name: String,
    pub element_type: TensorElementType,
    pub dimensions: Vec<TensorDimension>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TensorDimension {
    Static(u64),
    Symbol(String),
    Dynamic,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ModelMetadataInfo {
    pub producer_name: Option<String>,
    pub producer_version: Option<String>,
    pub name: Option<String>,
    pub domain: Option<String>,
    pub version: Option<i64>,
    pub description: Option<String>,
    pub graph_description: Option<String>,
    pub custom_metadata: BTreeMap<String, String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpsetImport {
    pub domain: String,
    pub version: i64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ImageInterfaceIssue {
    InputCardinality {
        actual: usize,
    },
    OutputCardinality {
        actual: usize,
    },
    NonTensor {
        name: String,
        kind: ValueKind,
    },
    UnsupportedElementType {
        name: String,
        element_type: TensorElementType,
    },
    InputRank {
        name: String,
        actual: usize,
    },
    OutputRank {
        name: String,
        actual: usize,
    },
    BatchDimension {
        name: String,
        actual: TensorDimension,
    },
    ChannelDimension {
        name: String,
        actual: TensorDimension,
    },
    SpatialDimension {
        name: String,
        axis: usize,
        actual: TensorDimension,
    },
    OutputFormatUnknown {
        name: String,
    },
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageInterfaceReport {
    pub runnable: bool,
    pub expected_input_layout: String,
    pub output_format: String,
    pub issues: Vec<ImageInterfaceIssue>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageInterfaceError(pub ImageInterfaceIssue);
impl fmt::Display for ImageInterfaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Image interface error: {:?}", self.0)
    }
}
impl std::error::Error for ImageInterfaceError {}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelArtifact {
    pub sha256: String,
    pub byte_length: u64,
    pub format: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct InferenceSessionInfo {
    pub input_names: Vec<String>,
    pub output_names: Vec<String>,
    pub backend: String,
    pub inputs: Vec<TensorMetadata>,
    pub outputs: Vec<TensorMetadata>,
    pub input_values: Vec<ValueMetadata>,
    pub output_values: Vec<ValueMetadata>,
    pub overridable_initializers: Vec<ValueMetadata>,
    pub plan: ExecutionPlan,
    pub generation: SessionGeneration,
    pub artifact: ModelArtifact,
    pub model: ModelMetadataInfo,
    pub opset_imports: Option<Vec<OpsetImport>>,
    pub opset_unavailable_reason: Option<String>,
    /// Original declarations, before runtime optimization; includes exact dtype and unknown-rank information.
    pub source_interface: Option<ArtifactInterface>,
    pub image_interface: ImageInterfaceReport,
}
impl Default for InferenceSessionInfo {
    fn default() -> Self {
        Self {
            input_names: Vec::new(),
            output_names: Vec::new(),
            backend: String::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            input_values: Vec::new(),
            output_values: Vec::new(),
            overridable_initializers: Vec::new(),
            plan: ExecutionPlan {
                requested_policy: ExecutionPolicy::default(),
                configured_backend: BackendPreference::default(),
                configured_providers: Vec::new(),
                provider_placement: ProviderPlacement::Unknown,
                transfer_path: TransferPath::Cpu,
                fallback_reason: None,
            },
            generation: SessionGeneration(0),
            artifact: ModelArtifact {
                sha256: String::new(),
                byte_length: 0,
                format: String::new(),
            },
            model: ModelMetadataInfo::default(),
            opset_imports: None,
            opset_unavailable_reason: None,
            source_interface: None,
            image_interface: ImageInterfaceReport::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TensorOutput {
    pub data: Vec<f32>,
    pub shape: Vec<i64>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedTensor {
    pub name: String,
    pub data: Vec<f32>,
    pub shape: Vec<i64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvocationMetrics {
    pub total_duration: Duration,
    pub runtime_duration: Duration,
    /// Host FP32 payload bytes submitted; not a measured GPU transfer count.
    pub input_cpu_bytes: u64,
    /// Host FP32 payload bytes materialized; not a measured GPU transfer count.
    pub output_cpu_bytes: u64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvocationResult {
    pub stamp: RequestStamp,
    pub outputs: Vec<NamedTensor>,
    pub metrics: InvocationMetrics,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TensorLimits {
    pub max_tensor_bytes: u64,
    pub max_invocation_bytes: u64,
}
impl Default for TensorLimits {
    fn default() -> Self {
        Self {
            max_tensor_bytes: 1024 * 1024 * 1024,
            max_invocation_bytes: 2 * 1024 * 1024 * 1024,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InferenceError {
    RuntimeUnavailable(String),
    Runtime {
        operation: &'static str,
        message: String,
    },
    UnsupportedResidency(ResidencyPolicy),
    UnsupportedPlacement(PlacementPolicy),
    ProviderUnavailable(InferenceProvider),
    UnsupportedType {
        name: String,
        element_type: String,
    },
    UnknownName(String),
    DuplicateName(String),
    MissingInput(String),
    EmptyOutputs,
    InvalidModelInterface(String),
    ImageInterface(ImageInterfaceError),
    Rank {
        name: String,
        expected: usize,
        actual: usize,
    },
    Dimension {
        name: String,
        axis: usize,
        expected: String,
        actual: i64,
    },
    TensorLength {
        name: String,
        expected: usize,
        actual: usize,
    },
    Capacity {
        name: String,
        bytes: u64,
        limit: u64,
    },
    SizeOverflow(String),
    StaleSession {
        expected: SessionGeneration,
        actual: SessionGeneration,
    },
}
impl fmt::Display for InferenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Inference error: {self:?}")
    }
}
impl std::error::Error for InferenceError {}

#[cfg(any(test, all(feature = "ort", not(target_arch = "wasm32"))))]
fn tensor_bytes(
    tensor: &NamedTensor,
    metadata: &TensorMetadata,
    symbols: &mut BTreeMap<String, i64>,
    limits: TensorLimits,
) -> Result<u64, InferenceError> {
    validate_tensor(
        &tensor.name,
        &tensor.shape,
        tensor.data.len(),
        metadata,
        symbols,
        limits,
    )
}
#[cfg(any(test, all(feature = "ort", not(target_arch = "wasm32"))))]
fn validate_tensor(
    name: &str,
    shape: &[i64],
    data_length: usize,
    metadata: &TensorMetadata,
    symbols: &mut BTreeMap<String, i64>,
    limits: TensorLimits,
) -> Result<u64, InferenceError> {
    if metadata.element_type != TensorElementType::Float32 {
        return Err(InferenceError::UnsupportedType {
            name: name.into(),
            element_type: format!("{:?}", metadata.element_type),
        });
    }
    if shape.len() != metadata.dimensions.len() {
        return Err(InferenceError::Rank {
            name: name.into(),
            expected: metadata.dimensions.len(),
            actual: shape.len(),
        });
    }
    let mut length = 1usize;
    for (axis, (&actual, dimension)) in shape.iter().zip(&metadata.dimensions).enumerate() {
        let valid = if actual < 0 {
            false
        } else {
            match dimension {
                TensorDimension::Static(expected) => actual as u64 == *expected,
                TensorDimension::Symbol(symbol) => match symbols.get(symbol) {
                    Some(expected) => actual == *expected,
                    None => {
                        symbols.insert(symbol.clone(), actual);
                        true
                    }
                },
                TensorDimension::Dynamic => true,
            }
        };
        if !valid {
            return Err(InferenceError::Dimension {
                name: name.into(),
                axis,
                expected: format!("{dimension:?}"),
                actual,
            });
        }
        length = length
            .checked_mul(
                usize::try_from(actual).map_err(|_| InferenceError::SizeOverflow(name.into()))?,
            )
            .ok_or_else(|| InferenceError::SizeOverflow(name.into()))?;
    }
    let bytes = u64::try_from(length)
        .ok()
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| InferenceError::SizeOverflow(name.into()))?;
    if bytes > limits.max_tensor_bytes {
        return Err(InferenceError::Capacity {
            name: name.into(),
            bytes,
            limit: limits.max_tensor_bytes,
        });
    }
    if length != data_length {
        return Err(InferenceError::TensorLength {
            name: name.into(),
            expected: length,
            actual: data_length,
        });
    }
    Ok(bytes)
}
#[cfg(any(test, all(feature = "ort", not(target_arch = "wasm32"))))]
fn validate_names(
    names: impl Iterator<Item = String>,
    metadata: &[TensorMetadata],
    require_all: bool,
) -> Result<(), InferenceError> {
    let mut seen = BTreeSet::new();
    for name in names {
        if !metadata.iter().any(|m| m.name == name) {
            return Err(InferenceError::UnknownName(name));
        }
        if !seen.insert(name.clone()) {
            return Err(InferenceError::DuplicateName(name));
        }
    }
    if require_all {
        for m in metadata {
            if !seen.contains(&m.name) {
                return Err(InferenceError::MissingInput(m.name.clone()));
            }
        }
    }
    Ok(())
}

#[cfg(all(feature = "ort", not(target_arch = "wasm32")))]
mod runtime;
#[cfg(all(feature = "ort", not(target_arch = "wasm32")))]
pub use runtime::{inference_capabilities, InferenceSession};
#[cfg(not(all(feature = "ort", not(target_arch = "wasm32"))))]
pub fn inference_capabilities() -> Result<InferenceCapabilities, InferenceError> {
    Ok(InferenceCapabilities::default())
}

#[cfg(test)]
mod contract_tests;
