#[cfg(target_arch = "wasm32")]
mod browser_player;
mod sdk;

#[cfg(target_arch = "wasm32")]
pub use browser_player::BrowserPlayerBinding;
pub use sdk::{GraphBinding, OpenQuartzBinding, PlayerBinding, ProjectBinding};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

pub const SDK_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub fn hello() -> String {
    "OpenQuartz Rust SDK".to_owned()
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = sdkVersion))]
pub fn sdk_version() -> String {
    SDK_VERSION.to_owned()
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = runtimeContract))]
pub fn runtime_contract_json() -> String {
    serde_json::to_string(&open_quartz_execution::runtime::public_surface_manifest())
        .expect("Runtime public surface manifest is always serializable")
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = catalog))]
pub fn catalog_json() -> String {
    serde_json::to_string(&open_quartz_execution::catalog::catalog_snapshot())
        .expect("Catalog snapshot is always serializable")
}

/// Parse WGSL and return the TypeScript-compatible ParsedShader JSON payload.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = parseShader))]
pub fn parse_shader_json(code: &str) -> String {
    serde_json::to_string(&open_quartz_execution::wgsl::parse_shader(code))
        .expect("ParsedShader is always JSON serializable")
}

/// Build a graph execution plan from TypeScript-compatible graph JSON.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = planGraph))]
pub fn plan_graph_json(graph_json: &str) -> Result<String, String> {
    let request: open_quartz_execution::graph::GraphRequest =
        serde_json::from_str(graph_json).map_err(|error| format!("Invalid graph JSON: {error}"))?;
    serde_json::to_string(&open_quartz_execution::graph::plan_graph(request))
        .map_err(|error| format!("Cannot serialize graph plan: {error}"))
}

/// Plan host-owned resource lifecycle operations without moving platform handles over FFI.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = planHostResourceIntents))]
pub fn plan_host_resource_intents_json(request_json: &str) -> Result<String, String> {
    let request: open_quartz_host_api::HostResourceIntentRequest =
        serde_json::from_str(request_json).map_err(|error| format!("Invalid host resource intent request: {error}"))?;
    let plan = open_quartz_host_api::plan_host_resource_intents(request)?;
    serde_json::to_string(&plan).map_err(|error| format!("Cannot serialize host resource intent plan: {error}"))
}

/// Validate GPU resource dimensions without crossing GPU objects through FFI.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = validateGpuTexture))]
pub fn validate_gpu_texture(
    width: u32,
    height: u32,
    rgba_byte_length: usize,
) -> Result<(), String> {
    if width == 0 || height == 0 {
        return Err("GPU texture dimensions must be positive".to_owned());
    }
    let expected = width as usize * height as usize * 4;
    if rgba_byte_length != expected {
        return Err(format!(
            "RGBA byte length {rgba_byte_length} does not match {width}x{height} texture"
        ));
    }
    Ok(())
}

/// Compile WGSL binding metadata and injected source without crossing GPU objects through FFI.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = compileShader))]
pub fn compile_shader_json(request_json: &str) -> Result<String, String> {
    let request: open_quartz_execution::wgsl::CompileRequest = serde_json::from_str(request_json)
        .map_err(|error| format!("Invalid compiler request: {error}"))?;
    serde_json::to_string(&open_quartz_execution::wgsl::compile_shader(&request))
        .map_err(|error| format!("Cannot serialize compiled shader: {error}"))
}

/// Validate WGSL source and return compiler errors as JSON.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = validateShader))]
pub fn validate_shader_json(code: &str, preamble_lines: u32) -> String {
    serde_json::to_string(&open_quartz_execution::wgsl::validate_shader(code, preamble_lines))
        .expect("WGSL errors are always JSON serializable")
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = onnxBackend))]
pub fn onnx_backend() -> String {
    if cfg!(target_arch = "wasm32") {
        "onnxruntime-web-host".to_owned()
    } else {
        "ort-native".to_owned()
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = preprocessOnnxImage))]
pub fn preprocess_onnx_image(
    rgba: &[u8],
    width: u32,
    height: u32,
    target_size: u32,
) -> Result<String, String> {
    let tensor = open_quartz_execution::onnx::letterbox_preprocess(rgba, width, height, target_size)?;
    serde_json::to_string(&tensor).map_err(|error| format!("Cannot serialize ONNX tensor: {error}"))
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = planBrowserOnnxTask))]
pub fn plan_browser_onnx_task_json(request_json: &str) -> Result<String, String> {
    let request: open_quartz_execution::onnx::BrowserOnnxTaskPlanRequest =
        serde_json::from_str(request_json).map_err(|error| format!("Invalid ONNX task plan request: {error}"))?;
    let plan = open_quartz_execution::onnx::plan_browser_onnx_task(request)?;
    serde_json::to_string(&plan).map_err(|error| format!("Cannot serialize ONNX task plan: {error}"))
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = encodeBrowserOnnxInput))]
pub fn encode_browser_onnx_input_json(rgba: &[u8], request_json: &str) -> Result<String, String> {
    let request: open_quartz_execution::onnx::BrowserOnnxTensorRequest =
        serde_json::from_str(request_json).map_err(|error| format!("Invalid ONNX input request: {error}"))?;
    let tensor = open_quartz_execution::onnx::encode_browser_onnx_input(rgba, request)?;
    serde_json::to_string(&tensor).map_err(|error| format!("Cannot serialize ONNX input tensor: {error}"))
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = decodeBrowserOnnxOutput))]
pub fn decode_browser_onnx_output_json(
    source_rgba: &[u8],
    raw: &[f32],
    request_json: &str,
) -> Result<String, String> {
    let request: open_quartz_execution::onnx::BrowserOnnxOutputRequest =
        serde_json::from_str(request_json).map_err(|error| format!("Invalid ONNX output request: {error}"))?;
    let output = open_quartz_execution::onnx::decode_browser_onnx_output(source_rgba, raw, request)?;
    serde_json::to_string(&output).map_err(|error| format!("Cannot serialize ONNX output: {error}"))
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = buildBrowserOnnxCompletion))]
pub fn build_browser_onnx_completion_json(request_json: &str) -> Result<String, String> {
    let request: open_quartz_execution::onnx::BrowserOnnxCompletionRequest =
        serde_json::from_str(request_json).map_err(|error| format!("Invalid ONNX completion request: {error}"))?;
    let completion = open_quartz_execution::onnx::build_browser_onnx_completion(request);
    serde_json::to_string(&completion).map_err(|error| format!("Cannot serialize ONNX completion: {error}"))
}

#[allow(clippy::too_many_arguments)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = postprocessDetections))]
pub fn postprocess_detections_json(
    raw: &[f32],
    width: u32,
    height: u32,
    scale: f32,
    pad_x: f32,
    pad_y: f32,
    score_threshold: f32,
    iou_threshold: f32,
) -> Result<String, String> {
    let detections = open_quartz_execution::onnx::detect_postprocess(
        raw,
        width,
        height,
        scale,
        pad_x,
        pad_y,
        score_threshold,
        iou_threshold,
    );
    serde_json::to_string(&detections)
        .map_err(|error| format!("Cannot serialize detections: {error}"))
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = postprocessSegmentation))]
pub fn postprocess_segmentation_json(
    raw: &[f32],
    width: usize,
    height: usize,
    scale: f32,
    pad_x: f32,
    pad_y: f32,
) -> Result<String, String> {
    let segmentation = open_quartz_execution::onnx::segment_postprocess(raw, width, height, scale, pad_x, pad_y)?;
    serde_json::to_string(&segmentation)
        .map_err(|error| format!("Cannot serialize segmentation: {error}"))
}
