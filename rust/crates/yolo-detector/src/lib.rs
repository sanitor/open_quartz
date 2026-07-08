//! OpenQuartz wrapper around the upstream `rimeflow-yolov8n` crate.
//!
//! Upstream: https://github.com/caozisheng/rimeflow-yolov8n
//!
//! Upstream ships the pure Rust `postprocess` module (decode + NMS) and the
//! `wasm_bindgen(inline_js)` `ort_bridge` module (auto-loads onnxruntime-web
//! from `/ort/ort.min.js`, drives WebGPU / WASM EP, GPU zero-copy path reserved).
//!
//! This crate re-exports upstream verbatim through the free-function API and
//! adds a `YoloDetectorWasm` facade + `class_names()` helper so the OpenQuartz
//! TypeScript layer can consume a stable, typed handle. Nothing here duplicates
//! upstream: postprocess and inline_js bridge live upstream only.

pub use rimeflow_yolov8n::postprocess;
pub use rimeflow_yolov8n::MODEL_URL;

#[cfg(target_arch = "wasm32")]
pub use rimeflow_yolov8n::ort_bridge;

pub const DEFAULT_TARGET_SIZE: u32 = 640;

/// COCO 80 class names in canonical YOLOv8 order. Upstream postprocess emits
/// `class_id: u32`; naming is a UI concern owned here.
pub const COCO_CLASSES: [&str; 80] = [
    "person", "bicycle", "car", "motorcycle", "airplane", "bus", "train", "truck",
    "boat", "traffic light", "fire hydrant", "stop sign", "parking meter", "bench",
    "bird", "cat", "dog", "horse", "sheep", "cow", "elephant", "bear", "zebra",
    "giraffe", "backpack", "umbrella", "handbag", "tie", "suitcase", "frisbee",
    "skis", "snowboard", "sports ball", "kite", "baseball bat", "baseball glove",
    "skateboard", "surfboard", "tennis racket", "bottle", "wine glass", "cup",
    "fork", "knife", "spoon", "bowl", "banana", "apple", "sandwich", "orange",
    "broccoli", "carrot", "hot dog", "pizza", "donut", "cake", "chair", "couch",
    "potted plant", "bed", "dining table", "toilet", "tv", "laptop", "mouse",
    "remote", "keyboard", "cell phone", "microwave", "oven", "toaster", "sink",
    "refrigerator", "book", "clock", "vase", "scissors", "teddy bear", "hair drier",
    "toothbrush",
];

#[cfg(target_arch = "wasm32")]
mod wasm_facade {
    use super::{postprocess, COCO_CLASSES, DEFAULT_TARGET_SIZE, MODEL_URL};
    use js_sys::Reflect;
    use serde::Serialize;
    use wasm_bindgen::prelude::*;

    /// One detection as delivered to TypeScript.
    #[derive(Debug, Clone, Serialize)]
    pub struct DetectionJs {
        pub bbox: [f32; 4],
        pub score: f32,
        pub class_id: u32,
        pub class_name: &'static str,
    }

    impl From<postprocess::Detection> for DetectionJs {
        fn from(d: postprocess::Detection) -> Self {
            let class_name = COCO_CLASSES.get(d.class_id as usize).copied().unwrap_or("unknown");
            Self { bbox: d.bbox, score: d.score, class_id: d.class_id, class_name }
        }
    }

    /// Stable façade for the OpenQuartz TS side. Wraps upstream's free-function
    /// `ort_bridge` API into a session-shaped handle so the engine can hold and
    /// release detectors per model.
    ///
    /// Upstream `ort_detect` returns `{ output, scale, padX, padY, srcW, srcH }`
    /// where `output` is the raw YOLOv8 tensor. Decode + NMS run here.
    #[wasm_bindgen]
    pub struct YoloDetectorWasm {
        model_url: String,
        // Target size is fixed to upstream's 640 today; kept as a field so the TS
        // constructor signature stays stable when upstream exposes a knob.
        target_size: u32,
        score_threshold: f32,
        iou_threshold: f32,
        initialized: bool,
    }

    #[wasm_bindgen]
    impl YoloDetectorWasm {
        #[wasm_bindgen(constructor)]
        pub fn new(model_url: Option<String>, target_size: Option<u32>) -> Self {
            Self {
                model_url: model_url.unwrap_or_else(|| MODEL_URL.to_string()),
                target_size: target_size.unwrap_or(DEFAULT_TARGET_SIZE),
                score_threshold: 0.25,
                iou_threshold: 0.45,
                initialized: false,
            }
        }

