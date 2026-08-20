use open_quartz::{
    DataType, Environment, NodeData, NodeType, OpenQuartz, OutputPolicy, PlayerState, Port,
    PortDirection, Position, ProjectNode, Resource, ResourceId, ResourceKind, ResourceSource,
};
use serde_json::Map;

#[test]
fn player_exposes_a_single_predictable_lifecycle() {
    let sdk = OpenQuartz::new(Environment::headless());
    let project = sdk.create_project("Lifecycle");
    let mut player = sdk.player(project.graph()).build().unwrap();

    assert_eq!(player.state(), PlayerState::Ready);
    player.play().unwrap();
    assert_eq!(player.state(), PlayerState::Playing);
    player.pause().unwrap();
    assert_eq!(player.state(), PlayerState::Paused);
    player.resume().unwrap();
    assert_eq!(player.state(), PlayerState::Playing);
    player.stop().unwrap();
    assert_eq!(player.state(), PlayerState::Stopped);
    player.close().unwrap();
    assert_eq!(player.state(), PlayerState::Closed);
}

#[test]
fn invalid_player_transitions_return_structured_errors() {
    let sdk = OpenQuartz::new(Environment::headless());
    let project = sdk.create_project("Invalid transitions");
    let mut player = sdk.player(project.graph()).build().unwrap();

    let pause = player.pause().unwrap_err();
    assert_eq!(pause.code, open_quartz::SdkErrorCode::InvalidState);

    player.close().unwrap();
    let play = player.play().unwrap_err();
    assert_eq!(play.code, open_quartz::SdkErrorCode::Disposed);
}

#[test]
fn project_resources_are_public_descriptors_not_live_handles() {
    let sdk = OpenQuartz::new(Environment::headless());
    let mut project = sdk.create_project("Resources");
    project
        .resources_mut()
        .add(Resource::new(
            ResourceId::new("image"),
            ResourceKind::Image,
            ResourceSource::Path("photo.png".into()),
        ))
        .unwrap();

    let player = sdk
        .player(project.graph())
        .with_resources(project.resources())
        .build()
        .unwrap();

    assert_eq!(project.resources().len(), 1);
    assert_eq!(player.resources().len(), 1);
    assert_eq!(player.resources()[0].id().as_str(), "image");
}

#[test]
fn resource_catalog_rejects_duplicate_resource_ids() {
    let sdk = OpenQuartz::new(Environment::headless());
    let mut project = sdk.create_project("Resources");
    project
        .resources_mut()
        .add(Resource::new(
            ResourceId::new("image"),
            ResourceKind::Image,
            ResourceSource::Path("photo.png".into()),
        ))
        .unwrap();

    let error = project
        .resources_mut()
        .add(Resource::new(
            ResourceId::new("image"),
            ResourceKind::Image,
            ResourceSource::Path("other.png".into()),
        ))
        .unwrap_err();

    assert_eq!(error.code, open_quartz::SdkErrorCode::InvalidResource);
}

#[test]
fn output_creates_an_owned_subscription_that_can_close_independently() {
    let sdk = OpenQuartz::new(Environment::headless());
    let mut project = sdk.create_project("Outputs");
    project.graph_mut().nodes.push(ProjectNode {
        id: "source".into(),
        node_type: NodeType::Input,
        position: Position { x: 0.0, y: 0.0 },
        data: NodeData {
            node_type: NodeType::Input,
            label: "Source".into(),
            template_name: None,
            shader_template_id: None,
            shader_code: String::new(),
            inputs: vec![],
            outputs: vec![Port {
                id: "value".into(),
                label: "value".into(),
                data_type: DataType::Float,
                direction: PortDirection::Output,
                default_value: None,
                description: None,
            }],
            uniforms: Map::new(),
            input_data_type: Some(DataType::Float),
            input_mode: None,
            image_data_url: None,
            image_width: None,
            image_height: None,
            fb_format: None,
            fb_width: None,
            fb_height: None,
            fb_stride: None,
            raw_data_url: None,
            tex_filter: None,
            tex_wrap: None,
            width: None,
            height: None,
            auto_size: None,
            out_format: None,
            onnx_model_id: None,
            onnx_score_threshold: None,
            onnx_iou_threshold: None,
            onnx_target_size: None,
            onnx_source: None,
            onnx_catalog_id: None,
            onnx_custom_path: None,
            onnx_params: None,
            video_source_type: None,
            video_url: None,
            video_file_path: None,
            video_device_id: None,
            video_loop: None,
            video_playback_rate: None,
            system_source: None,
            math_op: None,
            feedback_enabled: None,
            feedback_clear_color: None,
            extra: Map::new(),
        },
    });
    let player = sdk.player(project.graph()).build().unwrap();
    let output = &player.outputs()[0];
    let mut subscription = output.subscribe(OutputPolicy::latest()).unwrap();

    assert!(!subscription.is_closed());
    subscription.close().unwrap();
    assert!(subscription.is_closed());
}
