use super::*;

#[test]
fn compatibility_options_default_to_cpu_and_unsupported_policy_is_early() {
    assert_eq!(InferenceOptions::default().provider, InferenceProvider::Cpu);
    let policy = ExecutionPolicy { residency: ResidencyPolicy::RequireResident, ..ExecutionPolicy::default() };
    assert!(matches!(InferenceSession::from_memory_with_policy(b"unused", &policy), Err(InferenceError::UnsupportedResidency(_))));
    let cpu_only = InferenceCapabilities { cpu: true, direct_ml: false, core_ml: false, shared_wgpu_device: false };
    let direct_ml = ExecutionPolicy { backend: BackendPreference::OrtDirectMl, fallback: FallbackPolicy::Forbid, ..ExecutionPolicy::default() };
    assert!(matches!(select_execution_plan(&direct_ml, &cpu_only), Err(InferenceError::ProviderUnavailable(InferenceProvider::DirectMl))));
}

#[test]
fn core_reexports_are_the_independent_types_without_runtime_shells() {
    fn accepts_core(_: TensorOutput) {}
    accepts_core(oq_inference::TensorOutput { data: vec![1.0], shape: vec![1] });
    let policy: ExecutionPolicy = oq_inference::ExecutionPolicy::default();
    assert_eq!(policy.backend, BackendPreference::OrtCpu);
}
