use open_quartz::{Environment, OpenQuartz, PlayerState};

#[test]
fn public_player_behavior_is_target_neutral() {
    let sdk = OpenQuartz::new(Environment::headless());
    let project = sdk.create_project("WASM parity");
    let mut player = sdk.player(project.graph()).build().unwrap();

    player.play().unwrap();
    assert_eq!(player.state(), PlayerState::Playing);
    player.pause().unwrap();
    assert_eq!(player.state(), PlayerState::Paused);
    player.resume().unwrap();
    player.stop().unwrap();
    assert_eq!(player.state(), PlayerState::Stopped);
}

#[cfg(target_arch = "wasm32")]
mod wasm_contract {
    use open_quartz::wasm_environment::{BrowserFrame, BrowserGpuEnvironment};
    use open_quartz::ffi::BrowserPlayerBinding;
    use serde_json::json;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn image_bitmap_is_a_typed_browser_frame_source() {
        fn accepts_frame(_: Option<BrowserFrame>) {}
        accepts_frame(None);
    }

    #[wasm_bindgen_test(async)]
    async fn offscreen_canvas_creates_rust_owned_webgpu_environment() {
        let canvas = web_sys::OffscreenCanvas::new(8, 8).unwrap();
        let environment = BrowserGpuEnvironment::from_offscreen_canvas(canvas)
            .await
            .unwrap();
        assert_eq!(environment.surface_format.is_srgb(), true);
    }

    #[wasm_bindgen_test(async)]
    async fn browser_player_executes_shader_and_presents_with_rust_wgpu() {
        let canvas = web_sys::OffscreenCanvas::new(4, 4).unwrap();
        let mut player = BrowserPlayerBinding::create(canvas).await.unwrap();
        let graph = json!({
            "nodes": [
                {
                    "id": "color", "type": "shader", "position": {"x": 0.0, "y": 0.0},
                    "data": {
                        "type": "shader", "label": "Color",
                        "shaderCode": "@group(0) @binding(0) var<uniform> color: vec4f; @fragment fn main() -> @location(0) vec4f { return color; }",
                        "inputs": [{"id": "color_in", "label": "color", "dataType": "vec4", "direction": "input"}],
                        "outputs": [{"id": "color_out", "label": "output", "dataType": "sampler2D", "direction": "output"}],
                        "uniforms": {"color": [0.25, 0.5, 0.75, 1.0]},
                        "autoSize": false, "width": 4, "height": 4
                    }
                },
                {
                    "id": "renderer", "type": "renderer", "position": {"x": 1.0, "y": 0.0},
                    "data": {
                        "type": "renderer", "label": "Renderer", "shaderCode": "",
                        "inputs": [{"id": "renderer_in", "label": "inputImage", "dataType": "sampler2D", "direction": "input"}],
                        "outputs": [], "uniforms": {}
                    }
                }
            ],
            "edges": [{
                "id": "e1", "source": "color", "sourceHandle": "color_out",
                "target": "renderer", "targetHandle": "renderer_in"
            }]
        });
        player.set_graph(&graph.to_string()).unwrap();
        player.play(0).unwrap();
        let result = player
            .frame(
                &json!({
                    "nowNs": 16_000_000,
                    "date": [2026.0, 8.0, 19.0, 0.0],
                    "mouse": [0.0, 0.0, 0.0, 0.0],
                    "resolution": [4.0, 4.0, 1.0]
                })
                .to_string(),
            )
            .unwrap();
        assert!(result.contains("\"inferenceTasks\":[]"));
        let rgba = player.read_output_rgba("renderer").await.unwrap();
        assert_eq!(&rgba[..4], &[64, 128, 191, 255]);
    }

