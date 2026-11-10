use super::*;
use ort::{
    ep::{ExecutionProvider, CPU},
    session::{OutputSelector, RunOptions, Session},
    value::ValueType,
};
use sha2::{Digest, Sha256};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        LazyLock,
    },
    time::Instant,
};

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
static INITIALIZED: LazyLock<Result<(), String>> = LazyLock::new(|| {
    // Explicit loading returns missing-library/version errors instead of ort's
    // implicit loader's expect panic. API 21 targets the shipped runtime 1.21.
    let path = std::env::var_os("ORT_DYLIB_PATH")
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| {
            if cfg!(target_os = "windows") {
                "onnxruntime.dll".into()
            } else if cfg!(target_os = "macos") {
                "libonnxruntime.dylib".into()
            } else {
                "libonnxruntime.so".into()
            }
        });
    ort::init_from(std::path::PathBuf::from(path))
        .map(|builder| {
            builder.commit();
        })
        .map_err(|error| error.to_string())
});
fn initialize() -> Result<(), InferenceError> {
    INITIALIZED
        .clone()
        .map_err(InferenceError::RuntimeUnavailable)
}
fn runtime_error(operation: &'static str, error: impl fmt::Display) -> InferenceError {
    InferenceError::Runtime {
        operation,
        message: error.to_string(),
    }
}
pub fn inference_capabilities() -> Result<InferenceCapabilities, InferenceError> {
    initialize()?;
    #[cfg(target_os = "windows")]
    let direct_ml = ort::ep::DirectML::default()
        .is_available()
        .map_err(|e| runtime_error("query DirectML", e))?;
    #[cfg(not(target_os = "windows"))]
    let direct_ml = false;
    #[cfg(target_os = "macos")]
    let core_ml = ort::ep::CoreML::default()
        .is_available()
        .map_err(|e| runtime_error("query CoreML", e))?;
    #[cfg(not(target_os = "macos"))]
    let core_ml = false;
    let cpu = CPU::default()
        .is_available()
        .map_err(|e| runtime_error("query CPU", e))?;
    Ok(InferenceCapabilities {
        cpu,
        direct_ml,
        core_ml,
        shared_wgpu_device: false,
    })
}

/// Reusable real ORT session; owns model state independently of any graph or GPU.
pub struct InferenceSession {
    session: Session,
    info: InferenceSessionInfo,
}
impl InferenceSession {
    pub fn from_memory(model: &[u8]) -> Result<Self, InferenceError> {
        Self::from_memory_with_policy(model, &ExecutionPolicy::default())
    }
    pub fn from_memory_with_options(
        model: &[u8],
        options: InferenceOptions,
    ) -> Result<Self, InferenceError> {
        Self::from_memory_with_policy(model, &options.into())
    }
    pub fn from_memory_with_policy(
        model: &[u8],
        policy: &ExecutionPolicy,
    ) -> Result<Self, InferenceError> {
        Self::prepare(model, policy, None)
    }

    /// Profiling is opt-in; the returned trace reports actual runtime provider assignments.
    pub fn from_memory_with_profile(
        model: &[u8],
        policy: &ExecutionPolicy,
        profile_prefix: &std::path::Path,
    ) -> Result<Self, InferenceError> {
        Self::prepare(model, policy, Some(profile_prefix))
    }

