#[cfg(all(feature = "native-onnx", not(target_arch = "wasm32"), target_os = "windows"))]
#[test]
fn native_session_runs_identity_model() {
    let runtime = std::env::var_os("ORT_DYLIB_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../node_modules/.cache/open-quartz/desktop-runtime-win32-x64/onnxruntime.dll"));
    std::env::set_var("ORT_DYLIB_PATH", runtime);
    let mut session =
        open_quartz::inference::InferenceSession::from_memory(include_bytes!("data/identity.onnx"))
            .unwrap();
    assert_eq!(session.info().input_names, ["input"]);
    assert_eq!(session.info().output_names, ["output"]);
    let output = session.run_f32(vec![42.0], vec![1]).unwrap();
    assert_eq!(output.shape, [1]);
    assert_eq!(output.data, [42.0]);
    assert!(open_quartz::inference::validate_image_session(&session).is_err());
    let error = open_quartz::inference::run_image_task(
        &mut session, open_quartz::inference::InferenceTask::Generic,
        "scalar-identity", &[], 0, 0, 640, 0.25, 0.45,
    ).unwrap_err();
    assert!(!error.contains("RGBA"), "model interface must fail before preprocessing: {error}");
}

#[cfg(all(feature = "native-onnx", not(target_arch = "wasm32"), target_os = "windows"))]
#[test]
fn native_image_task_roundtrips_texture_pixels() {
    let runtime = std::env::var_os("ORT_DYLIB_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../node_modules/.cache/open-quartz/desktop-runtime-win32-x64/onnxruntime.dll"));
    std::env::set_var("ORT_DYLIB_PATH", runtime);
    let mut session = open_quartz::inference::InferenceSession::from_memory(include_bytes!(
        "data/image_identity.onnx"
    ))
    .unwrap();
    let rgba = vec![
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
    ];
    let stamp = open_quartz::inference::RequestStamp {
        request_id: 471,
        session_generation: session.generation(),
    };
    let output = open_quartz::inference::run_image_task_with_request(
        &mut session,
        stamp,
        open_quartz::inference::InferenceTask::Generic,
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

#[cfg(all(feature = "native-onnx", not(target_arch = "wasm32"), target_os = "windows"))]
#[test]
fn native_directml_session_runs_without_cpu_fallback() {
    let runtime = std::env::var_os("ORT_DYLIB_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../node_modules/.cache/open-quartz/desktop-runtime-win32-x64/onnxruntime.dll"));
    std::env::set_var("ORT_DYLIB_PATH", runtime);
    let capabilities = open_quartz::inference::inference_capabilities().unwrap();
    assert!(capabilities.cpu);
    assert!(capabilities.direct_ml);
    assert!(!capabilities.shared_wgpu_device);

    let mut session = open_quartz::inference::InferenceSession::from_memory_with_options(
        include_bytes!("data/identity.onnx"),
        open_quartz::inference::InferenceOptions {
            provider: open_quartz::inference::InferenceProvider::DirectMl,
            allow_cpu_fallback: false,
        },
    )
    .unwrap();
    assert_eq!(session.info().backend, "directml");
    let output = session.run_f32(vec![7.0], vec![1]).unwrap();
    assert_eq!(output.data, [7.0]);
}
