use super::*;
use open_quartz::inference::{InferenceDiagnosticSnapshot, InferenceSession, NamedTensor, RequestStamp};
use open_quartz::project::catalog::{inference_models, InferenceModelDescriptor};
use open_quartz::{DataType, Environment, GraphCommand, InputMode, NodeFactoryRequest, NodeId,
    OpenQuartz, PortId, PortKey, VideoSourceType};
use serde_json::{json, Value};
use std::path::Path;

const TIMEOUT: Duration = Duration::from_secs(180);
const ABS: f64 = 1e-3;
const REL: f64 = 1e-3;

// Separate processes are intentional: production must work with an invalid ORT
// path; only the reference process may load the explicitly provided CPU runtime.
#[test]
#[ignore = "explicit seven-model Metal/VideoToolbox lane; pinned manifest required; ORT must be unavailable"]
fn inference_e2e_production() {
    let root = output_root();
    let mut report = json!({"schemaVersion":1,"suite":"inference-production","passed":false,
        "productionPassed":false,"referencePending":true,"cases":[],"models":[],
        "boundaries":{"diagnostics":"explicit GPU-only armed snapshots and opt-in readback; no per-frame host pixels/tensors",
        "saturation":"generic three-lease capacity/reuse covered by catalog_benchmark and resident task unit tests; this lane tests retained renderer consumer lease across unload/project replacement",
        "performance":"no wall-clock FPS acceptance gates"}});
    save_json(&root.join("production.json"), &report);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let manifest = manifest();
        let catalog = inference_models();
        assert_eq!(catalog.len(), 7, "seven-model acceptance must be updated explicitly when catalog changes");
        assert_eq!(serde_json::from_value::<Vec<InferenceModelDescriptor>>(manifest["catalog"].clone()).unwrap(), catalog,
            "manifest must export complete current descriptors");
        let ort = std::env::var("ORT_DYLIB_PATH").expect("driver must explicitly disable ORT in production process");
        assert!(!Path::new(&ort).is_file(), "production proof requires unavailable ORT_DYLIB_PATH");
        assert!(open_quartz::media::native::find_ffmpeg().is_ok(), "real video runtime prerequisite");
        for model in &catalog {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_model(model, &manifest, &root, &mut report)
            }));
            report["models"].as_array_mut().unwrap().push(json!({"id":model.id,"passed":result.is_ok(),
                "error":result.as_ref().err().map(panic_text)}));
            save_json(&root.join("production.json"), &report);
        }
        assert!(report["models"].as_array().unwrap().iter().all(|m| m["passed"] == true),
            "at least one catalog model failed; see production.json");
        report["productionPassed"] = json!(true);
    }));
    if let Err(error) = &outcome { report["error"] = json!(panic_text(error)); }
    save_json(&root.join("production.json"), &report);
    assert!(outcome.is_ok(), "production inference lane failed; see {}", root.join("production.json").display());
}

#[test]
#[ignore = "explicit independent CPU reference for every saved production image/video snapshot; ORT_DYLIB_PATH required"]
fn inference_e2e_reference() {
    let root = output_root();
    let mut report: Value = serde_json::from_slice(&std::fs::read(root.join("production.json")).unwrap()).unwrap();
    report["passed"] = json!(false);
    report["referencePending"] = json!(true);
    save_json(&root.join("production.json"), &report);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let manifest = manifest();
        assert_eq!(report["productionPassed"], true, "reference cannot pass a partial production run");
        let ort = std::env::var("ORT_DYLIB_PATH").expect("explicit CPU reference runtime required");
        assert!(Path::new(&ort).is_file(), "missing CPU reference runtime: {ort}");
        for case in report["cases"].as_array_mut().unwrap() {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| reference_case(case, &manifest, &root)));
            case["referencePassed"] = json!(result.is_ok());
            if let Err(error) = result { case["referenceError"] = json!(panic_text(&error)); }
        }
        let cases = report["cases"].as_array().unwrap();
        for model in inference_models() {
            for input in ["image", "alternateImage", "constantImage", "video", "video-updated"] {
                assert!(cases.iter().any(|c| c["modelId"] == model.id && c["input"] == input && c["referencePassed"] == true),
                    "missing successful independent reference: {} {input}", model.id);
            }
        }
        assert!(cases.iter().all(|c| c["referencePassed"] == true), "reference parity failure; see per-case errors");
        report["referencePending"] = json!(false);
        report["passed"] = json!(true);
    }));
    if let Err(error) = &outcome { report["error"] = json!(panic_text(error)); }
    save_json(&root.join("production.json"), &report);
    assert!(outcome.is_ok(), "independent CPU inference parity failed; see production.json");
}

