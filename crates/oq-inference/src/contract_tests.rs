use super::*;

fn metadata(name: &str, dimensions: Vec<TensorDimension>) -> TensorMetadata {
    TensorMetadata {
        name: name.into(),
        element_type: TensorElementType::Float32,
        dimensions,
    }
}
#[test]
fn policy_selection_is_device_free_explicit_and_truthful() {
    let cpu_only = InferenceCapabilities {
        cpu: true,
        direct_ml: false,
        core_ml: false,
        shared_wgpu_device: false,
    };
    let default = select_execution_plan(&ExecutionPolicy::default(), &cpu_only).unwrap();
    assert_eq!(default.configured_backend, BackendPreference::OrtCpu);
    assert_eq!(default.provider_placement, ProviderPlacement::Unknown);
    assert_eq!(default.transfer_path, TransferPath::Cpu);
    let mut policy = ExecutionPolicy {
        backend: BackendPreference::OrtDirectMl,
        ..ExecutionPolicy::default()
    };
    let fallback = select_execution_plan(&policy, &cpu_only).unwrap();
    assert_eq!(fallback.configured_providers, [InferenceProvider::Cpu]);
    assert!(fallback.fallback_reason.is_some());
    policy.fallback = FallbackPolicy::Forbid;
    assert_eq!(
        select_execution_plan(&policy, &cpu_only),
        Err(InferenceError::ProviderUnavailable(
            InferenceProvider::DirectMl
        ))
    );
    policy = ExecutionPolicy {
        residency: ResidencyPolicy::RequireResident,
        ..ExecutionPolicy::default()
    };
    assert!(matches!(
        policy.validate(),
        Err(InferenceError::UnsupportedResidency(_))
    ));
    for placement in [
        PlacementPolicy::HardwareAccelerated,
        PlacementPolicy::AcceleratorOnly,
    ] {
        policy = ExecutionPolicy {
            placement,
            ..ExecutionPolicy::default()
        };
        assert!(matches!(
            select_execution_plan(&policy, &cpu_only),
            Err(InferenceError::UnsupportedPlacement(_))
        ));
    }
    let dml = InferenceCapabilities {
        direct_ml: true,
        ..cpu_only
    };
    policy = ExecutionPolicy {
        backend: BackendPreference::Auto,
        ..ExecutionPolicy::default()
    };
    assert_eq!(
        select_execution_plan(&policy, &dml)
            .unwrap()
            .configured_backend,
        BackendPreference::OrtDirectMl
    );
    assert_eq!(
        select_execution_plan(&ExecutionPolicy::default(), &dml)
            .unwrap()
            .configured_backend,
        BackendPreference::OrtCpu
    );
}
#[test]
fn coreml_selection_reports_provider_and_cpu_fallback() {
    let policy = ExecutionPolicy {
        backend: BackendPreference::OrtCoreMl,
        ..ExecutionPolicy::default()
    };
    let unavailable = InferenceCapabilities {
        cpu: true,
        direct_ml: false,
        core_ml: false,
        shared_wgpu_device: false,
    };
    let plan = select_execution_plan(&policy, &unavailable).unwrap();
    assert_eq!(plan.configured_backend, BackendPreference::OrtCpu);
    assert_eq!(plan.configured_providers, [InferenceProvider::Cpu]);
    assert!(plan.fallback_reason.is_some());

    let forbid = ExecutionPolicy {
        fallback: FallbackPolicy::Forbid,
        ..policy
    };
    assert_eq!(
        select_execution_plan(&forbid, &unavailable),
        Err(InferenceError::ProviderUnavailable(
            InferenceProvider::CoreMl
        ))
    );

    let available = InferenceCapabilities {
        core_ml: true,
        ..unavailable
    };
    let plan = select_execution_plan(&policy, &available).unwrap();
    assert_eq!(plan.configured_backend, BackendPreference::OrtCoreMl);
    assert_eq!(
        plan.configured_providers,
        [InferenceProvider::CoreMl, InferenceProvider::Cpu]
    );
    assert_eq!(plan.provider_placement, ProviderPlacement::Unknown);
    assert_eq!(plan.transfer_path, TransferPath::Cpu);
    let auto = select_execution_plan(
        &ExecutionPolicy {
            backend: BackendPreference::Auto,
            ..ExecutionPolicy::default()
        },
        &available,
    )
    .unwrap();
    assert_eq!(auto.configured_backend, BackendPreference::OrtCoreMl);
}
#[test]
fn tensor_validation_checks_names_shape_symbols_length_and_capacity() {
    let m = metadata(
        "left",
        vec![
            TensorDimension::Symbol("batch".into()),
            TensorDimension::Static(2),
        ],
    );
    let tensor = NamedTensor {
        name: "left".into(),
        data: vec![1.0, 2.0],
        shape: vec![1, 2],
    };
    let mut symbols = BTreeMap::new();
    assert_eq!(
        tensor_bytes(&tensor, &m, &mut symbols, TensorLimits::default()).unwrap(),
        8
    );
    let mismatch = NamedTensor {
        shape: vec![2, 2],
        data: vec![0.0; 4],
        ..tensor.clone()
    };
    assert!(matches!(
        tensor_bytes(&mismatch, &m, &mut symbols, TensorLimits::default()),
        Err(InferenceError::Dimension { axis: 0, .. })
    ));
    let invalid = NamedTensor {
        shape: vec![1, 3],
        ..tensor.clone()
    };
    assert!(matches!(
        tensor_bytes(&invalid, &m, &mut BTreeMap::new(), TensorLimits::default()),
        Err(InferenceError::Dimension { axis: 1, .. })
    ));
    let invalid = NamedTensor {
        shape: vec![2],
        ..tensor.clone()
    };
    assert!(matches!(
        tensor_bytes(&invalid, &m, &mut BTreeMap::new(), TensorLimits::default()),
        Err(InferenceError::Rank { .. })
    ));
    let invalid = NamedTensor {
        data: vec![1.0],
        ..tensor.clone()
    };
    assert!(matches!(
        tensor_bytes(&invalid, &m, &mut BTreeMap::new(), TensorLimits::default()),
        Err(InferenceError::TensorLength { .. })
    ));
    assert!(matches!(
        tensor_bytes(
            &tensor,
            &m,
            &mut BTreeMap::new(),
            TensorLimits {
                max_tensor_bytes: 4,
                max_invocation_bytes: 8
            }
        ),
        Err(InferenceError::Capacity { .. })
    ));
    let invalid = NamedTensor {
        shape: vec![-1, 2],
        ..tensor.clone()
    };
    assert!(matches!(
        tensor_bytes(&invalid, &m, &mut BTreeMap::new(), TensorLimits::default()),
        Err(InferenceError::Dimension { .. })
    ));
    let pair = [m, metadata("right", vec![TensorDimension::Static(1)])];
    assert!(matches!(
        validate_names(["nope".into()].into_iter(), &pair, true),
        Err(InferenceError::UnknownName(_))
    ));
    assert!(matches!(
        validate_names(["left".into(), "left".into()].into_iter(), &pair, true),
        Err(InferenceError::DuplicateName(_))
    ));
    assert!(matches!(
        validate_names(["left".into()].into_iter(), &pair, true),
        Err(InferenceError::MissingInput(_))
    ));
}
#[test]
fn shape_product_overflow_is_an_error_not_an_allocation() {
    let m = metadata("huge", vec![TensorDimension::Dynamic; 3]);
    let tensor = NamedTensor {
        name: "huge".into(),
        data: vec![],
        shape: vec![i64::MAX; 3],
    };
    assert!(matches!(
        tensor_bytes(&tensor, &m, &mut BTreeMap::new(), TensorLimits::default()),
        Err(InferenceError::SizeOverflow(_))
    ));
}
