use open_quartz::ffi::{
    build_browser_onnx_completion_json, decode_browser_onnx_output_json,
    encode_browser_onnx_input_json, onnx_backend, plan_browser_onnx_task_json,
    postprocess_detections_json, preprocess_onnx_image,
};

#[test]
fn onnx_ffi_preprocesses_and_serializes() {
    assert_eq!(onnx_backend(), "ort-native");
    let rgba = [255, 0, 0, 255, 0, 255, 0, 255];
    let tensor = preprocess_onnx_image(&rgba, 2, 1, 2).unwrap();
    assert!(tensor.contains("\"shape\":[1,3,2,2]"));
}

#[test]
fn onnx_detection_ffi_serializes_empty_results() {
    let raw = vec![0.0; 84 * 8400];
    assert_eq!(
        postprocess_detections_json(&raw, 640, 640, 1.0, 0.0, 0.0, 0.25, 0.45).unwrap(),
        "[]"
    );
}

#[test]
fn browser_onnx_task_ffi_roundtrips_plan_tensor_output_and_completion() {
    let plan = plan_browser_onnx_task_json(
        r#"{"modelId":"yolov8n","task":"detection","sourceWidth":2,"sourceHeight":1,"params":{"scoreThreshold":0.25}}"#,
    )
    .unwrap();
    assert!(plan.contains(r#""family":"detection""#));
    assert!(plan.contains(r#""classLabels":["person""#));

    let rgba = [255, 0, 0, 255, 0, 255, 0, 255];
    let tensor = encode_browser_onnx_input_json(&rgba, &format!(r#"{{"plan":{plan}}}"#)).unwrap();
    assert!(tensor.contains(r#""shape":[1,3,640,640]"#));

    let output = decode_browser_onnx_output_json(
        &rgba,
        &vec![0.0; 84 * 8400],
        &format!(r#"{{"plan":{plan}}}"#),
    )
    .unwrap();
    assert!(output.contains(r#""data":[]"#));

    let completion = build_browser_onnx_completion_json(
        r#"{"nodeId":"onnx","graphRevision":2,"nodeGeneration":5,"inputStamp":{"epoch":1,"frame":7,"timelineNs":9,"deadlineNs":10},"data":[],"outputs":[{"id":"overlay","dataType":"sampler2D"},{"id":"detections","dataType":"roi"}]}"#,
    )
    .unwrap();
    assert!(completion.contains(r#""handle":5"#));
    assert!(completion.contains(r#""timelineNs":9"#));
}

#[cfg(all(not(target_arch = "wasm32"), target_os = "windows"))]
#[test]
fn native_session_runs_identity_model() {
    let runtime = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../node_modules/onnxruntime-node/bin/napi-v3/win32/x64/onnxruntime.dll");
    std::env::set_var("ORT_DYLIB_PATH", runtime);
    let mut session =
        open_quartz_execution::onnx::OnnxSession::from_memory(include_bytes!("data/identity.onnx")).unwrap();
    assert_eq!(session.info().input_names, ["input"]);
    assert_eq!(session.info().output_names, ["output"]);
    let output = session.run_f32(vec![42.0], vec![1]).unwrap();
    assert_eq!(output.shape, [1]);
    assert_eq!(output.data, [42.0]);
}

#[cfg(all(not(target_arch = "wasm32"), target_os = "windows"))]
#[test]
fn native_image_task_roundtrips_texture_pixels() {
    let runtime = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../node_modules/onnxruntime-node/bin/napi-v3/win32/x64/onnxruntime.dll");
    std::env::set_var("ORT_DYLIB_PATH", runtime);
    let mut session =
        open_quartz_execution::onnx::OnnxSession::from_memory(include_bytes!("data/image_identity.onnx"))
            .unwrap();
    let rgba = vec![
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
    ];
    let output = open_quartz_execution::onnx::run_native_image_task(
        &mut session,
        open_quartz_execution::onnx::OnnxTask::Generic,
        "image-identity",
        &rgba,
        2,
        2,
        2,
        0.25,
        0.45,
    )
    .unwrap();
    assert_eq!((output.width, output.height), (2, 2));
    assert_eq!(output.rgba, rgba);
    assert!(output.data.is_none());
}

#[cfg(all(not(target_arch = "wasm32"), target_os = "windows"))]
#[test]
fn native_directml_session_runs_without_cpu_fallback() {
    let runtime = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../node_modules/onnxruntime-node/bin/napi-v3/win32/x64/onnxruntime.dll");
    std::env::set_var("ORT_DYLIB_PATH", runtime);
    let capabilities = open_quartz_execution::onnx::native_onnx_capabilities().unwrap();
    assert!(capabilities.cpu);
    assert!(capabilities.direct_ml);
    assert!(!capabilities.shared_wgpu_device);

    let mut session = open_quartz_execution::onnx::OnnxSession::from_memory_with_options(
        include_bytes!("data/identity.onnx"),
        open_quartz_execution::onnx::NativeOnnxOptions {
            provider: open_quartz_execution::onnx::NativeOnnxProvider::DirectMl,
            allow_cpu_fallback: false,
        },
    )
    .unwrap();
    assert_eq!(session.info().backend, "directml");
    let output = session.run_f32(vec![7.0], vec![1]).unwrap();
    assert_eq!(output.data, [7.0]);
}
