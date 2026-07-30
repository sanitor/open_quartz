use open_quartz::{onnx_backend, postprocess_detections_json, preprocess_onnx_image};

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

#[cfg(all(not(target_arch = "wasm32"), target_os = "windows"))]
#[test]
fn native_session_runs_identity_model() {
    let runtime = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../node_modules/onnxruntime-node/bin/napi-v3/win32/x64/onnxruntime.dll");
    std::env::set_var("ORT_DYLIB_PATH", runtime);
    let mut session =
        open_quartz::onnx::OnnxSession::from_memory(include_bytes!("data/identity.onnx")).unwrap();
    assert_eq!(session.info().input_names, ["input"]);
    assert_eq!(session.info().output_names, ["output"]);
    let output = session.run_f32(vec![42.0], vec![1]).unwrap();
    assert_eq!(output.shape, [1]);
    assert_eq!(output.data, [42.0]);
}

#[cfg(all(not(target_arch = "wasm32"), target_os = "windows"))]
#[test]
fn native_directml_session_runs_without_cpu_fallback() {
    let runtime = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../node_modules/onnxruntime-node/bin/napi-v3/win32/x64/onnxruntime.dll");
    std::env::set_var("ORT_DYLIB_PATH", runtime);
    let capabilities = open_quartz::onnx::native_onnx_capabilities().unwrap();
    assert!(capabilities.cpu);
    assert!(capabilities.direct_ml);
    assert!(!capabilities.shared_wgpu_device);

    let mut session = open_quartz::onnx::OnnxSession::from_memory_with_options(
        include_bytes!("data/identity.onnx"),
        open_quartz::onnx::NativeOnnxOptions {
            provider: open_quartz::onnx::NativeOnnxProvider::DirectMl,
            allow_cpu_fallback: false,
        },
    )
    .unwrap();
    assert_eq!(session.info().backend, "directml");
    let output = session.run_f32(vec![7.0], vec![1]).unwrap();
    assert_eq!(output.data, [7.0]);
}
