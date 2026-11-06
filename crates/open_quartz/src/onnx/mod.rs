#[cfg(all(feature = "native-onnx", not(target_arch = "wasm32")))]
pub mod pipeline;
pub mod postprocess;
pub mod preprocess;
#[cfg(all(feature = "native-onnx", not(target_arch = "wasm32")))]
pub mod session;
pub mod task;
pub mod types;
#[cfg(all(feature = "native-onnx", not(target_arch = "wasm32")))]
mod prepared;
#[cfg(all(feature = "native-onnx", not(target_arch = "wasm32")))]
pub use prepared::PreparedOnnxModel;

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
pub(crate) use session::TensorRunner;
#[cfg(all(feature = "native-onnx", not(target_arch = "wasm32")))]
pub use session::{
    NativeOnnxCapabilities, NativeOnnxOptions, NativeOnnxProvider, OnnxSessionInfo, TensorOutput,
};
pub use task::{
    build_browser_onnx_completion, decode_browser_onnx_output, encode_browser_onnx_input,
    plan_browser_onnx_task, BrowserOnnxCompletionPort, BrowserOnnxCompletionRequest,
    BrowserOnnxDecodedOutput, BrowserOnnxOutputRequest, BrowserOnnxTaskPlan,
    BrowserOnnxTaskPlanRequest, BrowserOnnxTensor, BrowserOnnxTensorRequest, OnnxExecutionPlan,
    OnnxModelFamily, TensorDescriptor, ThresholdDescriptor, TileDescriptor,
};
pub use types::{NativeOnnxConfig, OnnxTask};

#[cfg(all(feature = "native-onnx", not(target_arch = "wasm32")))]
pub use pipeline::{run_native_image_task, NativeOnnxImageOutput};

#[cfg(all(feature = "native-onnx", not(target_arch = "wasm32")))]
pub use session::native_onnx_capabilities;
#[cfg(all(feature = "native-onnx", not(target_arch = "wasm32")))]
pub use session::OnnxSession;
