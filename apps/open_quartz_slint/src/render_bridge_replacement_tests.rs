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
    let wait_for = |bridge: &RenderBridge, predicate: &dyn Fn(&RenderUpdate) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(update) = bridge.take_update() {
                if predicate(&update) { return update; }
            }
            assert!(Instant::now() < deadline, "render worker did not reach expected state");
            thread::sleep(Duration::from_millis(5));
        }
    };
    bridge.set_graph(graph.clone(), BTreeMap::new(), true).unwrap();
    let accepted = wait_for(&bridge, &|update| update.texture.is_some());
    let revision = accepted.rendered_revision.unwrap();
    wait_for(&bridge, &|update| update.previews.as_ref().is_some_and(|previews| previews.contains_key(&video.id)));

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
        let rejected = wait_for(&bridge, &|update| update.status.as_ref()
            .is_some_and(|status| status.contains("Graph replacement rejected")));
        assert!(!rejected.clear_image);
        let continuing = wait_for(&bridge, &|update| update.texture.is_some());
        assert_eq!(continuing.rendered_revision, Some(revision));
        bridge.set_playback("pause").unwrap();
        bridge.capture_png(video.id.clone(), capture.clone()).unwrap()
            .recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        let pixels = image::open(&capture).unwrap().to_rgba8();
        assert_eq!(pixels.dimensions(), (64, 64));
        assert!(pixels.pixels().all(|pixel| pixel[0] > 240 && pixel[1] < 10 && pixel[2] < 10));
        let paused = bridge.take_update().unwrap();
        assert_eq!(paused.status.as_deref(), Some("Playback: pause"));
        bridge.set_playback("play").unwrap();
        let resumed = wait_for(&bridge, &|update| update.texture.is_some());
        assert_eq!(resumed.rendered_revision, Some(revision));
    }
    drop(bridge);
    std::fs::remove_dir_all(directory).unwrap();
}
