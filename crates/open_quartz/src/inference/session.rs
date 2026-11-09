//! Core image tasks use the independent inference implementation directly.
pub use oq_inference::{
    inference_capabilities, select_execution_plan, BackendPreference, ExecutionPlan,
    ExecutionPolicy, FallbackPolicy, InferenceCapabilities, InferenceError,
    InferenceOptions, InferenceProvider, InferenceSessionInfo, InvocationMetrics,
    InvocationResult, ModelArtifact, NamedTensor, PlacementPolicy, ProviderPlacement,
    RequestStamp, ResidencyPolicy, SessionGeneration, TensorDimension,
    TensorElementType, TensorLimits, TensorMetadata, TensorOutput, TransferPath,
    ValueMetadata, ValueKind, ValueOptionality, ModelMetadataInfo, OpsetImport,
    ImageInterfaceReport, ImageInterfaceIssue, ImageInterfaceError,
};
#[cfg(all(feature = "native-onnx", not(target_arch = "wasm32")))]
pub use oq_inference::InferenceSession;

#[cfg(all(test, feature = "native-onnx", not(target_arch = "wasm32")))]
#[path = "session_coverage_tests.rs"]
mod coverage_tests;
