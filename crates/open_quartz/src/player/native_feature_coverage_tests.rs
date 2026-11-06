use super::NativePlayer;
use serde_json::json;

async fn runtime() -> NativePlayer {
    let (runtime, _) = NativePlayer::new().await.unwrap();
    runtime
}

fn shader_graph() -> String {
    json!({
        "nodes": [
            {
                "id": "shader", "type": "shader", "position": {"x": 0, "y": 0},
                "data": {
                    "type": "shader", "label": "Solid blue",
                    "shaderCode": "@fragment fn main() -> @location(0) vec4f { return vec4f(0.0, 0.0, 1.0, 1.0); }",
                    "inputs": [],
                    "outputs": [{"id": "out", "label": "output", "dataType": "sampler2D", "direction": "output"}],
                    "uniforms": {}, "autoSize": false, "width": 2, "height": 2
                }
            },
            {
                "id": "renderer", "type": "renderer", "position": {"x": 1, "y": 0},
                "data": {
                    "type": "renderer", "label": "Renderer", "shaderCode": "",
                    "inputs": [{"id": "in", "label": "image", "dataType": "sampler2D", "direction": "input"}],
                    "outputs": [], "uniforms": {}
                }
            }
        ],
        "edges": [{"id": "edge", "source": "shader", "sourceHandle": "out", "target": "renderer", "targetHandle": "in"}]
    }).to_string()
}

#[test]
fn unsupported_video_and_onnx_rejection_preserves_current_rendered_graph() {
    pollster::block_on(async {
        let mut runtime = runtime().await;
        let revision = runtime.set_graph(&shader_graph()).unwrap();
        runtime.start_playback().unwrap();
        runtime.render_next().unwrap();
        let before = runtime.read_output("renderer").unwrap();
        let plan = serde_json::to_value(runtime.session.execution_plan().unwrap()).unwrap();
        let generation = runtime.session.node_generation("shader").unwrap();
        assert_eq!(before.len(), 8 + 2 * 2 * 4);
        assert!(before[8..].chunks_exact(4).all(|pixel| pixel == [0, 0, 255, 255]));
        assert!(runtime.has_graph());
        assert!(runtime.has_output_texture("renderer"));

        let onnx = json!({"nodes":[{"id":"onnx","type":"onnx","position":{"x":0,"y":0},"data":{"type":"onnx","label":"ONNX","shaderCode":"","inputs":[],"outputs":[],"uniforms":{}}}],"edges":[]}).to_string();
        let error = runtime.set_graph(&onnx).unwrap_err();
        assert!(error.contains("requires the native-onnx feature"), "{error}");
        assert_eq!(runtime.session.revision(), revision);
        assert!(runtime.has_graph());
        assert!(runtime.has_output_texture("renderer"));
        assert_eq!(runtime.read_output("renderer").unwrap(), before);
        assert_eq!(serde_json::to_value(runtime.session.execution_plan().unwrap()).unwrap(), plan);
        assert_eq!(runtime.session.node_generation("shader").unwrap(), generation);
        assert!(runtime.session.node_generation("onnx").is_err());
        let frame = runtime.render_next().unwrap();
        assert_eq!(frame.revision, revision);
        assert_eq!(frame.output_node_id, "renderer");
        assert_eq!(runtime.read_output("renderer").unwrap(), before);

        let video = json!({"nodes":[{"id":"video","type":"input","position":{"x":0,"y":0},"data":{"type":"input","label":"Video","shaderCode":"","inputs":[],"outputs":[],"uniforms":{},"inputMode":"video"}}],"edges":[]}).to_string();
        let error = runtime.set_graph(&video).unwrap_err();
        assert!(error.contains("requires the native-video feature"), "{error}");
        assert_eq!(runtime.session.revision(), revision);
        assert!(runtime.has_graph());
        assert!(runtime.has_output_texture("renderer"));
        assert_eq!(runtime.read_output("renderer").unwrap(), before);
        assert_eq!(serde_json::to_value(runtime.session.execution_plan().unwrap()).unwrap(), plan);
        assert_eq!(runtime.session.node_generation("shader").unwrap(), generation);
        assert!(runtime.session.node_generation("video").is_err());
        let frame = runtime.render_next().unwrap();
        assert_eq!(frame.revision, revision);
        assert_eq!(frame.output_node_id, "renderer");
        assert_eq!(runtime.read_output("renderer").unwrap(), before);
    });
}

#[test]
fn invalid_graph_shader_and_target_leave_revision_generation_and_pixels_intact() {
    pollster::block_on(async {
        let mut player = runtime().await;
        let graph = shader_graph();
        let revision = player.set_graph(&graph).unwrap();
        player.start_playback().unwrap();
        player.render_next().unwrap();
        let pixels = player.read_output("renderer").unwrap();
        let generation = player.session.node_generation("shader").unwrap();
        let plan = serde_json::to_value(player.session.execution_plan().unwrap()).unwrap();
        let state = player.session.state();
        let mut duplicate: serde_json::Value = serde_json::from_str(&graph).unwrap();
        let shader = duplicate["nodes"][0].clone();
        duplicate["nodes"].as_array_mut().unwrap().push(shader);
        let mut invalid_shader: serde_json::Value = serde_json::from_str(&graph).unwrap();
        invalid_shader["nodes"][0]["data"]["shaderCode"] = json!("@fragment fn main() -> @location(0) vec4f { return missing_symbol; }");
        let mut invalid_target: serde_json::Value = serde_json::from_str(&graph).unwrap();
        invalid_target["nodes"][0]["data"]["width"] = json!(
            player.render_device.device().limits().max_texture_dimension_2d + 1
        );
        for invalid in [duplicate, invalid_shader, invalid_target] {
            assert!(player.set_graph(&invalid.to_string()).is_err());
            assert_eq!(player.session.revision(), revision);
            assert_eq!(player.session.node_generation("shader").unwrap(), generation);
            assert_eq!(player.session.state(), state);
            assert_eq!(serde_json::to_value(player.session.execution_plan().unwrap()).unwrap(), plan);
            assert_eq!(player.read_output("renderer").unwrap(), pixels);
            assert_eq!(player.render_next().unwrap().revision, revision);
            assert_eq!(player.read_output("renderer").unwrap(), pixels);
        }
    });
}

#[test]
fn close_releases_plan_and_textures_and_rejects_further_work() {
    pollster::block_on(async {
        let mut player = runtime().await;
        let graph = shader_graph();
        player.set_graph(&graph).unwrap();
        player.start_playback().unwrap();
        player.render_next().unwrap();
        let backend = player.backend();
        player.close().unwrap();
        assert!(!player.has_graph());
        assert!(!player.has_output_texture("shader"));
        assert!(!player.has_output_texture("renderer"));
        assert!(player.read_output("renderer").is_err());
        assert!(player.set_graph(&graph).is_err());
        assert!(player.render_next().is_err());
        assert!(std::sync::Arc::ptr_eq(&backend, &player.backend()));
    });
}
