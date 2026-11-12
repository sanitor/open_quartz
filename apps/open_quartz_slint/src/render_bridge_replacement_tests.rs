use super::*;

#[test]
fn rejected_graph_replacements_keep_playback_revision_and_video_source() {
    use open_quartz::{DataType, Environment, GraphCommand, InputMode, NodeFactoryRequest,
        NodeId, OpenQuartz, PortId, PortKey, VideoSourceType};
    use std::time::{SystemTime, UNIX_EPOCH};

    let directory = std::env::temp_dir().join(format!("oq-rejected-graph-{}",
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("red.mp4");
    let ffmpeg = open_quartz::media::native::find_ffmpeg().unwrap();
    assert!(std::process::Command::new(ffmpeg).args([
        "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i",
        "color=c=red:s=64x64:r=10", "-t", "2", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-y",
    ]).arg(&path).status().unwrap().success());
    let sdk = OpenQuartz::new(Environment::headless());
    let mut project = sdk.create_project("Retained playback");
    let (video, _) = project.create_graph_node(NodeFactoryRequest::Input {
        position: None, data_type: DataType::Sampler2d, input_mode: Some(InputMode::Video),
    }, project.graph_revision()).unwrap();
    let (shader, _) = project.create_graph_node(NodeFactoryRequest::Shader {
        position: None,
        code: "@fragment fn main() -> @location(0) vec4f { return vec4f(1.0); }".into(),
        label: "Retained".into(), template_name: None, shader_template_id: None,
    }, project.graph_revision()).unwrap();
    let (renderer, _) = project.create_graph_node(NodeFactoryRequest::Renderer { position: None },
        project.graph_revision()).unwrap();
    project.apply_graph_command(GraphCommand::Connect {
        source: PortKey::new(NodeId::new(&shader.id), PortId::new(&shader.data.outputs[0].id)),
        target: PortKey::new(NodeId::new(&renderer.id), PortId::new(&renderer.data.inputs[0].id)),
    }, project.graph_revision()).unwrap();
    let mut graph = project.graph_snapshot();
    let node = graph.nodes.iter_mut().find(|node| node.id == video.id).unwrap();
    node.data.video_source_type = Some(VideoSourceType::File);
    node.data.video_file_path = Some(path.to_string_lossy().into_owned());
    let (mut bridge, _) = RenderBridge::initialize().unwrap();
    let wait_for = |bridge: &RenderBridge, label: &str, predicate: &dyn Fn(&RenderUpdate) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut last_status = None;
        loop {
            if let Some(update) = bridge.take_update() {
                if update.status.is_some() { last_status = update.status.clone(); }
                if predicate(&update) { return update; }
            }
            assert!(Instant::now() < deadline, "render worker did not reach {label}; last status: {last_status:?}");
            thread::sleep(Duration::from_millis(5));
        }
    };
    bridge.set_graph(graph.clone(), BTreeMap::new(), true).unwrap();
    let accepted = wait_for(&bridge, "initial texture", &|update| update.texture.is_some());
    let revision = accepted.rendered_revision.unwrap();
    wait_for(&bridge, "video node preview", &|update| update.previews.as_ref().is_some_and(|previews| previews.contains_key(&video.id)));

    let mut invalid_graph = graph.clone();
    invalid_graph.nodes.push(invalid_graph.nodes[0].clone());
    let mut invalid_shader = graph.clone();
    invalid_shader.nodes.iter_mut().find(|node| node.id == shader.id).unwrap()
        .data.shader_code = "@fragment fn main() -> @location(0) vec4f { return missing_value; }".into();
    let mut missing_video = graph.clone();
    missing_video.nodes.iter_mut().find(|node| node.id == video.id).unwrap()
        .data.video_file_path = Some(directory.join("missing.mp4").to_string_lossy().into_owned());
    let invalid_images = BTreeMap::from([(video.id.clone(), ImageUpload {
        key: 1, width: 1, height: 1, rgba: Arc::new(vec![0]),
    })]);
    let capture = directory.join("retained.png");
    for (candidate, images) in [
        (invalid_graph, BTreeMap::new()),
        (invalid_shader, BTreeMap::new()),
        (graph.clone(), invalid_images),
        (missing_video, BTreeMap::new()),
    ] {
        bridge.begin_project();
        bridge.set_graph(candidate, images, true).unwrap();
        let rejected = wait_for(&bridge, "rejected graph status", &|update| update.status.as_ref()
            .is_some_and(|status| status.contains("Graph replacement rejected")));
        assert!(!rejected.clear_image);
        drop(rejected);
        let continuing = wait_for(&bridge, "continued texture after rejection", &|update| update.texture.is_some());
        assert_eq!(continuing.rendered_revision, Some(revision));
        bridge.set_playback("pause").unwrap();
        bridge.capture_png(video.id.clone(), capture.clone()).unwrap()
            .recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        let pixels = image::open(&capture).unwrap().to_rgba8();
        assert_eq!(pixels.dimensions(), (64, 64));
        assert!(pixels.pixels().all(|pixel| pixel[0] > 240 && pixel[1] < 10 && pixel[2] < 10));
        let paused = bridge.take_update().unwrap();
        assert_eq!(paused.status.as_deref(), Some("Playback: pause"));
        drop(paused);
        bridge.set_playback("play").unwrap();
        let resumed = wait_for(&bridge, "resumed texture", &|update| update.texture.is_some());
        assert_eq!(resumed.rendered_revision, Some(revision));
    }
    drop(bridge);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn paused_selected_preview_retries_after_slow_consumers_release_without_resuming() {
    let (mut bridge, config) = RenderBridge::initialize().unwrap();
    let WGPUConfiguration::Manual { device, queue, .. } = config else { panic!("shared GPU config required") };
    let backend = open_quartz::rendering::GpuBackend::from_device(device.clone(), queue);
    let graph: Graph = serde_json::from_value(serde_json::json!({"nodes":[
        {"id":"blue","type":"shader","position":{"x":0,"y":0},"data":{"type":"shader","label":"Blue","shaderCode":"@fragment fn main()->@location(0) vec4f{return vec4f(0.0,0.0,1.0,1.0);}","inputs":[],"outputs":[{"id":"out","label":"fragColor","dataType":"vec4","direction":"output"}],"uniforms":{},"autoSize":false,"width":2,"height":2}},
        {"id":"red","type":"shader","position":{"x":0,"y":0},"data":{"type":"shader","label":"Red","shaderCode":"@fragment fn main()->@location(0) vec4f{return vec4f(1.0,0.0,0.0,1.0);}","inputs":[],"outputs":[{"id":"out","label":"fragColor","dataType":"vec4","direction":"output"}],"uniforms":{},"autoSize":false,"width":2,"height":2}}
    ],"edges":[]})).unwrap();
    bridge.select_preview(Some("blue".into())).unwrap();
    bridge.set_graph(graph, BTreeMap::new(), true).unwrap();
    let wait_for = |predicate: &dyn Fn(&RenderUpdate) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(update) = bridge.take_update() {
                assert!(update.node_error.is_none(), "{:?}", update.node_error);
                if predicate(&update) { return update; }
            }
            assert!(Instant::now() < deadline, "preview request was not retried after capacity release");
            thread::sleep(Duration::from_millis(5));
        }
    };
    let mut consumers = Vec::new();
    for _ in 0..3 {
        consumers.push(wait_for(&|u| u.texture.is_some()).texture.unwrap());
    }
    bridge.set_playback("pause").unwrap();
    wait_for(&|u| u.status.as_deref() == Some("Playback: pause"));
    bridge.select_preview(Some("red".into())).unwrap();
    wait_for(&|u| u.status.as_deref() == Some("Waiting for selected node preview…"));
    for handle in &consumers {
        let pixels = pollster::block_on(backend.read_texture_rgba(&handle.texture, 2, 2)).unwrap();
        assert_eq!(pixels, [0, 0, 255, 255].repeat(4));
    }
    // No play, seek or further UI command is sent after releasing a consumer.
    drop(consumers.remove(0));
    let red = wait_for(&|u| u.texture.is_some()).texture.unwrap();
    assert_eq!(pollster::block_on(backend.read_texture_rgba(&red.texture, 2, 2)).unwrap(), [255, 0, 0, 255].repeat(4));
    assert_eq!(pollster::block_on(backend.read_texture_rgba(&consumers[0].texture, 2, 2)).unwrap(), [0, 0, 255, 255].repeat(4));
    drop(red); drop(consumers); drop(bridge);
}
