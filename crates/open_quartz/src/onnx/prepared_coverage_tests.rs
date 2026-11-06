use super::*;
use crate::onnx::OnnxTask;

fn config() -> NativeOnnxConfig {
    NativeOnnxConfig {
        model_id: "offline-identity".to_owned(),
        task: OnnxTask::Generic,
        target_size: 1,
        score_threshold: 0.25,
        iou_threshold: 0.45,
    }
}

#[test]
fn offline_model_file_preserves_session_and_config_ownership() {
    const CHILD: &str = "OPEN_QUARTZ_PREPARED_MODEL_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let runtime = std::env::var_os("ORT_DYLIB_PATH")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                let architecture = if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" };
                let (platform, library) = if cfg!(target_os = "windows") {
                    ("win32", "onnxruntime.dll")
                } else if cfg!(target_os = "macos") {
                    ("darwin", "libonnxruntime.1.21.1.dylib")
                } else {
                    ("linux", "libonnxruntime.so.1")
                };
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../node_modules/onnxruntime-node/bin/napi-v3")
                    .join(platform)
                    .join(architecture)
                    .join(library)
            });
        assert!(runtime.is_file(), "ORT runtime library does not exist: {}", runtime.display());
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .env(CHILD, "1")
            .env("ORT_DYLIB_PATH", runtime)
            .arg("onnx::prepared::tests::offline_model_file_preserves_session_and_config_ownership")
            .arg("--exact")
            .arg("--nocapture")
            .status()
            .unwrap();
        assert!(status.success(), "offline prepared model child failed: {status}");
        return;
    }

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("identity.onnx");
    fs::write(&path, include_bytes!("../../tests/data/identity.onnx")).unwrap();
    let prepared = PreparedOnnxModel::open(&path, config(), false).unwrap();
    assert_eq!(prepared.info().input_names, ["input"]);
    assert_eq!(prepared.info().output_names, ["output"]);
    assert_eq!(prepared.info().backend, "cpu");
    let info = prepared.info().clone();
    let (mut session, owned_config) = prepared.into_parts();
    assert_eq!(session.info(), &info);
    assert_eq!(owned_config.model_id, "offline-identity");
    assert_eq!(owned_config.task, OnnxTask::Generic);
    assert_eq!(owned_config.target_size, 1);
    assert_eq!(owned_config.score_threshold, 0.25);
    assert_eq!(owned_config.iou_threshold, 0.45);
    let output = session.run_f32(vec![42.0], vec![1]).unwrap();
    assert_eq!(output.data, [42.0]);
    assert_eq!(output.shape, [1]);

    let preferred = PreparedOnnxModel::open(&path, config(), true).unwrap();
    assert_eq!(preferred.info().input_names, ["input"]);
    assert_eq!(preferred.info().output_names, ["output"]);
    assert_eq!(preferred.info().backend, if cfg!(windows) { "directml+cpu" } else { "cpu" });

    let missing = directory.path().join("missing.onnx");
    let error = match PreparedOnnxModel::open(&missing, config(), false) {
        Err(error) => error,
        Ok(_) => panic!("nonexistent model unexpectedly opened"),
    };
    assert!(error.starts_with("Cannot read ONNX model "), "{error}");
    assert!(error.contains("missing.onnx"), "{error}");

    let invalid = directory.path().join("invalid.onnx");
    fs::write(&invalid, b"not an ONNX model").unwrap();
    let error = match PreparedOnnxModel::open(&invalid, config(), false) {
        Err(error) => error,
        Ok(_) => panic!("invalid model unexpectedly opened"),
    };
    assert!(error.starts_with("Cannot load ONNX model:"), "{error}");
}
