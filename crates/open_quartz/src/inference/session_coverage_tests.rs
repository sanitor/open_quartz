#![allow(unexpected_cfgs)]

use super::*;

#[test]
fn options_capabilities_sessions_and_tensor_errors_are_observable() {
    assert_eq!(
        NativeOnnxOptions::default().provider,
        NativeOnnxProvider::Cpu
    );
    let capabilities = native_onnx_capabilities().unwrap();
    assert!(capabilities.cpu);
    if !cfg!(target_os = "windows") {
        assert!(!capabilities.direct_ml);
    }

    if !cfg!(target_os = "windows") {
        assert!(OnnxSession::from_memory_with_options(
            b"unused",
            NativeOnnxOptions {
                provider: NativeOnnxProvider::DirectMl,
                allow_cpu_fallback: true,
            },
        )
        .is_err());
    }
}

#[test]
fn native_session_loads_and_runs_identity_model() {
    const CHILD: &str = "OPEN_QUARTZ_ONNX_COVERAGE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .env(CHILD, "1")
            .arg("inference::session::coverage_tests::native_session_loads_and_runs_identity_model")
            .arg("--exact")
            .arg("--nocapture")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }

    let architecture = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    };
    let (platform, library) = if cfg!(target_os = "windows") {
        ("win32", "onnxruntime.dll")
    } else if cfg!(target_os = "macos") {
        ("darwin", "libonnxruntime.1.21.0.dylib")
    } else {
        ("linux", "libonnxruntime.so.1.21.0")
    };
    let runtime = std::env::var_os("ORT_DYLIB_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../node_modules/.cache/open-quartz")
            .join(format!("desktop-runtime-{platform}-{architecture}"))
            .join(library));
    std::env::set_var("ORT_DYLIB_PATH", runtime);
    assert!(OnnxSession::from_memory(b"invalid").is_err());
    let mut session = OnnxSession::from_memory(include_bytes!(
        "../../../open_quartz/tests/data/identity.onnx"
    ))
    .unwrap();
    assert_eq!(session.info().backend, "cpu");
    assert_eq!(session.info().input_names, ["input"]);
    assert_eq!(session.info().output_names, ["output"]);
    let output = session.run_f32(vec![42.0], vec![1]).unwrap();
    assert_eq!(output.data, [42.0]);
    assert_eq!(output.shape, [1]);
    let output = TensorRunner::run_f32(&mut session, vec![7.0], vec![1]).unwrap();
    assert_eq!(output.data, [7.0]);
    let input_names = session.info.input_names.clone();
    session.info.input_names.clear();
    assert!(session.run_f32(vec![1.0], vec![1]).is_err());
    session.info.input_names = input_names;
    let rgba = [255, 0, 0, 255];
    for task in [
        crate::project::model::OnnxTask::SuperResolution,
        crate::project::model::OnnxTask::BackgroundRemoval,
        crate::project::model::OnnxTask::DepthEstimation,
        crate::project::model::OnnxTask::Detection,
        crate::project::model::OnnxTask::Segmentation,
        crate::project::model::OnnxTask::StyleTransfer,
        crate::project::model::OnnxTask::Denoising,
        crate::project::model::OnnxTask::Generic,
    ] {
        assert!(crate::inference::run_native_image_task(
            &mut session,
            task,
            "identity",
            &rgba,
            1,
            1,
            1,
            0.25,
            0.45,
        )
        .is_err());
    }
    assert!(session.run_f32(vec![], vec![2]).is_err());
}
