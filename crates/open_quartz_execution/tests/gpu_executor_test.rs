use std::collections::HashSet;
use std::sync::Arc;

use open_quartz_execution::engine::{ExecutionEngine, FrameInputs};
use open_quartz_execution::gpu::{GpuBackend, GpuExecutor};
use open_quartz_schema::{Edge, ProjectNode};
use serde_json::json;

fn frame() -> FrameInputs {
    FrameInputs {
        time: 0.0,
        delta: 1.0 / 60.0,
        frame: 1,
        date: [2026.0, 7.0, 29.0, 0.0],
        mouse: [0.0; 4],
        resolution: [4.0, 4.0, 1.0],
        video_nodes: Vec::new(),
    }
}

async fn request_backend() -> Arc<GpuBackend> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: !cfg!(target_os = "macos"),
            compatible_surface: None,
        })
        .await
        .expect("a GPU adapter is required for native GPU tests");
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default())
        .await
        .expect("native GPU device creation must succeed");
    Arc::new(GpuBackend::from_device(device, queue))
}

#[test]
fn executes_shader_cascade_and_exposes_renderer_output() {
    pollster::block_on(async {
        let nodes: Vec<ProjectNode> = serde_json::from_value(json!([
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
                "id": "copy", "type": "shader", "position": {"x": 1.0, "y": 0.0},
                "data": {
                    "type": "shader", "label": "Copy",
                    "shaderCode": "@group(0) @binding(0) var inputImage: texture_2d<f32>; @group(0) @binding(1) var inputImageSampler: sampler; @fragment fn main(@location(0) uv: vec2f) -> @location(0) vec4f { return textureSample(inputImage, inputImageSampler, uv); }",
                    "inputs": [{"id": "copy_in", "label": "inputImage", "dataType": "sampler2D", "direction": "input"}],
                    "outputs": [{"id": "copy_out", "label": "output", "dataType": "sampler2D", "direction": "output"}],
                    "uniforms": {}, "autoSize": false, "width": 4, "height": 4
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
        ]))
        .unwrap();
        let edges: Vec<Edge> = serde_json::from_value(json!([
            {"id": "e1", "source": "color", "sourceHandle": "color_out", "target": "copy", "targetHandle": "copy_in"},
            {"id": "e2", "source": "copy", "sourceHandle": "copy_out", "target": "renderer", "targetHandle": "renderer_in"}
        ]))
        .unwrap();
        let mut engine = ExecutionEngine::prepare(nodes, edges);
        let result = engine.run_frame(&frame());
        let backend = request_backend().await;
        let mut executor = GpuExecutor::new(backend.clone());

        executor.execute(engine.plan(), &result).unwrap();
        let target = executor.output_target("renderer").unwrap();
        let rgba = backend.read_target_rgba(target).await.unwrap();

        assert_eq!(&rgba[..4], &[64, 128, 191, 255]);
        assert!(rgba.chunks_exact(4).all(|pixel| pixel == &rgba[..4]));
    });
}

#[test]
fn connected_system_time_updates_hue_uniform_on_native_gpu() {
    pollster::block_on(async {
        let nodes: Vec<ProjectNode> = serde_json::from_value(json!([
            {
                "id": "time", "type": "input", "position": {"x": 0.0, "y": 0.0},
                "data": {
                    "type": "input", "label": "Time", "shaderCode": "", "inputs": [],
                    "outputs": [{"id": "time_out", "label": "value", "dataType": "float", "direction": "output"}],
                    "uniforms": {}, "inputMode": "system", "inputDataType": "float", "systemSource": "time"
                }
            },
            {
                "id": "hue", "type": "shader", "position": {"x": 1.0, "y": 0.0},
                "data": {
                    "type": "shader", "label": "Hue Rotate",
                    "shaderCode": "@group(0) @binding(0) var<uniform> angle: f32; @fragment fn main() -> @location(0) vec4f { return vec4f(angle, 0.0, 0.0, 1.0); }",
                    "inputs": [{"id": "angle", "label": "angle", "dataType": "float", "direction": "input"}],
                    "outputs": [{"id": "hue_out", "label": "output", "dataType": "sampler2D", "direction": "output"}],
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
        ])).unwrap();
        let edges: Vec<Edge> = serde_json::from_value(json!([
            {"id": "time_to_hue", "source": "time", "sourceHandle": "time_out", "target": "hue", "targetHandle": "angle"},
            {"id": "hue_to_renderer", "source": "hue", "sourceHandle": "hue_out", "target": "renderer", "targetHandle": "renderer_in"}
        ])).unwrap();
        let mut engine = ExecutionEngine::prepare(nodes, edges);
        let backend = request_backend().await;
        let mut executor = GpuExecutor::new(backend.clone());

        let mut first_frame = frame();
        first_frame.time = 0.25;
        let first_work = engine.run_frame(&first_frame);
        executor.execute(engine.plan(), &first_work).unwrap();
        let first = executor.read_output_rgba("renderer").await.unwrap();

        let mut second_frame = frame();
        second_frame.time = 0.75;
        second_frame.frame = 2;
        let second_work = engine.run_frame(&second_frame);
        executor.execute(engine.plan(), &second_work).unwrap();
        let second = executor.read_output_rgba("renderer").await.unwrap();

        assert_eq!(&first[..4], &[64, 0, 0, 255]);
        assert_eq!(&second[..4], &[191, 0, 0, 255]);
    });
}

#[test]
fn position_only_update_preserves_gpu_feedback_targets() {
    pollster::block_on(async {
        let nodes: Vec<ProjectNode> = serde_json::from_value(json!([{
            "id": "feedback", "type": "shader", "position": {"x": 0.0, "y": 0.0},
            "data": {
                "type": "shader", "label": "Feedback",
                "shaderCode": "@fragment fn main(@location(0) uv: vec2f) -> @location(0) vec4f { return textureSample(previousFrame, previousFrameSampler, uv) + vec4f(0.25, 0.0, 0.0, 1.0); }",
                "inputs": [],
                "outputs": [{"id": "feedback_out", "label": "output", "dataType": "sampler2D", "direction": "output"}],
                "uniforms": {}, "autoSize": false, "width": 2, "height": 2
            }
        }]))
        .unwrap();
        let mut engine = ExecutionEngine::prepare(nodes.clone(), Vec::new());
        let backend = request_backend().await;
        let mut executor = GpuExecutor::new(backend.clone());

        let first = engine.run_frame(&frame());
        executor.execute(engine.plan(), &first).unwrap();
        let mut moved = nodes;
        moved[0].position.x = 100.0;
        engine.replace_graph_preserving_state(moved, Vec::new(), &HashSet::new());
        let mut next_frame = frame();
        next_frame.frame = 2;
        let second = engine.run_frame(&next_frame);
        executor.execute(engine.plan(), &second).unwrap();
        let target = executor.output_target("feedback").unwrap();
        let rgba = backend.read_target_rgba(target).await.unwrap();

        assert_eq!(&rgba[..4], &[128, 0, 0, 255]);
    });
}

#[test]
fn uploads_reuses_and_releases_image_texture_resources() {
    pollster::block_on(async {
        let nodes: Vec<ProjectNode> = serde_json::from_value(json!([
            {
                "id": "image", "type": "input", "position": {"x": 0.0, "y": 0.0},
                "data": {
                    "type": "input", "label": "Image", "shaderCode": "",
                    "inputs": [],
                    "outputs": [{"id": "image_out", "label": "output", "dataType": "sampler2D", "direction": "output"}],
                    "uniforms": {}, "inputMode": "image", "inputDataType": "sampler2D",
                    "imageWidth": 2, "imageHeight": 2
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
        ]))
        .unwrap();
        let edges: Vec<Edge> = serde_json::from_value(json!([
            {"id": "e1", "source": "image", "sourceHandle": "image_out", "target": "copy", "targetHandle": "copy_in"},
            {"id": "e2", "source": "copy", "sourceHandle": "copy_out", "target": "renderer", "targetHandle": "renderer_in"}
        ]))
        .unwrap();
        let backend = request_backend().await;
        let mut executor = GpuExecutor::new(backend.clone());
        let pixels = [
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ];
        executor.upload_rgba("image", &pixels, 2, 2).unwrap();
        let first_texture = executor.output_texture("image").unwrap().texture as *const _;
        executor.upload_rgba("image", &pixels, 2, 2).unwrap();
        let reused_texture = executor.output_texture("image").unwrap().texture as *const _;
        assert_eq!(first_texture, reused_texture);

        let mut engine = ExecutionEngine::prepare(nodes, edges);
        let result = engine.run_frame(&frame());
        executor.execute(engine.plan(), &result).unwrap();
        let rgba = executor.read_output_rgba("renderer").await.unwrap();
        assert_eq!(rgba, pixels);

        executor.remove_texture("image");
        assert!(executor.output_texture("image").is_none());
    });
}