fn panic_text(error: &Box<dyn std::any::Any + Send>) -> String {
    error.downcast_ref::<String>().cloned().or_else(|| error.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_else(|| "non-string panic".into())
}
fn output_root() -> PathBuf {
    let root = PathBuf::from(std::env::var("OPEN_QUARTZ_INFERENCE_E2E_OUTPUT").expect("report directory required"));
    assert!(root.is_absolute());
    std::fs::create_dir_all(&root).unwrap();
    root
}
fn manifest() -> Value {
    let path = PathBuf::from(std::env::var("OPEN_QUARTZ_INFERENCE_E2E_MANIFEST").expect("pinned manifest required"));
    assert!(path.is_absolute() && path.is_file(), "missing absolute manifest: {}", path.display());
    let value: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(serde_json::from_value::<Vec<InferenceModelDescriptor>>(value["catalog"].clone()).unwrap(), inference_models());
    assert_eq!(value["models"].as_array().unwrap().len(), inference_models().len());
    for model in value["models"].as_array().unwrap() {
        let path = Path::new(model["path"].as_str().unwrap());
        assert!(path.is_absolute() && path.is_file(), "missing model {}", path.display());
        assert_eq!(model["sha256"].as_str().unwrap().len(), 64, "pinned digest required");
    }
    for fixture in ["image", "alternateImage", "constantImage", "video"] {
        let path = Path::new(value["fixtures"][fixture]["path"].as_str().unwrap());
        assert!(path.is_absolute() && path.is_file(), "missing fixture {fixture}");
    }
    assert!(value["fixtures"]["video"]["frameCount"].as_u64().unwrap() > 1);
    value
}
fn model_path(manifest: &Value, id: &str) -> String {
    manifest["models"].as_array().unwrap().iter().find(|m| m["id"] == id)
        .expect("canonical model artifact missing")["path"].as_str().unwrap().to_owned()
}
fn save_json(path: &Path, value: &Value) {
    std::fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

struct TestGraph { graph: Graph, source: String, inference: String, renderer: String }
fn test_graph(model: &InferenceModelDescriptor, video: Option<&str>) -> TestGraph {
    let sdk = OpenQuartz::new(Environment::headless());
    let mut project = sdk.create_project("Canonical production inference acceptance");
    let (source, _) = project.create_graph_node(NodeFactoryRequest::Input { position: None,
        data_type: DataType::Sampler2d, input_mode: Some(if video.is_some() { InputMode::Video } else { InputMode::Image }) },
        project.graph_revision()).unwrap();
    let (inference, _) = project.create_graph_node(NodeFactoryRequest::Inference { position: None,
        label: model.label.clone(), template_name: Some(model.label.clone()), model_id: Some(model.id.clone()),
        catalog_id: Some(model.id.clone()), inputs: model.expected_io.inputs.clone(), outputs: model.expected_io.outputs.clone() },
        project.graph_revision()).unwrap();
    let (renderer, _) = project.create_graph_node(NodeFactoryRequest::Renderer { position: None }, project.graph_revision()).unwrap();
    // YOLO's first output is ROI, not a texture. Never wire ports by position.
    let image_output = inference.data.outputs.iter().find(|p| p.data_type == DataType::Sampler2d).unwrap();
    for (from_node, from_port, to_node, to_port) in [
        (&source.id, &source.data.outputs[0].id, &inference.id, &inference.data.inputs[0].id),
        (&inference.id, &image_output.id, &renderer.id, &renderer.data.inputs[0].id),
    ] {
        project.apply_graph_command(GraphCommand::Connect {
            source: PortKey::new(NodeId::new(from_node), PortId::new(from_port)),
            target: PortKey::new(NodeId::new(to_node), PortId::new(to_port)),
        }, project.graph_revision()).unwrap();
    }
    let mut graph = project.graph_snapshot();
    let node = graph.nodes.iter_mut().find(|n| n.id == inference.id).unwrap();
    node.data.inference_policy = Some(open_quartz::inference::ExecutionPolicy::resident());
    if let Some(path) = video {
        let node = graph.nodes.iter_mut().find(|n| n.id == source.id).unwrap();
        node.data.video_source_type = Some(VideoSourceType::File);
        node.data.video_file_path = Some(path.to_owned());
    }
    TestGraph { graph, source: source.id, inference: inference.id, renderer: renderer.id }
}
fn images(graph: &TestGraph, image: &image::RgbaImage, key: u64) -> BTreeMap<String, ImageUpload> {
    BTreeMap::from([(graph.source.clone(), ImageUpload { key, width: image.width(), height: image.height(),
        rgba: Arc::new(image.as_raw().clone()) })])
}
fn wait_update(bridge: &RenderBridge, predicate: impl Fn(&RenderUpdate) -> bool) -> RenderUpdate {
    let deadline = Instant::now() + TIMEOUT;
    let mut last = String::new();
    loop {
        if let Some(update) = bridge.take_update() {
            assert!(update.node_error.is_none(), "{:?}", update.node_error);
            if let Some(status) = &update.status { last.clone_from(status); }
            if let Some(states) = &update.inference {
                for state in states.values() { assert!(!state.failed, "{}", state.status); }
            }
            if predicate(&update) { return update; }
        }
        assert!(Instant::now() < deadline, "production render deadline exceeded: {last}");
        thread::sleep(Duration::from_millis(5));
    }
}
fn load(bridge: &RenderBridge, graph: &TestGraph, path: &str, sha256: &str) {
    bridge.load_inference(graph.inference.clone(), path.to_owned()).unwrap();
    let update = wait_update(bridge, |u| u.inference.as_ref().and_then(|s| s.get(&graph.inference)).is_some_and(|s| s.attached));
    let state = &update.inference.as_ref().unwrap()[&graph.inference];
    assert_eq!(state.backend, "metal");
    assert!(state.artifact.ends_with(&format!("SHA-256 {sha256}")), "loaded bytes must match the pinned model: {}", state.artifact);
    for evidence in ["MetalGpu", "Gpu", "Device", "host pixel/tensor bytes: 0"] {
        assert!(state.execution.contains(evidence), "{}", state.execution);
    }
}
fn arm(bridge: &RenderBridge, node: &str) {
    let (response, receiver) = mpsc::channel();
    bridge.sender.send(Command::RequestInferenceSnapshot { generation: bridge.generation, node_id: node.to_owned(), response }).unwrap();
    receiver.recv_timeout(TIMEOUT).unwrap().unwrap();
}
fn snapshot(bridge: &RenderBridge, node: &str) -> InferenceDiagnosticSnapshot {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let (response, receiver) = mpsc::channel();
        bridge.sender.send(Command::TakeInferenceSnapshot { generation: bridge.generation, node_id: node.to_owned(), response }).unwrap();
        if let Some(snapshot) = receiver.recv_timeout(TIMEOUT).unwrap().unwrap() { return snapshot; }
        if let Some(update) = bridge.take_update() {
            assert!(update.node_error.is_none(), "{:?}", update.node_error);
            if let Some(states) = update.inference { for state in states.values() { assert!(!state.failed, "{}", state.status); } }
        }
        assert!(Instant::now() < deadline, "no armed production inference snapshot was published");
        thread::sleep(Duration::from_millis(5));
    }
}
fn capture(bridge: &RenderBridge, node: &str, path: &Path) -> image::RgbaImage {
    bridge.capture_png(node.to_owned(), path.to_owned()).unwrap().recv_timeout(TIMEOUT).unwrap().unwrap();
    image::open(path).unwrap().to_rgba8()
}
fn pause(bridge: &RenderBridge) {
    bridge.set_playback("pause").unwrap();
    wait_update(bridge, |u| u.status.as_deref() == Some("Playback: pause"));
}

fn run_model(model: &InferenceModelDescriptor, manifest: &Value, root: &Path, report: &mut Value) {
    let directory = root.join(&model.id);
    std::fs::create_dir_all(&directory).unwrap();
    let path = model_path(manifest, &model.id);
    let sha256 = manifest["models"].as_array().unwrap().iter().find(|entry| entry["id"] == model.id).unwrap()["sha256"].as_str().unwrap();
    let (mut bridge, configuration) = RenderBridge::initialize().expect("real hardware GPU prerequisite");
    let WGPUConfiguration::Manual { device, queue, adapter, .. } = &configuration else { panic!("must share Slint renderer device") };
    assert_eq!(adapter.get_info().backend, wgpu::Backend::Metal);
    assert_ne!(adapter.get_info().device_type, wgpu::DeviceType::Cpu);
    let graph = test_graph(model, None);
    let source = image::open(manifest["fixtures"]["image"]["path"].as_str().unwrap()).unwrap().to_rgba8();
    bridge.set_graph(graph.graph.clone(), images(&graph, &source, 1), false).unwrap();
    load(&bridge, &graph, &path, sha256);
    arm(&bridge, &graph.inference);
    bridge.set_playback("play").unwrap();
    let first = snapshot(&bridge, &graph.inference);
    assert_eq!(first.model_id, model.id);
    let first_stamp = first.stamp;
    pause(&bridge);
    let held = wait_renderer(&bridge, &graph);
    assert!(held.lease.is_some(), "published renderer must retain inference output pool lease");
    let retained_pixels = read_texture(device, queue, &held.texture);
    record_case(&bridge, &graph, model, "image", first, device, queue, &directory, report);
    let paused = capture(&bridge, &graph.renderer, &directory.join("paused.png"));
    thread::sleep(Duration::from_millis(150));
    assert_eq!(capture(&bridge, &graph.renderer, &directory.join("paused-again.png")), paused,
        "Pause must freeze actual renderer bytes");
    bridge.set_playback("play").unwrap();
    wait_update(&bridge, |u| u.texture.is_some());
    pause(&bridge);
    bridge.unload_inference(graph.inference.clone()).unwrap();
    wait_update(&bridge, |u| u.inference.as_ref().and_then(|s| s.get(&graph.inference)).is_some_and(|s| !s.attached && !s.loading));
    assert_eq!(read_texture(device, queue, &held.texture), retained_pixels, "consumer lease must survive unload");
    load(&bridge, &graph, &path, sha256);
    // Changing actual source data dirties the production image input. It does not
    // inject an inference result or manually publish a graph output.
    for (index, fixture) in ["alternateImage", "constantImage"].into_iter().enumerate() {
        let source = image::open(manifest["fixtures"][fixture]["path"].as_str().unwrap()).unwrap().to_rgba8();
        bridge.set_graph(graph.graph.clone(), images(&graph, &source, index as u64 + 2), false).unwrap();
        arm(&bridge, &graph.inference);
        bridge.set_playback("play").unwrap();
        let snap = snapshot(&bridge, &graph.inference);
        pause(&bridge);
        record_case(&bridge, &graph, model, fixture, snap, device, queue, &directory, report);
    }
    if model.id == "realesrgan-x4" {
        let small = image::imageops::resize(&source, source.width() / 2, source.height() / 2, image::imageops::FilterType::Nearest);
        for (key, name, input) in [(10, "shape-small", &small), (11, "shape-restored", &source)] {
            bridge.set_graph(graph.graph.clone(), images(&graph, input, key), false).unwrap();
            arm(&bridge, &graph.inference);
            bridge.set_playback("play").unwrap();
            let snap = snapshot(&bridge, &graph.inference);
            pause(&bridge);
            let rendered = capture(&bridge, &graph.renderer, &directory.join(format!("{name}-dimensions.png")));
            assert_eq!(rendered.dimensions(), (input.width() * 4, input.height() * 4));
            record_case(&bridge, &graph, model, name, snap, device, queue, &directory, report);
        }
    }
    // begin_project is the actual replacement-resource boundary. New SDK IDs and
    // a separately loaded session must not inherit the preceding project's model.
    let video = test_graph(model, Some(manifest["fixtures"]["video"]["path"].as_str().unwrap()));
    bridge.begin_project();
    bridge.set_graph(video.graph.clone(), BTreeMap::new(), false).unwrap();
    let prepared_video = wait_update(&bridge, |u| u.inference.as_ref().is_some_and(|s| s.get(&video.inference).is_none_or(|state| !state.attached && !state.loading))
        && u.status.as_ref().is_some_and(|s| s.contains("ffmpeg-")));
    let decoder = prepared_video.status.as_deref().unwrap();
    assert!(decoder.contains("ffmpeg-videotoolbox") && !decoder.contains("fallback"), "hardware-video is required: {decoder}");
    report["videoDecoder"][&model.id] = json!(decoder);
    bridge.select_preview(Some(video.renderer.clone())).unwrap();
    assert_eq!(read_texture(device, queue, &held.texture), retained_pixels, "consumer lease must survive project replacement");
    drop(held);
    load(&bridge, &video, &path, sha256);
    arm(&bridge, &video.inference);
    bridge.set_playback("play").unwrap();
    let snap = snapshot(&bridge, &video.inference);
    assert_ne!(snap.stamp.session_generation, first_stamp.session_generation, "replacement must create new session identity");
    pause(&bridge);
    let original = record_case(&bridge, &video, model, "video", snap, device, queue, &directory, report);
    let marker = |frame: &image::RgbaImage| {
        let pixel = frame.get_pixel(16, 16);
        let difference = i16::from(pixel[0]) - i16::from(pixel[2]);
        assert!(difference.abs() > 100, "video must expose its pinned red/blue frame-group marker");
        difference > 0
    };
    let first_group = marker(&original.0);
    thread::sleep(Duration::from_millis(150));
    assert_eq!(capture(&bridge, &video.renderer, &directory.join("video-paused.png")), original.1,
        "paused hardware video must retain identical Renderer pixels while wall time advances");
    let deadline = Instant::now() + TIMEOUT;
    let mut index = 0;
    loop {
        bridge.set_playback("play").unwrap();
        // Advance actual video playback, then arm a fresh paused-source request.
        // Throughput is not an acceptance condition.
        thread::sleep(Duration::from_millis(250));
        pause(&bridge);
        arm(&bridge, &video.inference);
        bridge.set_playback("play").unwrap();
        let snap = snapshot(&bridge, &video.inference);
        pause(&bridge);
        let pixels = read_texture(device, queue, &snap.source.texture);
        if marker(&pixels) != first_group {
            let updated = record_case(&bridge, &video, model, "video-updated", snap, device, queue, &directory, report);
            assert_ne!(updated.1.as_raw(), original.1.as_raw(), "distinct source frame must change Renderer bytes, not merely frame count");
            break;
        }
        index += 1;
        assert!(Instant::now() < deadline, "video never produced a second distinct decoded frame ({index} snapshots)");
    }
    let metrics = wait_update(&bridge, |u| u.inference_metrics.is_some()).inference_metrics.unwrap();
    assert!(metrics.accepted > 0 && metrics.completed > 0 && metrics.submitted > 0);
    assert!(metrics.device_submissions > 0 && metrics.compute_dispatches > 0);
    assert_eq!((metrics.failed, metrics.readback_bytes, metrics.upload_bytes, metrics.input_cpu_bytes, metrics.output_cpu_bytes), (0,0,0,0,0),
        "normal production inference must not transfer host pixels/tensors; diagnostic reads are deliberately separate");
    report["metrics"][&model.id] = serde_json::to_value(metrics).unwrap();
    report["transitions"][&model.id] = json!({"pauseFrozenPixels":true,"resume":true,"unloadReload":true,
        "replacementNewSession":true,"retainedConsumerSurvivesUnloadAndReplacement":true,"distinctVideoRendererContent":true,
        "sourceShapeSwitchRestore":model.id == "realesrgan-x4"});
}

fn wait_renderer(bridge: &RenderBridge, graph: &TestGraph) -> GpuOutputHandle {
    bridge.select_preview(Some(graph.renderer.clone())).unwrap();
    wait_update(bridge, |u| u.texture.is_some()).texture.unwrap()
}

fn map_buffer(device: &wgpu::Device, queue: &wgpu::Queue, source: &wgpu::Buffer) -> Vec<u8> {
    let staging = device.create_buffer(&wgpu::BufferDescriptor { label: Some("explicit E2E diagnostic readback"),
        size: source.size(), usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation:false });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(source, 0, &staging, 0, source.size());
    let submission = queue.submit([encoder.finish()]);
    let (sender, receiver) = mpsc::sync_channel(1);
    staging.map_async(wgpu::MapMode::Read, .., move |result| { let _ = sender.send(result); });
    device.poll(wgpu::PollType::Wait { submission_index:Some(submission), timeout:Some(TIMEOUT) }).unwrap();
    receiver.recv_timeout(TIMEOUT).unwrap().unwrap();
    let mapped = staging.get_mapped_range(..).unwrap();
    let bytes = mapped.to_vec();
    drop(mapped);
    staging.unmap();
    bytes
}
fn read_texture(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> image::RgbaImage {
    let (width, height) = (texture.width(), texture.height());
    let row = (width * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor { label:Some("explicit E2E texture snapshot"),
        size:u64::from(row) * u64::from(height), usage:wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC, mapped_at_creation:false });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(wgpu::TexelCopyTextureInfo { texture, mip_level:0, origin:wgpu::Origin3d::ZERO, aspect:wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo { buffer:&buffer, layout:wgpu::TexelCopyBufferLayout { offset:0, bytes_per_row:Some(row), rows_per_image:Some(height) } },
        texture.size());
    queue.submit([encoder.finish()]);
    let padded = map_buffer(device, queue, &buffer);
    let bytes = padded.chunks_exact(row as usize).flat_map(|r| r[..width as usize * 4].iter().copied()).collect();
    image::RgbaImage::from_raw(width, height, bytes).unwrap()
}

fn record_case(bridge: &RenderBridge, graph: &TestGraph, model: &InferenceModelDescriptor, input: &str,
    snapshot: InferenceDiagnosticSnapshot, device: &wgpu::Device, queue: &wgpu::Queue,
    directory: &Path, report: &mut Value) -> (image::RgbaImage, image::RgbaImage) {
    assert_eq!(snapshot.source_node_id, graph.source);
    let source = read_texture(device, queue, &snapshot.source.texture);
    let source_path = directory.join(format!("{input}-source.png"));
    source.save(&source_path).unwrap();
    let raw_path = directory.join(format!("{input}-raw.f32"));
    std::fs::write(&raw_path, map_buffer(device, queue, &snapshot.raw)).unwrap();
    let input_path = directory.join(format!("{input}-input.f32"));
    std::fs::write(&input_path, map_buffer(device, queue, &snapshot.input)).unwrap();
    let output = read_texture(device, queue, &snapshot.output.texture);
    let output_path = directory.join(format!("{input}-output.png"));
    output.save(&output_path).unwrap();
    let renderer_path = directory.join(format!("{input}-renderer.png"));
    let renderer = capture(bridge, &graph.renderer, &renderer_path);
    assert_eq!(renderer, output, "Renderer must display the actual accepted inference snapshot (stable video frame groups required)");
    let detections = snapshot.detections.as_ref().map(|b| {
        let path = directory.join(format!("{input}-detections.bin"));
        std::fs::write(&path, map_buffer(device, queue, b)).unwrap(); path
    });
    let depth_range = snapshot.depth_range.as_ref().map(|b| {
        let bytes = map_buffer(device, queue, b);
        bytes.chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect::<Vec<_>>()
    });
    let logits = snapshot.logits.as_ref().map(|buffer| {
        let path = directory.join(format!("{input}-logits.f32"));
        std::fs::write(&path, map_buffer(device, queue, buffer)).unwrap(); path
    });
    report["cases"].as_array_mut().unwrap().push(json!({"modelId":model.id,"input":input,"productionPassed":true,
        "sourceDimensions":[source.width(),source.height()],"dimensions":[renderer.width(),renderer.height()],
        "rawShape":snapshot.raw_shape,"rawDtype":format!("{:?}",snapshot.raw_dtype),"rawStorage":"Float32 export; original dtype retained",
        "stamp":snapshot.stamp,"depthRange":depth_range,"logitsShape":snapshot.logits_shape,
        "artifacts":{"source":source_path,"renderer":renderer_path,"output":output_path,"raw":raw_path,"input":input_path,"detections":detections,"logits":logits}}));
    save_json(&directory.join(format!("{input}.json")), report["cases"].as_array().unwrap().last().unwrap());
    (source, renderer)
}

fn reference_case(case: &mut Value, manifest: &Value, root: &Path) {
    use open_quartz::inference::{ImageTaskPreprocessing, ImageTaskResize, TensorElementType};
    use open_quartz::inference::diagnostic_reference::{prepare_reference_model, semantic_parity};
    let id = case["modelId"].as_str().unwrap().to_owned();
    let model = inference_models().into_iter().find(|m| m.id == id).unwrap();
    let source = image::open(case["artifacts"]["source"].as_str().unwrap()).unwrap().to_rgba8();
    let actual = image::open(case["artifacts"]["renderer"].as_str().unwrap()).unwrap().to_rgba8();
    let original_bytes = std::fs::read(model_path(manifest, &id)).unwrap();
    let mut cpu = InferenceSession::from_memory(&original_bytes).unwrap();
    let config = open_quartz::inference::InferenceConfig { model_id:id.clone(), task:model.task,
        target_size:640, score_threshold:0.25, iou_threshold:0.45, policy:cpu.plan().requested_policy };
    let contract = open_quartz::inference::validate_image_task_session(&cpu, &config).unwrap();
    assert_eq!(case["rawDtype"], format!("{:?}", contract.output.element_type), "original selected dtype must remain exact");
    // Task metadata always belongs to the pinned original artifact, not the
    // diagnostic graph with explicit stable normalization/logit outputs.
    let reference = prepare_reference_model(&original_bytes).unwrap();
    let pinned_hash = manifest["models"].as_array().unwrap().iter().find(|m|m["id"]==id).unwrap()["sha256"].as_str().unwrap();
    assert_eq!(reference.source_sha256,pinned_hash,"reference must retain pinned original artifact identity");
    case["referenceArithmetic"] = json!({"contract":"user-approved-numerically-stable",
        "normalizationOracle":"centered population variance and affine arithmetic in Float64; activation Cast Float32",
        "normalizedNodes":reference.normalized_nodes,"otherOriginalNodesAndWeightsPreserved":true,
        "sourceArtifactUnmodified":true,"sourceSha256":reference.source_sha256,
        "oracleSha256":reference.oracle_sha256,"diagnosticLogitsOutput":reference.logits_output});
    if !reference.normalized_nodes.is_empty() {
        cpu = InferenceSession::from_memory(&reference.bytes).unwrap();
    }
    // This input is computed independently on CPU from the actual, same-request
    // GPU source snapshot. The reference never consumes a GPU-preprocessed tensor.
    let (input, shape) = match contract.resize {
        ImageTaskResize::Letterbox(side) => {
            let input = open_quartz::inference::letterbox_preprocess(source.as_raw(), source.width(), source.height(), side).unwrap();
            (input.tensor, input.shape.into_iter().map(|x| x as i64).collect::<Vec<_>>())
        }
        resize => {
            let image = match resize { ImageTaskResize::Square(side) => nearest(&source, side, side), _ => source.clone() };
            let tensor = match contract.preprocessing {
                ImageTaskPreprocessing::RgbZeroToOne => open_quartz::inference::rgba_to_chw(image.as_raw(), image.width(), image.height()).unwrap(),
                ImageTaskPreprocessing::BgrImageNetMeanStd => open_quartz::inference::midas_preprocess(image.as_raw(), image.width(), image.height()).unwrap(),
                ImageTaskPreprocessing::Bt601LuminanceZeroToOne => image.pixels().map(|p| {
                    let r = p[0] as f32 / 255.0; let g = p[1] as f32 / 255.0; let b = p[2] as f32 / 255.0;
                    0.299 * r + 0.587 * g + 0.114 * b
                }).collect(),
            };
            let channels = if contract.preprocessing == ImageTaskPreprocessing::Bt601LuminanceZeroToOne { 1 } else { 3 };
            (tensor, vec![1, channels, image.height() as i64, image.width() as i64])
        }
    };
    let bytes = std::fs::read(case["artifacts"]["raw"].as_str().unwrap()).unwrap();
    assert_eq!(bytes.len() % 4, 0);
    let gpu: Vec<f32> = bytes.chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect();
    let mut gpu_shape: Vec<i64> = serde_json::from_value(case["rawShape"].clone()).unwrap();
    let gpu = if id == "super-resolution-3x" {
        assert_eq!(gpu_shape, [1,9,224,224], "SR3 diagnostic is pre-PixelShuffle convolution");
        assert_eq!(gpu.len(), 9 * 224 * 224);
        let mut shuffled = vec![0.0; 672 * 672];
        for y in 0..672 { for x in 0..672 {
            shuffled[y * 672 + x] = gpu[((y % 3) * 3 + x % 3) * 224 * 224 + (y / 3) * 224 + x / 3];
        } }
        gpu_shape = vec![1,1,672,672];
        shuffled
    } else { gpu };
    let mut cpu_logits = None;
    let (expected_raw, cpu_shape) = if contract.output.element_type == TensorElementType::Uint8 {
        let logits_name = reference.logits_output.as_deref().expect("original semantic pre-ArgMax tensor required");
        let logits_shape: Vec<i64> = serde_json::from_value(case["logitsShape"].clone()).expect("same-execution GPU logits shape required");
        assert_eq!(logits_shape.len(),4);
        assert_eq!([logits_shape[0],logits_shape[2],logits_shape[3]],gpu_shape.as_slice());
        assert_eq!(logits_shape[1],open_quartz::inference::CITYSCAPES_PALETTE.len() as i64);
        let mut semantic_cpu = InferenceSession::from_memory(&reference.bytes).unwrap();
        let ((values, shape), logits) = semantic_cpu.run_u8_with_logits(input, shape,
            &contract.output.name, logits_name, &logits_shape).unwrap();
        cpu_logits = Some(logits);
        (values.into_iter().map(f32::from).collect::<Vec<_>>(), shape)
    } else {
        let stamp = RequestStamp { request_id:1, session_generation:cpu.generation() };
        let mut result = cpu.invoke_with_output_shapes(stamp,
            vec![NamedTensor { name:contract.input.name.clone(), data:input, shape }],
            std::slice::from_ref(&contract.output.name), &[(contract.output.name.as_str(), gpu_shape.as_slice())]).unwrap();
        assert_eq!(result.stamp, stamp);
        assert_eq!(result.outputs.len(), 1);
        let output = result.outputs.remove(0);
        assert_eq!(output.name, contract.output.name);
        (output.data, output.shape)
    };
    assert_eq!(cpu_shape, gpu_shape, "exact original selected shape");
    let raw = float_parity(&expected_raw, &gpu, contract.output.element_type == TensorElementType::Uint8);
    case["parity"]["raw"] = raw.clone();
    let mut adjudicated_classes = None;
    if id == "yolo26n-sem" {
        let logits = cpu_logits.expect("semantic CPU logits required");
        let bytes = std::fs::read(case["artifacts"]["logits"].as_str().expect("same-execution GPU logits artifact required")).unwrap();
        assert_eq!(bytes.len()%4,0);
        let gpu_logits: Vec<f32> = bytes.chunks_exact(4).map(|b|f32::from_le_bytes(b.try_into().unwrap())).collect();
        let semantic = semantic_parity(&expected_raw,&gpu,&logits.data,&gpu_logits,&logits.shape,ABS,REL).unwrap();
        case["parity"]["semantic"] = serde_json::to_value(&semantic).unwrap();
        assert!(semantic.passed,"{} {} semantic numerical adjudication: {}",id,case["input"],case["parity"]["semantic"]);
        // Preserve exact-ID evidence. Presentation changes are allowed only at
        // individually reported, numerically adjudicated original class IDs.
        let mut classes: Vec<u8> = expected_raw.iter().map(|&v|v as u8).collect();
        for difference in &semantic.class_differences {
            let [batch,y,x] = difference.coordinate;
            let index = (batch * logits.shape[2] as usize + y) * logits.shape[3] as usize + x;
            classes[index] = difference.gpu_class as u8;
        }
        adjudicated_classes = Some(classes);
    } else {
        assert_eq!(raw["passed"], true, "{} {} raw tensor parity: {raw}", id, case["input"]);
    }
    if id == "yolov8n" {
        let path = case["artifacts"]["detections"].as_str().expect("YOLO structured ROI diagnostic required");
        let diagnostic = detection_parity(&std::fs::read(path).unwrap(), &expected_raw, source.width(), source.height());
        case["parity"]["detections"] = diagnostic.clone();
        assert_eq!(diagnostic["passed"], true, "exact YOLO count/order/class and numerical box/score parity required");
        if case["input"] == "image" { assert!(diagnostic["count"].as_u64().unwrap() > 0, "fixture must exercise nonempty YOLO detections"); }
        if case["input"] == "constantImage" { assert_eq!(diagnostic["count"], 0, "flat-image empty detection boundary"); }
    }
    if id == "midas-small" {
        let minimum = expected_raw.iter().copied().fold(f32::INFINITY, f32::min);
        let maximum = expected_raw.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let actual_range: Vec<f32> = serde_json::from_value(case["depthRange"].clone()).expect("resident depth range diagnostic required");
        let parity = float_parity(&[minimum,maximum], &actual_range, false);
        case["parity"]["depthRange"] = parity.clone();
        assert_eq!(parity["passed"], true);
        if case["input"] == "image" { assert!(maximum - minimum > f32::EPSILON, "real nonconstant depth evidence"); }
    }
    if id == "yolo26n-sem" {
        let labels: std::collections::BTreeSet<u8> = gpu.iter().map(|&v| {
            assert!(v.is_finite() && v >= 0.0 && v <= 255.0 && v.fract() == 0.0); v as u8
        }).collect();
        case["semantics"]["classLabels"] = json!(labels);
        if case["input"] == "image" { assert!(labels.len() > 1, "fixture must exercise multiple exact class IDs"); }
    }
    drop(gpu);
    drop(expected_raw);
    let (expected_rgba,expected_width,expected_height) = if let Some(classes) = adjudicated_classes {
        let (width,height) = source.dimensions();
        let scale = (640.0 / width as f32).min(640.0 / height as f32);
        let crop_x = ((640.0-width as f32*scale)*0.5).round() as usize;
        let crop_y = ((640.0-height as f32*scale)*0.5).round() as usize;
        let crop_width = (width as f32*scale).round().max(1.0) as usize;
        let crop_height = (height as f32*scale).round().max(1.0) as usize;
        assert!(crop_x+crop_width<=640 && crop_y+crop_height<=640);
        let mut rgba = source.as_raw().clone();
        for y in 0..height as usize { for x in 0..width as usize {
            let category = classes[(crop_y+y*crop_height/height as usize)*640+crop_x+x*crop_width/width as usize] as usize;
            let color = open_quartz::inference::CITYSCAPES_PALETTE[category];
            let offset = (y*width as usize+x)*4;
            for channel in 0..3 { rgba[offset+channel] = ((u16::from(rgba[offset+channel])+u16::from(color[channel]))/2) as u8; }
        } }
        case["parity"]["rendererClassSelection"] = json!({"criterion":"original CPU IDs plus individually adjudicated GPU boundary IDs",
            "boundaryAccepted":case["parity"]["semantic"]["boundaryAccepted"],"exactIdPassed":case["parity"]["semantic"]["exactIdPassed"]});
        (rgba,width,height)
    } else {
        let expected = open_quartz::inference::run_image_task(&mut cpu, model.task, &id,
            source.as_raw(), source.width(), source.height(), 640, 0.25, 0.45).unwrap();
        (expected.rgba,expected.width,expected.height)
    };
    assert_eq!(actual.dimensions(), (expected_width,expected_height));
    let parity = byte_parity(&expected_rgba, actual.as_raw());
    case["parity"]["renderer"] = parity.clone();
    case["independentCpuPreprocess"] = json!(true);
    assert_eq!(parity["passed"], true, "{} {} independent source-to-renderer parity: {parity}", id, case["input"]);
    if model.task == open_quartz::InferenceTask::BackgroundRemoval {
        let minimum = actual.pixels().map(|p| p[3]).min().unwrap();
        let maximum = actual.pixels().map(|p| p[3]).max().unwrap();
        case["semantics"]["alphaRange"] = json!([minimum,maximum]);
        if case["input"] == "image" { assert!(minimum < 128 && maximum > 128, "mask must preserve foreground and remove background"); }
    }
    let expected_path = root.join(&id).join(format!("{}-cpu-reference.png", case["input"].as_str().unwrap()));
    image::save_buffer(&expected_path, &expected_rgba, expected_width, expected_height, image::ColorType::Rgba8).unwrap();
    case["artifacts"]["cpuReference"] = json!(expected_path);
    case["referencePassed"] = json!(true);
    save_json(&root.join(&id).join(format!("{}.json",case["input"].as_str().unwrap())), case);
}

fn nearest(source: &image::RgbaImage, width: u32, height: u32) -> image::RgbaImage {
    image::RgbaImage::from_fn(width, height, |x,y| *source.get_pixel(x * source.width() / width, y * source.height() / height))
}
fn float_parity(cpu: &[f32], gpu: &[f32], exact: bool) -> Value {
    let mut mismatches = 0usize;
    let mut maximum = 0.0f64;
    let mut scaled = 0.0f64;
    for (&a,&b) in cpu.iter().zip(gpu) {
        if !a.is_finite() || !b.is_finite() { mismatches += 1; continue; }
        let error = (f64::from(a) - f64::from(b)).abs();
        maximum = maximum.max(error);
        scaled = scaled.max(error / (ABS + REL * f64::from(a).abs()));
        mismatches += usize::from(if exact { a != b } else { error > ABS + REL * f64::from(a).abs() });
    }
    json!({"passed":!cpu.is_empty() && cpu.len() == gpu.len() && mismatches == 0,
        "cpuValues":cpu.len(),"gpuValues":gpu.len(),"mismatchedValues":mismatches,
        "maximumAbsoluteError":maximum,"maximumScaledError":scaled,"exactClassIds":exact,
        "absoluteTolerance":ABS,"relativeTolerance":REL})
}
fn byte_parity(cpu: &[u8], gpu: &[u8]) -> Value {
    let mut maximum = 0u8;
    let mut sum = 0u64;
    for (&a,&b) in cpu.iter().zip(gpu) { let error = a.abs_diff(b); maximum = maximum.max(error); sum += u64::from(error); }
    let mean = sum as f64 / cpu.len().max(1) as f64;
    json!({"passed":!cpu.is_empty() && cpu.len() == gpu.len() && maximum <= 3 && mean <= 0.5,
        "checkedBytes":cpu.len(),"maxByteDifference":maximum,"meanByteDifference":mean,
        "maxAllowed":3,"meanAllowed":0.5,"boundaryExceptions":false})
}
fn detection_parity(bytes: &[u8], raw: &[f32], width: u32, height: u32) -> Value {
    assert!(bytes.len() >= 16 && (bytes.len()-16) % 32 == 0);
    let word = |offset:usize| u32::from_le_bytes(bytes[offset..offset+4].try_into().unwrap());
    let count = word(0) as usize;
    assert!(count <= 8400 && count <= (bytes.len()-16)/32);
    assert_eq!([word(4),word(8),word(12)],[0,0,0]);
    let scale = (640.0 / width as f32).min(640.0 / height as f32);
    let expected = open_quartz::inference::detect_postprocess(raw, width, height, scale,
        (640.0 - (width as f32 * scale).round()) * 0.5,
        (640.0 - (height as f32 * scale).round()) * 0.5, 0.25, 0.45);
    let mut passed = count == expected.len();
    let mut records = Vec::new();
    let mut prior: Option<(f32,u32)> = None;
    for index in 0..count {
        let offset = 16 + index * 32;
        let bbox: [f32;4] = std::array::from_fn(|axis| f32::from_bits(word(offset + axis * 4)));
        let score = f32::from_bits(word(offset+16));
        let class = word(offset+20);
        let anchor = word(offset+24);
        passed &= word(offset+28) == 0 && class < 80 && anchor < 8400;
        if let Some((s,a)) = prior { passed &= score < s || (score == s && anchor > a); }
        prior = Some((score,anchor));
        if let Some(reference) = expected.get(index) {
            passed &= class as usize == reference.class_id;
            passed &= float_parity(&reference.bbox, &bbox, false)["passed"] == true;
            passed &= float_parity(&[reference.score], &[score], false)["passed"] == true;
        }
        records.push(json!({"bbox":bbox,"score":score,"classId":class,"originalIndex":anchor}));
    }
    json!({"passed":passed,"count":count,"cpuCount":expected.len(),"exactClassOrderCount":true,
        "gpuRecords":records,"cpuDetections":expected,"absoluteTolerance":ABS,"relativeTolerance":REL})
}