    fn prepare(
        model: &[u8],
        policy: &ExecutionPolicy,
        profile_prefix: Option<&std::path::Path>,
    ) -> Result<Self, InferenceError> {
        policy.validate()?;
        if policy.backend == BackendPreference::MetalGpu {
            return Err(InferenceError::InvalidModelInterface("resident Metal execution requires GpuImageSession, not host tensors".into()));
        }
        let source_interface = inspect_onnx(model)
            .map_err(|error| InferenceError::InvalidModelInterface(error.to_string()))?;
        let capabilities = inference_capabilities()?;
        let mut plan = select_execution_plan(policy, &capabilities)?;
        let session = match build_session(model, &plan, profile_prefix) {
            Ok(session) => session,
            Err(error)
                if matches!(
                    plan.configured_backend,
                    BackendPreference::OrtDirectMl | BackendPreference::OrtCoreMl
                ) && policy.fallback == FallbackPolicy::AllowCpu =>
            {
                let provider_name = match plan.configured_backend {
                    BackendPreference::OrtCoreMl => "CoreML",
                    _ => "DirectML",
                };
                let cpu_policy = ExecutionPolicy {
                    backend: BackendPreference::OrtCpu,
                    ..*policy
                };
                plan = select_execution_plan(&cpu_policy, &capabilities)?;
                plan.requested_policy = *policy;
                plan.fallback_reason = Some(format!(
                    "{provider_name} activation failed; explicitly rebuilt CPU session: {error}"
                ));
                build_session(model, &plan, profile_prefix)?
            }
            Err(error) => return Err(error),
        };
        let inputs = session
            .inputs()
            .iter()
            .map(|m| metadata(m.name(), m.dtype()))
            .collect::<Result<Vec<_>, _>>()?;
        let outputs = session
            .outputs()
            .iter()
            .map(|m| metadata(m.name(), m.dtype()))
            .collect::<Result<Vec<_>, _>>()?;
        let overridable_initializers = session
            .overridable_initializers()
            .iter()
            .map(|m| value_metadata(m.name(), m.dtype(), true))
            .collect::<Vec<_>>();
        let initializer_names = overridable_initializers
            .iter()
            .map(|m| m.name.as_str())
            .collect::<BTreeSet<_>>();
        let input_values = session
            .inputs()
            .iter()
            .map(|m| value_metadata(m.name(), m.dtype(), initializer_names.contains(m.name())))
            .collect::<Vec<_>>();
        let output_values = session
            .outputs()
            .iter()
            .map(|m| value_metadata(m.name(), m.dtype(), false))
            .collect::<Vec<_>>();
        if inputs.is_empty() || outputs.is_empty() {
            return Err(InferenceError::InvalidModelInterface(
                "model requires at least one input and output".into(),
            ));
        }
        for group in [&inputs, &outputs] {
            let mut seen = BTreeSet::new();
            for item in group {
                if item.name.is_empty() || item.name.contains('\0') {
                    return Err(InferenceError::InvalidModelInterface(
                        "empty or NUL tensor name".into(),
                    ));
                }
                if !seen.insert(&item.name) {
                    return Err(InferenceError::DuplicateName(item.name.clone()));
                }
            }
        }
        let backend = match plan.configured_backend {
            BackendPreference::OrtCpu => "cpu",
            BackendPreference::OrtCoreMl
                if plan.configured_providers.contains(&InferenceProvider::Cpu) =>
            {
                "coreml+cpu"
            }
            BackendPreference::OrtCoreMl => "coreml",
            BackendPreference::OrtDirectMl
                if plan.configured_providers.contains(&InferenceProvider::Cpu) =>
            {
                "directml+cpu"
            }
            BackendPreference::OrtDirectMl => "directml",
            BackendPreference::MetalGpu => return Err(InferenceError::InvalidModelInterface("resident execution cannot use ORT host tensors".into())),
            BackendPreference::Auto => "unknown",
        }
        .into();
        let mut model_metadata = read_model_metadata(&session)?;
        model_metadata.producer_version = source_interface.producer_version.clone();
        let opset_imports = source_interface
            .opset_imports
            .iter()
            .map(|opset| {
                opset.version.map(|version| OpsetImport {
                    domain: opset.domain.clone().unwrap_or_default(),
                    version,
                })
            })
            .collect::<Option<Vec<_>>>();
        let opset_unavailable_reason = if opset_imports.is_none() {
            Some("source model contains an opset import without a version".to_owned())
        } else {
            None
        };
        let image_interface = inspect_image_interface(&input_values, &output_values);
        let info = InferenceSessionInfo {
            input_names: inputs.iter().map(|m| m.name.clone()).collect(),
            output_names: outputs.iter().map(|m| m.name.clone()).collect(),
            backend,
            inputs,
            outputs,
            input_values,
            output_values,
            overridable_initializers,
            plan,
            generation: SessionGeneration(NEXT_GENERATION.fetch_add(1, Ordering::Relaxed)),
            artifact: ModelArtifact {
                sha256: format!("{:x}", Sha256::digest(model)),
                byte_length: model.len() as u64,
                format: "onnx".into(),
            },
            model: model_metadata,
            opset_imports,
            opset_unavailable_reason,
            source_interface: Some(source_interface),
            image_interface,
        };
        Ok(Self { session, info })
    }
    pub fn info(&self) -> &InferenceSessionInfo {
        &self.info
    }
    pub fn plan(&self) -> &ExecutionPlan {
        &self.info.plan
    }
    pub fn generation(&self) -> SessionGeneration {
        self.info.generation
    }
    /// Finish an explicitly enabled profiling session and return its trace path.
    pub fn end_profiling(&mut self) -> Result<String, InferenceError> {
        self.session
            .end_profiling()
            .map_err(|e| runtime_error("finish profiling", e))
    }
    pub fn validate_single_image(&self) -> Result<(), ImageInterfaceError> {
        validate_image_interface(&self.info.input_values, &self.info.output_values, false)
            .map(|_| ())
    }
    /// Validate the supported NCHW FP32 interface before graph pixel readback.
    /// ONNX does not encode color, normalization, or task-specific decoder semantics.
    pub fn validate_image_shape(&self) -> Result<(), ImageInterfaceError> {
        validate_image_interface(&self.info.input_values, &self.info.output_values, true)
            .map(|_| ())
    }
    pub fn run_f32(
        &mut self,
        input: Vec<f32>,
        shape: Vec<i64>,
    ) -> Result<TensorOutput, InferenceError> {
        let stamp = RequestStamp {
            request_id: 0,
            session_generation: self.generation(),
        };
        let result = self.run_f32_scoped(stamp, input, shape)?;
        let output = result
            .outputs
            .into_iter()
            .next()
            .ok_or(InferenceError::EmptyOutputs)?;
        Ok(TensorOutput {
            data: output.data,
            shape: output.shape,
        })
    }
    /// Run the original single-input FP32 / single-output UInt8 model on the
    /// explicitly selected host runtime. This diagnostic API does not cast or
    /// rewrite model outputs and is not a resident GPU execution path.
    pub fn run_u8(
        &mut self,
        input: Vec<f32>,
        shape: Vec<i64>,
    ) -> Result<(Vec<u8>, Vec<i64>), InferenceError> {
        if self.info.inputs.len() != 1 || self.info.outputs.len() != 1 {
            return Err(InferenceError::InvalidModelInterface(
                "UInt8 reference execution requires exactly one input and one output".into(),
            ));
        }
        let input_metadata = &self.info.inputs[0];
        let output_metadata = &self.info.outputs[0];
        if let Some(source) = &self.info.source_interface {
            for value in source.inputs.iter().map(|input| &input.value).chain(&source.outputs) {
                if !matches!(
                    value.value_type.as_ref().map(|ty| &ty.kind),
                    Some(crate::artifact::ArtifactValueKind::Tensor {
                        shape: crate::artifact::ArtifactShape::KnownRank(_), ..
                    })
                ) {
                    return Err(InferenceError::InvalidModelInterface(
                        "UInt8 reference execution requires tensor interfaces with known rank".into(),
                    ));
                }
            }
        }
        if output_metadata.element_type != TensorElementType::Uint8 {
            return Err(InferenceError::UnsupportedType {
                name: output_metadata.name.clone(),
                element_type: format!("{:?}", output_metadata.element_type),
            });
        }
        let limits = TensorLimits::default();
        let mut symbols = BTreeMap::new();
        let input_bytes = validate_tensor(
            &input_metadata.name, &shape, input.len(), input_metadata, &mut symbols, limits,
        )?;
        checked_total(0, input_bytes, limits)?;
        // Bound all output dimensions known before execution; data-dependent
        // dimensions are checked against the actual tensor before copying it.
        let known_shape = output_metadata.dimensions.iter().map(|dimension| match dimension {
            TensorDimension::Static(n) => i64::try_from(*n)
                .map(Some).map_err(|_| InferenceError::SizeOverflow(output_metadata.name.clone())),
            TensorDimension::Symbol(symbol) => Ok(symbols.get(symbol).copied()),
            TensorDimension::Dynamic => Ok(None),
        }).collect::<Result<Option<Vec<_>>, _>>()?;
        if let Some(shape) = known_shape {
            let bytes = validate_u8_output(&shape, None, output_metadata, &mut symbols, limits)?;
            checked_total(input_bytes, bytes, limits)?;
        }
        let value = ort::value::Tensor::from_array((shape, input))
            .map_err(|e| runtime_error("create input tensor", e))?;
        let results = self.session.run(vec![(input_metadata.name.as_str(), value)])
            .map_err(|e| runtime_error("run model", e))?;
        let output = results.get(&output_metadata.name)
            .ok_or_else(|| InferenceError::UnknownName(output_metadata.name.clone()))?;
        let (shape, data) = output.try_extract_tensor::<u8>()
            .map_err(|e| runtime_error("extract UInt8 output", e))?;
        let bytes = validate_u8_output(shape, Some(data.len()), output_metadata, &mut symbols, limits)?;
        checked_total(input_bytes, bytes, limits)?;
        Ok((data.to_vec(), shape.to_vec()))
    }
    pub fn run_f32_scoped(
        &mut self,
        stamp: RequestStamp,
        input: Vec<f32>,
        shape: Vec<i64>,
    ) -> Result<InvocationResult, InferenceError> {
        self.validate_single_image()
            .map_err(InferenceError::ImageInterface)?;
        self.invoke(
            stamp,
            vec![NamedTensor {
                name: self.info.input_names[0].clone(),
                data: input,
                shape,
            }],
            &self.info.output_names.clone(),
        )
    }
    pub fn invoke(
        &mut self,
        stamp: RequestStamp,
        inputs: Vec<NamedTensor>,
        output_names: &[String],
    ) -> Result<InvocationResult, InferenceError> {
        self.invoke_with_limits(stamp, inputs, output_names, TensorLimits::default())
    }
    /// Invoke task-owned outputs whose concrete shapes are a deterministic function
    /// of the prepared input (for example source-scaled image reconstruction).
    /// The ONNX source interface remains unchanged; only the caller's exact expected
    /// shapes replace symbolic name-equality inference for the named result tensors.
    pub fn invoke_with_output_shapes(
        &mut self,
        stamp: RequestStamp,
        inputs: Vec<NamedTensor>,
        output_names: &[String],
        expected_shapes: &[(&str, &[i64])],
    ) -> Result<InvocationResult, InferenceError> {
        self.invoke_with_output_shapes_and_limits(stamp, inputs, output_names, expected_shapes, TensorLimits::default())
    }
    pub fn invoke_with_limits(
        &mut self,
        stamp: RequestStamp,
        inputs: Vec<NamedTensor>,
        output_names: &[String],
        limits: TensorLimits,
    ) -> Result<InvocationResult, InferenceError> {
        self.invoke_with_output_shapes_and_limits(stamp, inputs, output_names, &[], limits)
    }
    fn invoke_with_output_shapes_and_limits(
        &mut self,
        stamp: RequestStamp,
        inputs: Vec<NamedTensor>,
        output_names: &[String],
        expected_shapes: &[(&str, &[i64])],
        limits: TensorLimits,
    ) -> Result<InvocationResult, InferenceError> {
        let started = Instant::now();
        if stamp.session_generation != self.generation() {
            return Err(InferenceError::StaleSession {
                expected: self.generation(),
                actual: stamp.session_generation,
            });
        }
        validate_names(
            inputs.iter().map(|t| t.name.clone()),
            &self.info.inputs,
            true,
        )?;
        if output_names.is_empty() {
            return Err(InferenceError::EmptyOutputs);
        }
        validate_names(output_names.iter().cloned(), &self.info.outputs, false)?;
        let mut symbols = BTreeMap::new();
        let mut input_cpu_bytes = 0u64;
        for tensor in &inputs {
            let metadata = self
                .info
                .inputs
                .iter()
                .find(|m| m.name == tensor.name)
                .ok_or_else(|| InferenceError::UnknownName(tensor.name.clone()))?;
            let bytes = tensor_bytes(tensor, metadata, &mut symbols, limits)?;
            input_cpu_bytes = checked_total(input_cpu_bytes, bytes, limits)?;
        }
        for name in output_names {
            let m = self
                .info
                .outputs
                .iter()
                .find(|m| &m.name == name)
                .ok_or_else(|| InferenceError::UnknownName(name.clone()))?;
            if m.element_type != TensorElementType::Float32 {
                return Err(InferenceError::UnsupportedType {
                    name: name.clone(), element_type: format!("{:?}", m.element_type),
                });
            }
            if let Some((_, shape)) = expected_shapes.iter().find(|(expected,_)| *expected == name) {
                if shape.len() != m.dimensions.len() { return Err(InferenceError::Rank { name:name.clone(), expected:m.dimensions.len(), actual:shape.len() }); }
                if shape.iter().any(|dimension| *dimension <= 0) { return Err(InferenceError::InvalidModelInterface(format!("task-owned output {name} dimensions must be positive"))); }
                let mut elements = 1usize;
                for (axis, &dimension) in shape.iter().enumerate() {
                    if let TensorDimension::Static(expected) = &m.dimensions[axis] {
                        if dimension as u64 != *expected { return Err(InferenceError::Dimension { name:name.clone(), axis, expected:format!("{expected}"), actual:dimension }); }
                    }
                    elements = elements.checked_mul(dimension as usize).ok_or_else(||InferenceError::SizeOverflow(name.clone()))?;
                }
                let bytes = u64::try_from(elements).ok().and_then(|value|value.checked_mul(4)).ok_or_else(||InferenceError::SizeOverflow(name.clone()))?;
                if bytes > limits.max_tensor_bytes { return Err(InferenceError::Capacity { name:name.clone(), bytes, limit:limits.max_tensor_bytes }); }
            }
        }
        let values = inputs
            .into_iter()
            .map(|tensor| {
                ort::value::Tensor::from_array((tensor.shape, tensor.data))
                    .map(|value| (tensor.name, value))
                    .map_err(|e| runtime_error("create input tensor", e))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let selector = output_names
            .iter()
            .fold(OutputSelector::no_default(), |selector, name| {
                selector.with(name.clone())
            });
        let options = RunOptions::new()
            .map_err(|e| runtime_error("create run options", e))?
            .with_outputs(selector);
        let runtime_started = Instant::now();
        let results = self
            .session
            .run_with_options(values, &options)
            .map_err(|e| runtime_error("run model", e))?;
        let runtime_duration = runtime_started.elapsed();
        let mut outputs = Vec::with_capacity(output_names.len());
        let mut output_cpu_bytes = 0u64;
        for name in output_names {
            let value = results
                .get(name)
                .ok_or_else(|| InferenceError::UnknownName(name.clone()))?;
            let (shape, data) = value
                .try_extract_tensor::<f32>()
                .map_err(|e| runtime_error("extract FP32 output", e))?;
            let metadata = self.info.outputs.iter().find(|m| &m.name == name)
                .ok_or_else(|| InferenceError::UnknownName(name.clone()))?;
            let override_shape=expected_shapes.iter().find(|(expected,_)|*expected==name).map(|(_,shape)|*shape);
            let bytes = if let Some(expected) = override_shape {
                let actual = shape.to_vec();
                if actual.as_slice() != expected { return Err(InferenceError::Dimension { name:name.clone(), axis:0, expected:format!("task-owned shape {expected:?}"), actual:actual.first().copied().unwrap_or(-1) }); }
                let elements = expected.iter().try_fold(1usize, |n, &d| n.checked_mul(usize::try_from(d).ok()?)).ok_or_else(||InferenceError::SizeOverflow(name.clone()))?;
                if elements != data.len() { return Err(InferenceError::TensorLength { name:name.clone(), expected:elements, actual:data.len() }); }
                u64::try_from(elements).ok().and_then(|n|n.checked_mul(4)).ok_or_else(||InferenceError::SizeOverflow(name.clone()))?
            } else { validate_tensor(name, shape, data.len(), metadata, &mut symbols, limits)? };
            output_cpu_bytes = checked_total(output_cpu_bytes, bytes, limits)?;
            checked_total(input_cpu_bytes, output_cpu_bytes, limits)?;
            outputs.push(NamedTensor {
                name: name.clone(),
                data: data.to_vec(),
                shape: shape.to_vec(),
            });
        }
        Ok(InvocationResult {
            stamp,
            outputs,
            metrics: InvocationMetrics {
                total_duration: started.elapsed(),
                runtime_duration,
                input_cpu_bytes,
                output_cpu_bytes,
            },
        })
    }
}
fn validate_u8_output(
    shape: &[i64],
    data_length: Option<usize>,
    metadata: &TensorMetadata,
    symbols: &mut BTreeMap<String, i64>,
    limits: TensorLimits,
) -> Result<u64, InferenceError> {
    let name = &metadata.name;
    if shape.len() != metadata.dimensions.len() {
        return Err(InferenceError::Rank {
            name: name.clone(), expected: metadata.dimensions.len(), actual: shape.len(),
        });
    }
    let mut length = 1usize;
    for (axis, (&actual, dimension)) in shape.iter().zip(&metadata.dimensions).enumerate() {
        let valid = actual >= 0 && match dimension {
            TensorDimension::Static(expected) => actual as u64 == *expected,
            TensorDimension::Symbol(symbol) => match symbols.get(symbol) {
                Some(expected) => actual == *expected,
                None => { symbols.insert(symbol.clone(), actual); true }
            },
            TensorDimension::Dynamic => true,
        };
        if !valid {
            return Err(InferenceError::Dimension {
                name: name.clone(), axis, expected: format!("{dimension:?}"), actual,
            });
        }
        length = length.checked_mul(usize::try_from(actual)
            .map_err(|_| InferenceError::SizeOverflow(name.clone()))?)
            .ok_or_else(|| InferenceError::SizeOverflow(name.clone()))?;
    }
    let bytes = u64::try_from(length).map_err(|_| InferenceError::SizeOverflow(name.clone()))?;
    if bytes > limits.max_tensor_bytes {
        return Err(InferenceError::Capacity { name: name.clone(), bytes, limit: limits.max_tensor_bytes });
    }
    if let Some(actual) = data_length {
        if actual != length {
            return Err(InferenceError::TensorLength { name: name.clone(), expected: length, actual });
        }
    }
    Ok(bytes)
}
fn checked_total(total: u64, bytes: u64, limits: TensorLimits) -> Result<u64, InferenceError> {
    let bytes = total
        .checked_add(bytes)
        .ok_or_else(|| InferenceError::SizeOverflow("invocation".into()))?;
    if bytes > limits.max_invocation_bytes {
        return Err(InferenceError::Capacity {
            name: "invocation".into(),
            bytes,
            limit: limits.max_invocation_bytes,
        });
    }
    Ok(bytes)
}
fn tensor_dimensions(shape: &[i64], symbols: &[String]) -> Vec<TensorDimension> {
    shape
        .iter()
        .enumerate()
        .map(|(axis, &dim)| {
            if dim >= 0 {
                TensorDimension::Static(dim as u64)
            } else {
                symbols
                    .get(axis)
                    .filter(|s| !s.is_empty())
                    .map_or(TensorDimension::Dynamic, |s| {
                        TensorDimension::Symbol(s.clone())
                    })
            }
        })
        .collect()
}
fn element_type(ty: ort::value::TensorElementType) -> TensorElementType {
    use ort::value::TensorElementType as OrtType;
    match ty {
        OrtType::Float16 => TensorElementType::Float16,
        OrtType::Bfloat16 => TensorElementType::BFloat16,
        OrtType::Float32 => TensorElementType::Float32,
        OrtType::Float64 => TensorElementType::Float64,
        OrtType::Int8 => TensorElementType::Int8,
        OrtType::Int16 => TensorElementType::Int16,
        OrtType::Int32 => TensorElementType::Int32,
        OrtType::Int64 => TensorElementType::Int64,
        OrtType::Uint8 => TensorElementType::Uint8,
        OrtType::Uint16 => TensorElementType::Uint16,
        OrtType::Uint32 => TensorElementType::Uint32,
        OrtType::Uint64 => TensorElementType::Uint64,
        OrtType::Bool => TensorElementType::Bool,
        OrtType::String => TensorElementType::String,
        OrtType::Complex64 => TensorElementType::Complex64,
        OrtType::Complex128 => TensorElementType::Complex128,
        other => TensorElementType::Unsupported(format!("{other:?}")),
    }
}
fn value_kind(value_type: &ValueType) -> ValueKind {
    match value_type {
        ValueType::Tensor {
            ty,
            shape,
            dimension_symbols,
        } => ValueKind::Tensor {
            element_type: element_type(*ty),
            dimensions: tensor_dimensions(shape, dimension_symbols),
        },
        ValueType::Sequence(element) => ValueKind::Sequence {
            element: Box::new(value_kind(element)),
        },
        ValueType::Map { key, value } => ValueKind::Map {
            key_type: element_type(*key),
            value_type: element_type(*value),
        },
        ValueType::Optional(element) => ValueKind::Optional {
            element: Box::new(value_kind(element)),
        },
    }
}
fn value_metadata(
    name: &str,
    value_type: &ValueType,
    overridable_initializer: bool,
) -> ValueMetadata {
    let kind = value_kind(value_type);
    let optionality = if matches!(&kind, ValueKind::Optional { .. }) {
        ValueOptionality::Optional
    } else {
        ValueOptionality::Unknown
    };
    ValueMetadata {
        name: name.into(),
        kind,
        optionality,
        overridable_initializer,
    }
}
fn metadata(name: &str, value_type: &ValueType) -> Result<TensorMetadata, InferenceError> {
    let (element_type, dimensions) = match value_type {
        ValueType::Tensor {
            ty,
            shape,
            dimension_symbols,
        } => (
            element_type(*ty),
            tensor_dimensions(shape, dimension_symbols),
        ),
        _ => (
            TensorElementType::Unsupported(format!("non-tensor {value_type:?}")),
            Vec::new(),
        ),
    };
    Ok(TensorMetadata {
        name: name.into(),
        element_type,
        dimensions,
    })
}
fn read_model_metadata(session: &Session) -> Result<ModelMetadataInfo, InferenceError> {
    let metadata = session
        .metadata()
        .map_err(|e| runtime_error("read model metadata", e))?;
    let mut custom_metadata = BTreeMap::new();
    for key in metadata
        .custom_keys()
        .map_err(|e| runtime_error("read custom metadata keys", e))?
    {
        if let Some(value) = metadata.custom(&key) {
            custom_metadata.insert(key, value);
        }
    }
    Ok(ModelMetadataInfo {
        producer_name: metadata.producer(),
        producer_version: None,
        name: metadata.name(),
        domain: metadata.domain(),
        version: metadata.version(),
        description: metadata.description(),
        graph_description: metadata.graph_description(),
        custom_metadata,
    })
}
fn inspect_image_interface(
    inputs: &[ValueMetadata],
    outputs: &[ValueMetadata],
) -> ImageInterfaceReport {
    let mut issues = Vec::new();
    if inputs.len() != 1 {
        issues.push(ImageInterfaceIssue::InputCardinality {
            actual: inputs.len(),
        });
    }
    if outputs.len() != 1 {
        issues.push(ImageInterfaceIssue::OutputCardinality {
            actual: outputs.len(),
        });
    }
    if outputs.len() == 1 {
        issues.push(ImageInterfaceIssue::OutputFormatUnknown {
            name: outputs[0].name.clone(),
        });
    }
    if let Some(input) = inputs.first() {
        inspect_image_value(input, &mut issues);
    }
    ImageInterfaceReport {
        runnable: false,
        expected_input_layout:
            "NCHW FP32; color interpretation and normalization are task contracts, not ONNX facts"
                .into(),
        output_format: "unknown; output rank/layout/decoder are task-specific and not inferred"
            .into(),
        issues,
    }
}
fn inspect_image_value(value: &ValueMetadata, issues: &mut Vec<ImageInterfaceIssue>) {
    let ValueKind::Tensor {
        element_type,
        dimensions,
    } = &value.kind
    else {
        issues.push(ImageInterfaceIssue::NonTensor {
            name: value.name.clone(),
            kind: value.kind.clone(),
        });
        return;
    };
    if *element_type != TensorElementType::Float32 {
        issues.push(ImageInterfaceIssue::UnsupportedElementType {
            name: value.name.clone(),
            element_type: element_type.clone(),
        });
    }
    if dimensions.len() != 4 {
        issues.push(ImageInterfaceIssue::InputRank {
            name: value.name.clone(),
            actual: dimensions.len(),
        });
        return;
    }
    if !matches!(
        dimensions[0],
        TensorDimension::Dynamic | TensorDimension::Symbol(_) | TensorDimension::Static(1)
    ) {
        issues.push(ImageInterfaceIssue::BatchDimension {
            name: value.name.clone(),
            actual: dimensions[0].clone(),
        });
    }
    if let TensorDimension::Static(channel) = dimensions[1] {
        if ![1, 3].contains(&channel) {
            issues.push(ImageInterfaceIssue::ChannelDimension {
                name: value.name.clone(),
                actual: dimensions[1].clone(),
            });
        }
    }
    for axis in [2, 3] {
        if dimensions[axis] == TensorDimension::Static(0) {
            issues.push(ImageInterfaceIssue::SpatialDimension {
                name: value.name.clone(),
                axis,
                actual: dimensions[axis].clone(),
            });
        }
    }
}
fn validate_image_interface(
    inputs: &[ValueMetadata],
    outputs: &[ValueMetadata],
    check_shape: bool,
) -> Result<ImageInterfaceReport, ImageInterfaceError> {
    let mut report = inspect_image_interface(inputs, outputs);
    report
        .issues
        .retain(|issue| !matches!(issue, ImageInterfaceIssue::OutputFormatUnknown { .. }));
    if !check_shape {
        report.issues.retain(|issue| {
            !matches!(
                issue,
                ImageInterfaceIssue::InputRank { .. }
                    | ImageInterfaceIssue::BatchDimension { .. }
                    | ImageInterfaceIssue::ChannelDimension { .. }
                    | ImageInterfaceIssue::SpatialDimension { .. }
            )
        });
    }
    report
        .issues
        .first()
        .cloned()
        .map_or(Ok(report), |issue| Err(ImageInterfaceError(issue)))
}
fn build_session(
    model: &[u8],
    plan: &ExecutionPlan,
    profile_prefix: Option<&std::path::Path>,
) -> Result<Session, InferenceError> {
    let builder = Session::builder().map_err(|e| runtime_error("create session builder", e))?;
    let builder = if let Some(prefix) = profile_prefix {
        builder
            .with_profiling(prefix)
            .map_err(|e| runtime_error("configure profiling", e))?
    } else {
        builder
    };
    let mut builder = match plan.configured_backend {
        BackendPreference::OrtCpu => builder
            .with_execution_providers([CPU::default().build().error_on_failure()])
            .map_err(|e| runtime_error("configure CPU", e))?,
        BackendPreference::OrtDirectMl => {
            #[cfg(target_os = "windows")]
            {
                let builder = builder
                    .with_execution_providers([ort::ep::DirectML::default()
                        .with_device_id(0)
                        .build()
                        .error_on_failure()])
                    .and_then(|builder| builder.with_parallel_execution(false))
                    .and_then(|builder| builder.with_memory_pattern(false))
                    .map_err(|e| runtime_error("configure DirectML", e))?;
                if plan.requested_policy.fallback == FallbackPolicy::Forbid {
                    builder
                        .with_config_entry("session.disable_cpu_ep_fallback", "1")
                        .map_err(|e| runtime_error("disable CPU fallback", e))?
                } else {
                    builder
                }
            }
            #[cfg(not(target_os = "windows"))]
            {
                return Err(InferenceError::ProviderUnavailable(
                    InferenceProvider::DirectMl,
                ));
            }
        }
        BackendPreference::OrtCoreMl => {
            #[cfg(target_os = "macos")]
            {
                // ALL permits CoreML to choose compatible hardware; this does not
                // assert ANE-only execution. Dynamic shapes stay enabled unless
                // the model has been proven safe for RequireStaticInputShapes.
                let core_ml = ort::ep::CoreML::default()
                    .with_static_input_shapes(false)
                    .with_compute_units(ort::ep::coreml::ComputeUnits::All)
                    .build()
                    .error_on_failure();
                if plan.requested_policy.fallback == FallbackPolicy::AllowCpu {
                    builder
                        .with_execution_providers([
                            core_ml,
                            CPU::default().build().error_on_failure(),
                        ])
                        .map_err(|e| runtime_error("configure CoreML with CPU fallback", e))?
                } else {
                    builder
                        .with_execution_providers([core_ml])
                        .and_then(|builder| {
                            builder.with_config_entry("session.disable_cpu_ep_fallback", "1")
                        })
                        .map_err(|e| runtime_error("configure CoreML", e))?
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                return Err(InferenceError::ProviderUnavailable(
                    InferenceProvider::CoreMl,
                ));
            }
        }
        BackendPreference::MetalGpu => {
            return Err(InferenceError::InvalidModelInterface("resident execution cannot use ORT host tensors".into()));
        }
        BackendPreference::Auto => {
            return Err(InferenceError::InvalidModelInterface(
                "unresolved Auto execution plan".into(),
            ))
        }
    };
    builder
        .commit_from_memory(model)
        .map_err(|e| runtime_error("load model", e))
}

#[cfg(test)]
mod tests;
