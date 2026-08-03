pub mod pipeline;
pub mod postprocess;
pub mod preprocess;
pub mod session;
pub mod types;

pub use postprocess::{
    decode_segmentation_output, decode_yolo_output, detect_postprocess, iou, mask_to_rgba, nms,
    resize_mask_nearest, segment_postprocess, Detection, SegmentationMask, SegmentationResult,
    CITYSCAPES_CLASSES, CITYSCAPES_PALETTE, COCO_CLASSES,
};
pub use preprocess::{
    apply_alpha_mask, letterbox_preprocess, midas_preprocess, rgb_output_to_rgba, rgba_to_chw,
    LetterboxTensor,
};
pub use session::{
    NativeOnnxCapabilities, NativeOnnxOptions, NativeOnnxProvider, OnnxSessionInfo, TensorOutput,
};
pub use types::OnnxTask;

#[cfg(not(target_arch = "wasm32"))]
pub use pipeline::{run_native_image_task, NativeOnnxImageOutput};

#[cfg(not(target_arch = "wasm32"))]
pub use session::native_onnx_capabilities;
#[cfg(not(target_arch = "wasm32"))]
pub use session::OnnxSession;
