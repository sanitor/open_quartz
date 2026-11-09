use super::*;

/// Exercises the production background-preparation and render-worker path using
/// the catalog's real image model. Main runs this explicitly after integration.
#[test]
#[ignore = "requires a Metal GPU and the cached super-resolution-3x.onnx model; no ORT library"]
fn cached_image_model_runs_through_the_inference_render_worker() {
    run_cached_resident_image_model();
}

#[test]
#[ignore = "explicit diagnostic: requires Metal, cached SR3 model, and ORT_DYLIB_PATH for CPU reference only"]
fn cached_resident_image_matches_explicit_cpu_reference() {
    let (model_path, source, actual) = run_cached_resident_image_model();
    let bytes = std::fs::read(model_path).unwrap();
    let mut reference = open_quartz::inference::InferenceSession::from_memory(&bytes).unwrap();
    let expected = open_quartz::inference::run_image_task(
        &mut reference,
        open_quartz::InferenceTask::SuperResolution,
        "super-resolution-3x",
        &source,
        8,
        8,
        224,
        0.5,
        0.45,
    ).unwrap();
    assert_eq!(actual.dimensions(), (expected.width, expected.height));
    assert_eq!(actual.as_raw().len(), expected.rgba.len());
    let errors = actual.as_raw().iter().zip(&expected.rgba)
        .map(|(&gpu, &cpu)| gpu.abs_diff(cpu) as u64).collect::<Vec<_>>();
    assert!(errors.iter().copied().max().unwrap() <= 3, "GPU/CPU output diverged");
    assert!(errors.iter().sum::<u64>() as f64 / errors.len() as f64 <= 0.5,
        "GPU/CPU mean byte error exceeds rounding tolerance");
}

