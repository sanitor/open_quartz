use super::*;

// Encode the small ONNX protobuf directly: two Identity nodes, two explicitly
// named FP32 inputs/outputs with a shared symbolic batch dimension. The fixture
// is independent of OpenQuartz, protoc, Python, downloads, and model packages.
fn varint(mut value: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        let part = (value & 127) as u8;
        value >>= 7;
        bytes.push(part | if value == 0 { 0 } else { 128 });
        if value == 0 {
            return bytes;
        }
    }
}
fn integer(field: u8, value: u64) -> Vec<u8> {
    let mut out = varint((field as u64) << 3);
    out.extend(varint(value));
    out
}
fn message(field: u8, bytes: &[u8]) -> Vec<u8> {
    let mut out = varint(((field as u64) << 3) | 2);
    out.extend(varint(bytes.len() as u64));
    out.extend(bytes);
    out
}
fn value_info(name: &str, dimensions: &[Option<u64>], element_type: u64) -> Vec<u8> {
    let mut shape = Vec::new();
    for dimension in dimensions {
        let dim = match dimension {
            Some(n) => integer(1, *n),
            None => message(2, b"batch"),
        };
        shape.extend(message(1, &dim));
    }
    let tensor_type = [integer(1, element_type), message(2, &shape)].concat();
    [
        message(1, name.as_bytes()),
        message(2, &message(1, &tensor_type)),
    ]
    .concat()
}
fn identity_model(pairs: &[(&str, &str)], dimensions: &[Option<u64>]) -> Vec<u8> {
    identity_model_with_type(pairs, dimensions, 1)
}
fn identity_model_with_type(
    pairs: &[(&str, &str)],
    dimensions: &[Option<u64>],
    element_type: u64,
) -> Vec<u8> {
    let mut graph = message(2, b"standalone-identities");
    for (input, output) in pairs {
        let node = [
            message(1, input.as_bytes()),
            message(2, output.as_bytes()),
            message(4, b"Identity"),
        ]
        .concat();
        graph.extend(message(1, &node));
        graph.extend(message(11, &value_info(input, dimensions, element_type)));
        graph.extend(message(12, &value_info(output, dimensions, element_type)));
    }
    [
        integer(1, 8),
        message(2, b"oq-inference-tests"),
        message(7, &graph),
        message(8, &integer(2, 13)),
    ]
    .concat()
}
fn identity_model_with_model_metadata() -> Vec<u8> {
    let mut model = identity_model(
        &[("image", "result")],
        &[Some(1), Some(3), Some(2), Some(2)],
    );
    model.extend(message(2, b"metadata-producer"));
    model.extend(message(3, b"producer-version"));
    model.extend(message(4, b"example.domain"));
    model.extend(integer(5, 7));
    model.extend(message(6, b"model description"));
    let property = [message(1, b"fixture.key"), message(2, b"fixture.value")].concat();
    model.extend(message(14, &property));
    model
}
fn identity_model_with_io_dimensions(
    input_dimensions: &[Option<u64>],
    output_dimensions: &[Option<u64>],
) -> Vec<u8> {
    let input = "image";
    let output = "result";
    let node = [
        message(1, input.as_bytes()),
        message(2, output.as_bytes()),
        message(4, b"Identity"),
    ]
    .concat();
    let graph = [
        message(2, b"io-shape"),
        message(1, &node),
        message(11, &value_info(input, input_dimensions, 1)),
        message(12, &value_info(output, output_dimensions, 1)),
    ]
    .concat();
    [
        integer(1, 8),
        message(2, b"oq-inference-tests"),
        message(7, &graph),
        message(8, &integer(2, 13)),
    ]
    .concat()
}
fn configure_runtime() {
    if std::env::var_os("ORT_DYLIB_PATH").is_some() {
        return;
    }
    let architecture = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    };
    let (platform, library) = if cfg!(target_os = "windows") {
        ("win32", "onnxruntime.dll")
    } else if cfg!(target_os = "macos") {
        ("darwin", "libonnxruntime.1.21.0.dylib")
    } else {
        ("linux", "libonnxruntime.so.1.21.0")
    };
    let runtime = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../node_modules/.cache/open-quartz")
        .join(format!("desktop-runtime-{platform}-{architecture}"))
        .join(library);
    assert!(
        runtime.is_file(),
        "Provide ORT_DYLIB_PATH to ONNX Runtime >=1.21: {}",
        runtime.display()
    );
    std::env::set_var("ORT_DYLIB_PATH", runtime);
}
#[test]
fn model_facts_include_custom_metadata_and_unavailable_opsets() {
    configure_runtime();
    let session = InferenceSession::from_memory(&identity_model_with_model_metadata()).unwrap();
    assert_eq!(
        session.info().model.producer_name.as_deref(),
        Some("metadata-producer")
    );
    assert_eq!(
        session.info().model.name.as_deref(),
        Some("standalone-identities")
    );
    assert_eq!(
        session.info().model.domain.as_deref(),
        Some("example.domain")
    );
    assert_eq!(session.info().model.version, Some(7));
    assert_eq!(
        session.info().model.description.as_deref(),
        Some("model description")
    );
    assert_eq!(
        session
            .info()
            .model
            .custom_metadata
            .get("fixture.key")
            .map(String::as_str),
        Some("fixture.value")
    );
    assert_eq!(
        session.info().model.producer_version.as_deref(),
        Some("producer-version")
    );
    assert_eq!(
        session.info().opset_imports.as_ref().unwrap(),
        &vec![OpsetImport {
            domain: String::new(),
            version: 13
        }]
    );
    assert!(session.info().opset_unavailable_reason.is_none());
    assert_eq!(
        session.info().source_interface.as_ref().unwrap().ir_version,
        Some(8)
    );
    assert!(matches!(
        session.info().input_values[0].kind,
        ValueKind::Tensor {
            element_type: TensorElementType::Float32,
            ..
        }
    ));
    assert!(session.info().overridable_initializers.is_empty());
    assert!(!session.info().image_interface.runnable);
}
#[test]
fn independent_real_cpu_named_two_input_two_output_and_identity() {
    configure_runtime();
    let bytes = identity_model(
        &[("left", "left_result"), ("right", "right_result")],
        &[None, Some(2)],
    );
    let mut session = InferenceSession::from_memory(&bytes).unwrap();
    assert_eq!(session.info().input_names, ["left", "right"]);
    assert_eq!(session.info().output_names, ["left_result", "right_result"]);
    assert_eq!(
        session.info().inputs[0].dimensions,
        [
            TensorDimension::Symbol("batch".into()),
            TensorDimension::Static(2)
        ]
    );
    assert_eq!(session.plan().configured_backend, BackendPreference::OrtCpu);
    assert_eq!(
        session.plan().provider_placement,
        ProviderPlacement::Unknown
    );
    assert_eq!(session.info().artifact.sha256.len(), 64);
    assert_eq!(session.info().artifact.byte_length, bytes.len() as u64);
    assert!(session.validate_single_image().is_err());
    assert!(session.run_f32(vec![1.0, 2.0], vec![1, 2]).is_err());
    let stamp = RequestStamp {
        request_id: 19,
        session_generation: session.generation(),
    };
    let inputs = vec![
        NamedTensor {
            name: "right".into(),
            data: vec![7.0, 8.0],
            shape: vec![1, 2],
        },
        NamedTensor {
            name: "left".into(),
            data: vec![1.0, 2.0],
            shape: vec![1, 2],
        },
    ];
    let names = vec!["right_result".into(), "left_result".into()];
    let result = session.invoke(stamp, inputs.clone(), &names).unwrap();
    assert_eq!(result.stamp, stamp);
    assert_eq!(result.outputs[0].name, "right_result");
    assert_eq!(result.outputs[0].data, [7.0, 8.0]);
    assert_eq!(result.outputs[1].data, [1.0, 2.0]);
    assert_eq!(result.outputs[0].shape, [1, 2]);
    assert_eq!(result.metrics.input_cpu_bytes, 16);
    assert_eq!(result.metrics.output_cpu_bytes, 16);
    assert!(result.metrics.total_duration >= result.metrics.runtime_duration);
    assert!(result.metrics.runtime_duration > Duration::ZERO);
    let result = session
        .invoke(stamp, inputs.clone(), &["left_result".into()])
        .unwrap();
    assert_eq!(result.outputs.len(), 1);
    assert_eq!(result.outputs[0].data, [1.0, 2.0]);
    assert!(matches!(
        session.invoke(stamp, inputs.clone(), &["missing".into()]),
        Err(InferenceError::UnknownName(_))
    ));
    assert!(matches!(
        session.invoke(
            stamp,
            inputs.clone(),
            &["left_result".into(), "left_result".into()]
        ),
        Err(InferenceError::DuplicateName(_))
    ));
    assert!(matches!(
        session.invoke(stamp, inputs.clone(), &[]),
        Err(InferenceError::EmptyOutputs)
    ));
    let mut invalid = inputs.clone();
    invalid[0].name = "missing".into();
    assert!(matches!(
        session.invoke(stamp, invalid, &names),
        Err(InferenceError::UnknownName(_))
    ));
    let mut invalid = inputs.clone();
    invalid[0].shape = vec![1, 3];
    assert!(matches!(
        session.invoke(stamp, invalid, &names),
        Err(InferenceError::Dimension { .. })
    ));
    let mut invalid = inputs.clone();
    invalid[0].shape = vec![2, 2];
    invalid[0].data = vec![0.0; 4];
    assert!(matches!(
        session.invoke(stamp, invalid, &names),
        Err(InferenceError::Dimension { .. })
    ));
    let mut invalid = inputs.clone();
    invalid[0].data.clear();
    assert!(matches!(
        session.invoke(stamp, invalid, &names),
        Err(InferenceError::TensorLength { .. })
    ));
    assert!(matches!(
        session.invoke_with_limits(
            stamp,
            inputs.clone(),
            &names,
            TensorLimits {
                max_tensor_bytes: 4,
                max_invocation_bytes: 32
            }
        ),
        Err(InferenceError::Capacity { .. })
    ));
    let stale = RequestStamp {
        session_generation: SessionGeneration(stamp.session_generation.0 + 1),
        ..stamp
    };
    assert!(matches!(
        session.invoke(stale, inputs, &names),
        Err(InferenceError::StaleSession { .. })
    ));
    let mut identity =
        InferenceSession::from_memory(&identity_model(&[("input", "output")], &[Some(1)])).unwrap();
    let output = identity.run_f32(vec![42.0], vec![1]).unwrap();
    assert_eq!(
        output,
        TensorOutput {
            data: vec![42.0],
            shape: vec![1]
        }
    );
    assert_ne!(identity.generation(), session.generation());
    assert!(identity.validate_image_shape().is_err());
    let image = InferenceSession::from_memory(&identity_model(
        &[("image", "result")],
        &[Some(1), Some(3), Some(2), Some(2)],
    ))
    .unwrap();
    image.validate_image_shape().unwrap();
    assert!(!image.info().image_interface.runnable);
    assert!(image
        .info()
        .image_interface
        .output_format
        .contains("unknown"));
    let detection = InferenceSession::from_memory(&identity_model_with_io_dimensions(
        &[Some(1), Some(3), Some(224), Some(224)],
        &[Some(1), Some(84), Some(1792)],
    ))
    .unwrap();
    assert!(detection.validate_image_shape().is_ok());
    assert!(matches!(
        detection.info().output_values[0].kind,
        ValueKind::Tensor {
            element_type: TensorElementType::Float32,
            ..
        }
    ));
    assert!(detection
        .info()
        .image_interface
        .output_format
        .contains("unknown"));
    let invalid_batch = InferenceSession::from_memory(&identity_model(
        &[("image", "result")],
        &[Some(2), Some(3), Some(2), Some(2)],
    ))
    .unwrap();
    assert!(matches!(
        invalid_batch.validate_image_shape(),
        Err(ImageInterfaceError(
            ImageInterfaceIssue::BatchDimension { .. }
        ))
    ));
    let invalid_channels = InferenceSession::from_memory(&identity_model(
        &[("image", "result")],
        &[Some(1), Some(2), Some(2), Some(2)],
    ))
    .unwrap();
    assert!(matches!(
        invalid_channels.validate_image_shape(),
        Err(ImageInterfaceError(
            ImageInterfaceIssue::ChannelDimension { .. }
        ))
    ));
    assert!(InferenceSession::from_memory(b"not a model").is_err());
    let mut integer_session = InferenceSession::from_memory(&identity_model_with_type(
        &[("i64", "result")],
        &[Some(1)],
        7,
    ))
    .unwrap();
    assert_eq!(
        integer_session.info().inputs[0].element_type,
        TensorElementType::Int64
    );
    assert!(matches!(
        integer_session.run_f32(vec![1.0], vec![1]),
        Err(InferenceError::ImageInterface(ImageInterfaceError(
            ImageInterfaceIssue::UnsupportedElementType { .. }
        )))
    ));
}
#[test]
fn strict_policy_is_refused_before_runtime_loading() {
    let policy = ExecutionPolicy {
        residency: ResidencyPolicy::RequireResident,
        ..ExecutionPolicy::default()
    };
    assert!(matches!(
        InferenceSession::from_memory_with_policy(b"unused", &policy),
        Err(InferenceError::UnsupportedResidency(_))
    ));
}
#[cfg(target_os = "macos")]
#[test]
fn real_coreml_identity_when_runtime_exposes_provider() {
    configure_runtime();
    let capabilities = inference_capabilities().unwrap();
    if !capabilities.core_ml {
        return;
    }
    let model = identity_model(&[("input", "output")], &[Some(1), Some(1)]);
    let policy = ExecutionPolicy {
        backend: BackendPreference::OrtCoreMl,
        fallback: FallbackPolicy::AllowCpu,
        ..ExecutionPolicy::default()
    };
    let mut session = InferenceSession::from_memory_with_policy(&model, &policy).unwrap();
    assert_eq!(
        session.plan().configured_backend,
        BackendPreference::OrtCoreMl
    );
    assert_eq!(
        session.plan().configured_providers,
        [InferenceProvider::CoreMl, InferenceProvider::Cpu]
    );
    assert_eq!(
        session.plan().provider_placement,
        ProviderPlacement::Unknown
    );
    assert_eq!(session.plan().transfer_path, TransferPath::Cpu);
    assert_eq!(session.run_f32(vec![7.0], vec![1, 1]).unwrap().data, [7.0]);
}
