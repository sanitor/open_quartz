//! Explicit offline ORT reference coverage. NativePlayer never executes this path.
use super::*;

#[test]
fn offline_image_identity_preserves_original_pixel_assertions() {
    let runtime = std::env::var_os("ORT_DYLIB_PATH").map(std::path::PathBuf::from).unwrap_or_else(|| {
        let platform = if cfg!(target_os = "macos") { "darwin" } else if cfg!(windows) { "win32" } else { "linux" };
        let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" };
        let library = if cfg!(target_os = "macos") { "libonnxruntime.1.21.0.dylib" }
            else if cfg!(windows) { "onnxruntime.dll" } else { "libonnxruntime.so.1.21.0" };
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../node_modules/.cache/open-quartz")
            .join(format!("desktop-runtime-{platform}-{arch}")).join(library)
    });
    // The explicit reference feature may be built without installing the optional ORT library.
    if !runtime.is_file() { return; }
    std::env::set_var("ORT_DYLIB_PATH", runtime);
    let mut session = InferenceSession::from_memory_with_policy(
        include_bytes!("../../tests/data/image_identity.onnx"), &ExecutionPolicy::default()).unwrap();
    let pixels = [255,0,0,255, 0,255,0,255, 0,0,255,255, 255,255,255,255];
    let image = run_image_task(&mut session, InferenceTask::Generic, "image-identity", &pixels,
        2, 2, 2, 0.25, 0.45).unwrap();
    assert_eq!(image.rgba, pixels);
    assert_eq!((image.width, image.height), (2, 2));
}