    #[wasm_bindgen_test(async)]
    async fn browser_player_routes_onnx_work_to_the_host_provider() {
        let canvas = web_sys::OffscreenCanvas::new(2, 2).unwrap();
        let mut player = BrowserPlayerBinding::create(canvas).await.unwrap();
        player.set_graph(&json!({
            "nodes": [{
                "id": "onnx", "type": "onnx", "position": {"x": 0.0, "y": 0.0},
                "data": {
                    "type": "onnx", "label": "ONNX", "shaderCode": "", "inputs": [],
                    "outputs": [{"id": "result", "label": "result", "dataType": "json", "direction": "output"}],
                    "uniforms": {}
                }
            }],
            "edges": []
        }).to_string()).unwrap();
        player.play(0).unwrap();
        let result = player
            .frame(
                &json!({
                    "nowNs": 16_000_000,
                    "date": [2026.0, 8.0, 19.0, 0.0],
                    "mouse": [0.0, 0.0, 0.0, 0.0],
                    "resolution": [2.0, 2.0, 1.0]
                })
                .to_string(),
            )
            .unwrap();
        let result: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(result["inferenceTasks"][0]["nodeId"], "onnx");
        assert_eq!(result["inferenceTasks"][0]["kind"], "onnx");
    }

    #[wasm_bindgen_test(async)]
    async fn browser_player_uploads_external_media_without_cpu_readback() {
        let canvas = web_sys::OffscreenCanvas::new(2, 2).unwrap();
        let mut player = BrowserPlayerBinding::create(canvas).await.unwrap();
        player.set_graph(&json!({
            "nodes": [
                {
                    "id": "video", "type": "input", "position": {"x": 0.0, "y": 0.0},
                    "data": {
                        "type": "input", "label": "Video", "shaderCode": "", "inputs": [],
                        "outputs": [{"id": "video_out", "label": "output", "dataType": "sampler2D", "direction": "output"}],
                        "uniforms": {}, "inputMode": "video"
                    }
                },
                {
                    "id": "copy", "type": "shader", "position": {"x": 1.0, "y": 0.0},
                    "data": {
                        "type": "shader", "label": "Copy",
                        "shaderCode": "@group(0) @binding(0) var inputImage: texture_2d<f32>; @group(0) @binding(1) var inputImageSampler: sampler; @fragment fn main(@location(0) uv: vec2f) -> @location(0) vec4f { return textureSample(inputImage, inputImageSampler, uv); }",
                        "inputs": [{"id": "copy_in", "label": "inputImage", "dataType": "sampler2D", "direction": "input"}],
                        "outputs": [{"id": "copy_out", "label": "output", "dataType": "sampler2D", "direction": "output"}],
                        "uniforms": {}, "autoSize": false, "width": 2, "height": 2
                    }
                },
                {
                    "id": "renderer", "type": "renderer", "position": {"x": 2.0, "y": 0.0},
                    "data": {
                        "type": "renderer", "label": "Renderer", "shaderCode": "",
                        "inputs": [{"id": "renderer_in", "label": "inputImage", "dataType": "sampler2D", "direction": "input"}],
                        "outputs": [], "uniforms": {}
                    }
                }
            ],
            "edges": [
                {"id": "e1", "source": "video", "sourceHandle": "video_out", "target": "copy", "targetHandle": "copy_in"},
                {"id": "e2", "source": "copy", "sourceHandle": "copy_out", "target": "renderer", "targetHandle": "renderer_in"}
            ]
        }).to_string()).unwrap();
        let source = web_sys::OffscreenCanvas::new(2, 2).unwrap();
        player
            .upload_frame("video", source.transfer_to_image_bitmap().unwrap(), 0)
            .unwrap();
        player.play(0).unwrap();
        player
            .frame(
                &json!({
                    "nowNs": 16_000_000,
                    "date": [2026.0, 8.0, 19.0, 0.0],
                    "mouse": [0.0, 0.0, 0.0, 0.0],
                    "resolution": [2.0, 2.0, 1.0]
                })
                .to_string(),
            )
            .unwrap();
        assert_eq!(player.read_output_rgba("renderer").await.unwrap().len(), 16);
    }
}
