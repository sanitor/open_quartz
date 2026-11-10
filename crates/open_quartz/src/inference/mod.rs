#[cfg(all(feature = "native-onnx", not(target_arch = "wasm32")))]
pub mod pipeline;
pub mod postprocess;
pub mod preprocess;
#[cfg(all(feature = "native-onnx", not(target_arch = "wasm32")))]
pub mod session;
pub mod types;
#[cfg(all(any(feature = "native-runtime", all(test, feature = "native-onnx")), not(target_arch = "wasm32")))]
pub(crate) mod tasks;
#[cfg(all(any(feature = "native-runtime", feature = "native-onnx"), not(target_arch = "wasm32")))]
mod contract;
#[cfg(all(feature = "native-runtime", not(target_arch = "wasm32")))]
pub(crate) mod prepared;
#[cfg(all(feature = "native-runtime", not(target_arch = "wasm32")))]
pub use prepared::{PreparedInferenceModel, PreparedModel, ResidentInferenceSession};
#[cfg(all(test, feature = "native-runtime", not(target_arch = "wasm32")))]
pub(crate) mod resident_test_model;
#[cfg(all(test, feature = "native-onnx", not(target_arch = "wasm32")))]
mod cpu_reference_coverage_tests;
#[cfg(all(any(feature = "native-runtime", feature = "native-onnx"), not(target_arch = "wasm32")))]
pub use contract::{image_task_contract, ImageTaskContract, ImageTaskAdapter,
    ImageTaskPreprocessing, ImageTaskResize, ImageDimensionConstraint};

pub use postprocess::{
    decode_segmentation_output, decode_yolo_output, detect_postprocess, iou, mask_to_rgba, nms,
    resize_mask_nearest, segment_postprocess, Detection, SegmentationMask, SegmentationResult,
    CITYSCAPES_CLASSES, CITYSCAPES_PALETTE, COCO_CLASSES,
};
pub use preprocess::{
    apply_alpha_mask, letterbox_preprocess, midas_preprocess, rgb_output_to_rgba, rgba_to_chw,
    LetterboxTensor,
};
#[cfg(all(feature = "native-onnx", not(target_arch = "wasm32")))]
pub use session::{
    InferenceSession, InferenceOptions, InferenceProvider,
    InferenceCapabilities, TensorOutput, NamedTensor, InvocationResult, InvocationMetrics,
    InferenceError, inference_capabilities,
};
pub use oq_inference::{
    ExecutionPolicy, BackendPreference, FallbackPolicy, ResidencyPolicy, PlacementPolicy,
    InferenceSessionInfo,
    ExecutionPlan, RequestStamp, SessionGeneration,
    TensorMetadata, TensorElementType, TensorDimension, ValueMetadata, ValueKind,
    ValueOptionality, ModelMetadataInfo, OpsetImport, ImageInterfaceReport,
    ImageInterfaceIssue, ImageInterfaceError,
    inspect_onnx, ArtifactInterface, ArtifactError,
};
pub use types::{InferenceConfig, InferenceTask};

#[cfg(all(feature = "native-onnx", not(target_arch = "wasm32")))]
pub use pipeline::{
    run_image_task, run_image_task_with_request, run_image_task_with_request_and_metrics,
    validate_image_session, InferenceImageOutput, InferenceImageMetrics,
    validate_image_task_session,
};
