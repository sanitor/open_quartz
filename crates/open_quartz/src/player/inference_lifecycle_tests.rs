use std::time::{Duration, Instant};
use crate::inference::OnnxTask;

use super::*;

#[test]
fn configured_model_runs_through_native_host_and_exits_cleanly() {
    let runtime = std::env::var_os("ORT_DYLIB_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let platform = if cfg!(target_os = "macos") {
                "darwin"
            } else if cfg!(target_os = "windows") {
                "win32"
            } else {
                "linux"
            };
            let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" };
            let library = if cfg!(target_os = "macos") {
                "libonnxruntime.1.21.0.dylib"
            } else if cfg!(target_os = "windows") {
                "onnxruntime.dll"
            } else {
                "libonnxruntime.so.1.21.0"
            };
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../node_modules/.cache/open-quartz")
                .join(format!("desktop-runtime-{platform}-{arch}"))
                .join(library)
        });
    if !runtime.is_file() {
        return;
    }
    std::env::set_var("ORT_DYLIB_PATH", runtime);
    let (mut host, _) = pollster::block_on(NativePlayer::new()).unwrap();
    let graph = serde_json::json!({"nodes":[
        {"id":"image","type":"input","position":{"x":0,"y":0},"data":{"type":"input","label":"Image","shaderCode":"","inputs":[],"outputs":[{"id":"out","label":"output","dataType":"sampler2D","direction":"output"}],"uniforms":{},"inputMode":"image","inputDataType":"sampler2D"}},
        {"id":"onnx","type":"onnx","position":{"x":1,"y":0},"data":{"type":"onnx","label":"Image identity","shaderCode":"","inputs":[{"id":"in","label":"image","dataType":"sampler2D","direction":"input"}],"outputs":[{"id":"out","label":"output","dataType":"sampler2D","direction":"output"}],"uniforms":{}}},
        {"id":"renderer","type":"renderer","position":{"x":2,"y":0},"data":{"type":"renderer","label":"Renderer","shaderCode":"","inputs":[{"id":"in","label":"image","dataType":"sampler2D","direction":"input"}],"outputs":[],"uniforms":{}}}
    ],"edges":[
        {"id":"a","source":"image","sourceHandle":"out","target":"onnx","targetHandle":"in"},
        {"id":"b","source":"onnx","sourceHandle":"out","target":"renderer","targetHandle":"in"}
    ]});
    host.set_graph(&graph.to_string()).unwrap();
    let pixels = [
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
    ];
    host.upload_image("image", &pixels, 2, 2).unwrap();
    let model = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/image_identity.onnx");
    let prepared = crate::inference::PreparedOnnxModel::open(
        &model,
        NativeOnnxConfig {
            model_id: "image-identity".into(),
            task: OnnxTask::Generic,
            target_size: 2,
            score_threshold: 0.25,
            iou_threshold: 0.45,
        },
        false,
    )
    .unwrap();
    assert_eq!(
        prepared
            .attach(&mut host, "onnx".into())
            .unwrap()
            .input_names,
        ["input"]
    );
    host.start_playback().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !host.has_output_texture("onnx") && Instant::now() < deadline {
        host.render_next().unwrap();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(host.render_device.read_output_rgba("renderer").unwrap().0, pixels);
    assert!(host
        .take_output_events()
        .iter()
        .any(|event| event.node_id == "onnx" && event.backend == "cpu"));
}
