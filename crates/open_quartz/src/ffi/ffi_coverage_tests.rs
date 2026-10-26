use super::*;

#[test]
fn ffi_surface_covers_versions_catalog_graph_shader_gpu_and_invalid_json() {
    assert_eq!(hello(), "OpenQuartz Rust SDK");
    assert_eq!(sdk_version(), SDK_VERSION);
    assert!(runtime_contract_json().contains("apiVersion"));
    assert!(catalog_json().contains("math"));
    assert!(parse_shader_json("not wgsl").contains("parseError"));

    assert!(plan_graph_json("{")
        .unwrap_err()
        .contains("Invalid graph JSON"));
    let plan = plan_graph_json(r#"{"nodes":[],"edges":[]}"#).unwrap();
    assert!(!plan.is_empty());

    assert!(plan_host_resource_intents_json("{")
        .unwrap_err()
        .contains("Invalid host resource intent request"));
    assert!(plan_host_resource_intents_json(
        r#"{"host":"browser","graph":{"nodes":[],"edges":[]}}"#
    )
    .unwrap()
    .contains("intents"));

    assert!(validate_gpu_texture(1, 1, 4).is_ok());
    assert!(validate_gpu_texture(0, 1, 0).is_err());
    assert!(validate_gpu_texture(1, 1, 3).is_err());

    assert!(compile_shader_json("{")
        .unwrap_err()
        .contains("Invalid compiler request"));
    let request = r#"{"userCode":"@fragment fn main() -> @location(0) vec4f { return vec4f(1.0); }","inputPorts":[],"upstreamMap":{},"videoInputs":[],"targetFormat":"rgba8unorm"}"#;
    assert!(compile_shader_json(request)
        .unwrap()
        .contains("fullFragmentCode"));
    assert_eq!(validate_shader_json("not wgsl", 0).starts_with('['), true);
    assert_eq!(onnx_backend(), "ort-native");
}

#[test]
fn ffi_onnx_surface_covers_success_and_every_decode_error() {
    let rgba = [255, 0, 0, 255, 0, 255, 0, 255];
    assert!(preprocess_onnx_image(&rgba, 2, 1, 2)
        .unwrap()
        .contains("shape"));
    assert!(preprocess_onnx_image(&rgba[..3], 2, 1, 2).is_err());
    assert!(plan_browser_onnx_task_json("{").is_err());
    assert!(encode_browser_onnx_input_json(&rgba, "{").is_err());
    assert!(decode_browser_onnx_output_json(&rgba, &[], "{").is_err());
    assert!(build_browser_onnx_completion_json("{").is_err());
    assert!(plan_browser_onnx_task_json(
        r#"{"modelId":"bad","task":"generic","sourceWidth":0,"sourceHeight":1,"params":{}}"#,
    )
    .is_err());

    let plan = plan_browser_onnx_task_json(
        r#"{"modelId":"yolov8n","task":"detection","sourceWidth":2,"sourceHeight":1,"params":{}}"#,
    )
    .unwrap();
    assert!(encode_browser_onnx_input_json(&rgba, &format!(r#"{{"plan":{plan}}}"#)).is_ok());
    assert!(decode_browser_onnx_output_json(
        &rgba,
        &vec![0.0; 84 * 8400],
        &format!(r#"{{"plan":{plan}}}"#),
    )
    .unwrap()
    .contains("data"));
    assert!(build_browser_onnx_completion_json(
        r#"{"nodeId":"onnx","graphRevision":1,"nodeGeneration":1,"inputStamp":{"epoch":1,"frame":1,"timelineNs":1,"deadlineNs":2},"data":null,"outputs":[]}"#,
    )
    .is_ok());
    let tiled_plan = plan_browser_onnx_task_json(
        r#"{"modelId":"realesrgan-x4plus","task":"super-resolution","sourceWidth":2,"sourceHeight":2,"params":{}}"#,
    )
    .unwrap();
    assert!(
        encode_browser_onnx_input_json(&rgba, &format!(r#"{{"plan":{tiled_plan}}}"#),).is_err()
    );
    assert!(
        decode_browser_onnx_output_json(&rgba, &[], &format!(r#"{{"plan":{tiled_plan}}}"#),)
            .is_err()
    );

    assert_eq!(
        postprocess_detections_json(&vec![0.0; 84 * 8400], 640, 640, 1.0, 0.0, 0.0, 0.25, 0.45)
            .unwrap(),
        "[]"
    );
    assert!(postprocess_segmentation_json(&[], 0, 0, 1.0, 0.0, 0.0).is_err());
    let raw = vec![0.0; 19 * 2 * 2];
    assert!(postprocess_segmentation_json(&raw, 2, 2, 1.0, 0.0, 0.0).is_ok());
}
