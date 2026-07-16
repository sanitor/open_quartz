pub use rimeflow_yolo26n_sem::postprocess;
pub use rimeflow_yolo26n_sem::MODEL_URL;

#[cfg(target_arch = "wasm32")]
pub use rimeflow_yolo26n_sem::ort_bridge;

pub const DEFAULT_TARGET_SIZE: u32 = 640;

pub const CITYSCAPES_CLASSES: [&str; 19] = [
    "road", "sidewalk", "building", "wall", "fence",
    "pole", "traffic light", "traffic sign", "vegetation", "terrain",
    "sky", "person", "rider", "car", "truck",
    "bus", "train", "motorcycle", "bicycle",
];

#[cfg(target_arch = "wasm32")]
mod wasm_facade {
    use super::{postprocess, CITYSCAPES_CLASSES, DEFAULT_TARGET_SIZE, MODEL_URL};
    use js_sys::Reflect;
    use serde::Serialize;
    use wasm_bindgen::prelude::*;

    #[derive(Debug, Clone, Serialize)]
    pub struct SegmentationJs {
        pub mask_rgba: Vec<u8>,
        pub mask_w: u32,
        pub mask_h: u32,
        pub class_counts: Vec<u32>,
        pub num_classes: u32,
    }

    #[wasm_bindgen]
    pub struct YoloSemWasm {
        model_url: String,
        target_size: u32,
        initialized: bool,
    }

    #[wasm_bindgen]
    impl YoloSemWasm {
        #[wasm_bindgen(constructor)]
        pub fn new(model_url: Option<String>, target_size: Option<u32>) -> Self {
            Self {
                model_url: model_url.unwrap_or_else(|| MODEL_URL.to_string()),
                target_size: target_size.unwrap_or(DEFAULT_TARGET_SIZE),
                initialized: false,
            }
        }

        #[wasm_bindgen(getter)]
        pub fn initialized(&self) -> bool { self.initialized }

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

        #[wasm_bindgen(js_name = segment)]
        pub async fn segment(
            &self,
            canvas: JsValue,
            src_w: u32,
            src_h: u32,
        ) -> Result<JsValue, JsValue> {
            if !self.initialized {
                return Err(JsValue::from_str("YoloSemWasm not initialized"));
            }
            let canvas_ref: web_sys::HtmlCanvasElement = canvas.unchecked_into();
            let result = super::ort_bridge::ort_detect(&canvas_ref).await?;

            let raw = super::ort_bridge::get_output_f32(&result, "output")
                .ok_or_else(|| JsValue::from_str("missing `output`"))?;
            let scale = super::ort_bridge::get_f64(&result, "scale") as f32;
            let pad_x = super::ort_bridge::get_f64(&result, "padX") as f32;
            let pad_y = super::ort_bridge::get_f64(&result, "padY") as f32;

            let seg = postprocess::decode_segmentation_output(
                &raw, src_w, src_h, scale, pad_x, pad_y,
            );

            let resized = postprocess::resize_mask_nearest(
                &seg.mask, seg.mask_w, seg.mask_h, src_w, src_h,
            );

            let mask_rgba = postprocess::mask_to_rgba(
                &resized, src_w, src_h, &postprocess::CITYSCAPES_PALETTE,
            );

            let num_classes = seg.num_classes as usize;
            let mut class_counts = vec![0u32; num_classes];
            for &c in &resized {
                if (c as usize) < num_classes {
                    class_counts[c as usize] += 1;
                }
            }

            let js_seg = SegmentationJs {
                mask_rgba,
                mask_w: src_w,
                mask_h: src_h,
                class_counts,
                num_classes: seg.num_classes,
            };

            let out = js_sys::Object::new();
            let ser = serde_wasm_bindgen::to_value(&js_seg)
                .map_err(|e| JsValue::from_str(&format!("serialize: {e}")))?;
            Reflect::set(&out, &"segmentation".into(), &ser)?;
            Ok(out.into())
        }

        pub fn release(&mut self) {
            super::ort_bridge::ort_release();
            self.initialized = false;
        }

        #[wasm_bindgen(getter, js_name = targetSize)]
        pub fn target_size(&self) -> u32 { self.target_size }
    }

    #[wasm_bindgen(js_name = defaultModelUrl)]
    pub fn default_model_url() -> String { MODEL_URL.to_string() }

    #[wasm_bindgen(js_name = defaultTargetSize)]
    pub fn default_target_size() -> u32 { DEFAULT_TARGET_SIZE }

    #[wasm_bindgen(js_name = classNames)]
    pub fn class_names() -> JsValue {
        serde_wasm_bindgen::to_value(&CITYSCAPES_CLASSES[..]).unwrap_or(JsValue::NULL)
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm_facade::*;