        #[wasm_bindgen(getter)]
        pub fn initialized(&self) -> bool { self.initialized }

        #[wasm_bindgen(js_name = setThresholds)]
        pub fn set_thresholds(&mut self, score: f32, iou: f32) {
            self.score_threshold = score.clamp(0.0, 1.0);
            self.iou_threshold = iou.clamp(0.0, 1.0);
        }

        /// Reserved for future WebGPURenderer integration — upstream's
        /// `capture_webgpu_device` monkey-patches `GPUAdapter.prototype.requestDevice`
        /// so wgpu and ORT share one device. OpenQuartz's Three.js WebGL renderer
        /// has no device to intercept today; call this before any WebGPU init
        /// once the renderer swap lands.
        #[wasm_bindgen(js_name = captureWebgpuDevice)]
        pub fn capture_webgpu_device() {
            super::ort_bridge::capture_webgpu_device();
        }

        pub async fn init(&mut self) -> Result<(), JsValue> {
            if self.initialized { return Ok(()); }
            super::ort_bridge::ort_init(&self.model_url).await?;
            self.initialized = true;
            Ok(())
        }

        /// Run one detection. `canvas` accepts both `HTMLCanvasElement` and
        /// `OffscreenCanvas` at the JS side because upstream's inline_js uses
        /// `canvas.width/height` and passes the value to
        /// `copyExternalImageToTexture` / `drawImage`.
        #[wasm_bindgen(js_name = detect)]
        pub async fn detect(
            &self,
            canvas: JsValue,
            src_w: u32,
            src_h: u32,
        ) -> Result<JsValue, JsValue> {
            if !self.initialized {
                return Err(JsValue::from_str("YoloDetectorWasm not initialized"));
            }
            // web_sys::HtmlCanvasElement is the strict type upstream signs on,
            // but the underlying JS only reads `.width/.height` and passes the
            // value verbatim. Unchecked-cast so we can hand OffscreenCanvas
            // through without a runtime type check.
            let canvas_ref: web_sys::HtmlCanvasElement = canvas.unchecked_into();
            let result = super::ort_bridge::ort_detect(&canvas_ref).await?;

            let raw = super::ort_bridge::get_output_f32(&result, "output")
                .ok_or_else(|| JsValue::from_str("missing `output`"))?;
            let scale = super::ort_bridge::get_f64(&result, "scale") as f32;
            let pad_x = super::ort_bridge::get_f64(&result, "padX") as f32;
            let pad_y = super::ort_bridge::get_f64(&result, "padY") as f32;

            // Upstream's decode_yolo_output has a fixed 0.25 score gate. Run NMS
            // with our IoU threshold; then apply our score threshold on the way
            // out so the TS side controls both knobs.
            let raw_dets = postprocess::decode_yolo_output(&raw, src_w, src_h, scale, pad_x, pad_y);
            let filtered = postprocess::nms(&raw_dets, self.iou_threshold);
            let score = self.score_threshold;
            let js_dets: Vec<DetectionJs> = filtered
                .into_iter()
                .filter(|d| d.score >= score)
                .map(DetectionJs::from)
                .collect();

            let out = js_sys::Object::new();
            let ser = serde_wasm_bindgen::to_value(&js_dets)
                .map_err(|e| JsValue::from_str(&format!("serialize: {e}")))?;
            Reflect::set(&out, &"detections".into(), &ser)?;
            Ok(out.into())
        }

        pub fn release(&mut self) {
            super::ort_bridge::ort_release();
            self.initialized = false;
        }

        // Unused today but keeps the field live and lets the TS side introspect
        // the model input size chosen at construction.
        #[wasm_bindgen(getter, js_name = targetSize)]
        pub fn target_size(&self) -> u32 { self.target_size }
    }

    #[wasm_bindgen(js_name = defaultModelUrl)]
    pub fn default_model_url() -> String { MODEL_URL.to_string() }

    #[wasm_bindgen(js_name = defaultTargetSize)]
    pub fn default_target_size() -> u32 { DEFAULT_TARGET_SIZE }

    #[wasm_bindgen(js_name = classNames)]
    pub fn class_names() -> JsValue {
        serde_wasm_bindgen::to_value(&COCO_CLASSES[..]).unwrap_or(JsValue::NULL)
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm_facade::*;