fn run_cached_resident_image_model() -> (PathBuf, Vec<u8>, image::RgbaImage) {
    use open_quartz::{DataType, Environment, GraphCommand, InputMode, NodeFactoryRequest, NodeId, OpenQuartz, PortId, PortKey};

    let base = directories::BaseDirs::new().expect("application data directory");
    let model_path = base.data_dir().join("com.openquartz.app/models/super-resolution-3x.onnx");
    assert!(model_path.is_file(), "cache the real catalog model first: {}", model_path.display());
    let model = open_quartz::project::catalog::inference_models().into_iter()
        .find(|model| model.id == "super-resolution-3x").expect("catalog image model");
    let sdk = OpenQuartz::new(Environment::headless());
    let mut project = sdk.create_project("Inference render acceptance");
    let (image, _) = project.create_graph_node(NodeFactoryRequest::Input {
        position: None, data_type: DataType::Sampler2d, input_mode: Some(InputMode::Image),
    }, project.graph_revision()).unwrap();
    let (inference, _) = project.create_graph_node(NodeFactoryRequest::Inference {
        position: None, label: model.label.clone(), template_name: Some(model.label),
        model_id: Some(model.id.clone()), catalog_id: Some(model.id),
        inputs: model.expected_io.inputs, outputs: model.expected_io.outputs,
    }, project.graph_revision()).unwrap();
    let (renderer, _) = project.create_graph_node(NodeFactoryRequest::Renderer {
        position: None,
    }, project.graph_revision()).unwrap();
    for (source, target) in [(&image, &inference), (&inference, &renderer)] {
        project.apply_graph_command(GraphCommand::Connect {
            source: PortKey::new(NodeId::new(&source.id), PortId::new(&source.data.outputs[0].id)),
            target: PortKey::new(NodeId::new(&target.id), PortId::new(&target.data.inputs[0].id)),
        }, project.graph_revision()).unwrap();
    }
    let mut graph = project.graph_snapshot();
    let node = graph.nodes.iter_mut().find(|node| node.id == inference.id).unwrap();
    let policy = open_quartz::inference::ExecutionPolicy::resident();
    node.data.inference_policy = Some(policy);
    const SOURCE_WIDTH: u32 = 8;
    const SOURCE_HEIGHT: u32 = 8;
    let expected_dimensions = (SOURCE_WIDTH * 3, SOURCE_HEIGHT * 3);
    let expected_status = format!("Inference output · {}×{}", expected_dimensions.0, expected_dimensions.1);
    let rgba = (0..SOURCE_HEIGHT).flat_map(|y| (0..SOURCE_WIDTH).flat_map(move |x| [x as u8 * 32, y as u8 * 32, 80, 255]))
        .collect::<Vec<u8>>();
    let images = [(image.id.clone(), ImageUpload { key: 1, width: SOURCE_WIDTH, height: SOURCE_HEIGHT, rgba: Arc::new(rgba.clone()) })]
        .into_iter().collect();
    let (mut bridge, configuration) = RenderBridge::initialize().unwrap();
    let WGPUConfiguration::Manual { device, queue, adapter, .. } = &configuration else {
        panic!("production preview must share the exact rendering device and queue");
    };
    assert_eq!(adapter.get_info().backend, wgpu::Backend::Metal);
    let mut shared_output = None;
    bridge.set_graph(graph, images, false).unwrap();
    bridge.load_inference(inference.id.clone(), model_path.to_string_lossy().into_owned()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut saw_loading = false;
    let mut attached = false;
    let mut completed = false;
    let mut received_texture = false;
    let mut shared_frames = 0;
    let mut metrics = open_quartz::player::InferenceMetrics::default();
    let mut last_status = String::from("No inference state received");
    while !completed || !received_texture || shared_frames < 3 || metrics.accepted == 0 {
        if let Some(update) = bridge.take_update() {
            assert!(update.node_error.is_none(), "{:?}", update.node_error);
            if let Some(texture) = update.texture {
                if texture.width == expected_dimensions.0 && texture.height == expected_dimensions.1 {
                    shared_frames += 1;
                    received_texture = true;
                    shared_output = Some(texture);
                }
            }
            if let Some(snapshot) = update.inference_metrics { metrics = snapshot; }
            if let Some(states) = update.inference {
                if let Some(state) = states.get(&inference.id) {
                    last_status.clone_from(&state.status);
                    assert!(!state.failed, "{}", state.status);
                    saw_loading |= state.loading;
                    if state.attached && !attached {
                        attached = true;
                        assert_eq!(state.backend, "metal");
                        assert!(state.execution.contains("MetalGpu"), "{}", state.execution);
                        assert!(state.execution.contains("Gpu"), "{}", state.execution);
                        assert!(state.execution.contains("Device"), "{}", state.execution);
                        assert!(state.execution.contains("host pixel/tensor bytes: 0"), "{}", state.execution);
                        assert!(state.artifact.contains("onnx"), "{}", state.artifact);
                        assert!(state.inputs.contains("Float32"), "{}", state.inputs);
                        let interface: serde_json::Value = serde_json::from_str(&state.interface).unwrap();
                        assert_eq!(interface["model"]["sourceInterface"]["producerName"], "pytorch");
                        assert_eq!(interface["model"]["opsetImports"][0]["version"], 10);
                        assert_eq!(interface["taskContract"]["adapter"], "luminance-super-resolution3x");
                        assert_eq!(interface["taskContract"]["inputLayout"], "NCHW");
                        bridge.set_playback("play").unwrap();
                    }
                    completed |= state.status == expected_status;
                }
            }
        }
        assert!(Instant::now() < deadline,
            "model did not produce a real image: loading={saw_loading}, attached={attached}, completed={completed}, last_status={last_status:?}, submitted={}, worker_completed={}, accepted={}, failed={}, stale={}, in_flight={}, last_inference_ns={}",
            metrics.submitted, metrics.completed, metrics.accepted, metrics.failed, metrics.stale, metrics.in_flight, metrics.last_inference_ns);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(attached && completed);
    assert!(metrics.submitted >= 1 && metrics.completed >= 1 && metrics.accepted >= 1);
    assert_eq!(metrics.failed, 0);
    assert_eq!(metrics.readback_bytes, 0);
    assert_eq!(metrics.upload_bytes, 0);
    assert_eq!(metrics.input_cpu_bytes, 0);
    assert_eq!(metrics.output_cpu_bytes, 0);
    assert!(metrics.device_submissions > 0);
    assert!(metrics.compute_dispatches > 0);
    assert!(metrics.last_inference_ns > 0);
    // Encoding/submitting the actual preview texture on Slint's exact device and
    // queue rejects cross-device textures without a CPU pixel round-trip.
    let output = shared_output.expect("resident output handle");
    assert!(output.lease.is_some(), "GUI output must retain its pooled GPU buffer lease");
    let texture = output.texture.as_ref();
    let destination = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("resident-acceptance-same-device-copy"),
        size: texture.size(),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: texture.format(),
        usage: wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("resident-acceptance-shared-texture-identity"),
    });
    encoder.copy_texture_to_texture(
        wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyTextureInfo { texture: &destination, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        texture.size(),
    );
    queue.submit(Some(encoder.finish()));
    bridge.set_playback("pause").unwrap();
    // This is an explicit user-operation diagnostic, not a production frame path.
    let diagnostic_id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let capture = std::env::temp_dir().join(format!("open-quartz-inference-{}-{diagnostic_id}.png", std::process::id()));
    bridge.capture_png(inference.id.clone(), capture.clone()).unwrap()
        .recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
    let pixels = image::open(&capture).unwrap().to_rgba8();
    assert_eq!(pixels.dimensions(), expected_dimensions);
    assert!(pixels.pixels().all(|pixel| pixel[3] == 255));
    assert!(pixels.pixels().any(|pixel| pixel[0] > 160));
    assert!(pixels.pixels().any(|pixel| pixel[0] < 80));
    assert!(pixels.pixels().any(|pixel| pixel[1] > 160));
    assert!(pixels.pixels().any(|pixel| pixel[1] < 80));
    std::fs::remove_file(capture).unwrap();
    bridge.unload_inference(inference.id).unwrap();
    (model_path, rgba, pixels)
}
