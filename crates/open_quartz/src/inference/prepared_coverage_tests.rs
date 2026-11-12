use super::*;
use crate::inference::{InferenceTask, PlacementPolicy, ResidencyPolicy};
#[cfg(all(feature = "native-runtime", target_os = "macos"))]
use crate::inference::tasks::ResidentImage;

fn config() -> InferenceConfig {
    InferenceConfig {
        model_id: "super-resolution-3x".to_owned(),
        task: InferenceTask::SuperResolution,
        target_size: 224,
        policy: ExecutionPolicy::resident(),
        ..Default::default()
    }
}
#[test]
fn unsupported_policy_wins_over_missing_model_before_io() {
    let missing = Path::new("missing-inference-model-for-policy-test.onnx");
    for policy in [
        ExecutionPolicy::default(),
        ExecutionPolicy {
            residency: ResidencyPolicy::RequireResident,
            ..ExecutionPolicy::default()
        },
        ExecutionPolicy {
            placement: PlacementPolicy::AcceleratorOnly,
            ..ExecutionPolicy::default()
        },
        ExecutionPolicy {
            fallback: FallbackPolicy::AllowCpu,
            ..ExecutionPolicy::resident()
        },
    ] {
        let mut config = config();
        config.policy = policy;
        let error = PreparedInferenceModel::open_with_policy(missing, config)
            .err()
            .unwrap();
        assert!(error.contains("hardware-resident"), "{error}");
        assert!(!error.contains("Cannot read"), "{error}");
    }
    let error = PreparedInferenceModel::open(missing, config(), true)
        .err()
        .unwrap();
    assert!(error.contains("DirectML"), "{error}");
    assert!(!error.contains("Cannot read"), "{error}");
}
#[test]
fn model_preparation_preserves_source_facts_without_runtime_or_gpu() {
    let bytes = crate::inference::resident_test_model::sr3_model();
    let prepared = PreparedInferenceModel::from_memory(&bytes, config()).unwrap();
    let info = prepared.info();
    assert_eq!(info.backend, "metal");
    assert_eq!(info.input_names, ["input"]);
    assert_eq!(info.output_names, ["output"]);
    assert_eq!(
        info.inputs[0].dimensions[0],
        TensorDimension::Symbol("batch_size".to_owned())
    );
    assert_eq!(
        info.source_interface,
        Some(oq_inference::inspect_onnx(&bytes).unwrap())
    );
    assert_eq!(info.source_interface.as_ref().unwrap().inputs.len(), 9);
    assert_eq!(info.overridable_initializers.len(), 8);
    assert_eq!(info.artifact.byte_length, bytes.len() as u64);
    assert_eq!(info.artifact.sha256.len(), 64);
    assert_eq!(
        info.model.producer_name.as_deref(),
        Some("open-quartz-resident-test")
    );
    assert_eq!(
        info.plan.provider_placement,
        oq_inference::ProviderPlacement::Gpu
    );
    assert_eq!(info.plan.transfer_path, oq_inference::TransferPath::Device);
    assert_eq!(
        prepared.task_contract().unwrap().adapter,
        crate::inference::ImageTaskAdapter::LuminanceSuperResolution3x
    );
    assert_eq!(info.input_values[0].name, "input");
    assert_eq!(
        info.input_values[0].kind,
        ValueKind::Tensor {
            element_type: TensorElementType::Float32,
            dimensions: info.inputs[0].dimensions.clone(),
        }
    );
    assert_eq!(info.input_values[0].optionality, ValueOptionality::Unknown);
    assert_eq!(
        info.overridable_initializers[0].optionality,
        ValueOptionality::Optional
    );
    assert!(info
        .overridable_initializers
        .iter()
        .all(|initializer| initializer.overridable_initializer));
    assert_eq!(
        info.image_interface.expected_input_layout,
        "NCHW FP32 BT.601 luminance [1,1,224,224]"
    );
    assert!(info.image_interface.runnable);
    assert!(info.image_interface.issues.is_empty());
    assert_eq!(info.opset_imports.as_ref().unwrap()[0].version, 10);
    let (model, owned_config) = prepared.into_parts();
    assert_eq!(model.model_plan().source_byte_length, bytes.len() as u64);
    assert_eq!(owned_config.policy, ExecutionPolicy::resident());
}
#[test]
fn unsupported_tasks_and_invalid_graphs_never_become_reference_fallbacks() {
    let missing = Path::new("missing.onnx");
    let mut unsupported = config();
    unsupported.model_id = "unknown-model".to_owned();
    let error = PreparedInferenceModel::open_with_policy(missing, unsupported)
        .err()
        .unwrap();
    assert!(error.contains("seven supported"), "{error}");
    assert!(PreparedInferenceModel::from_memory(b"not ONNX", config()).is_err());
    assert!(PreparedInferenceModel::from_memory(
        include_bytes!("../../tests/data/image_identity.onnx"),
        config()
    )
    .is_err());
}

#[test]
fn resident_catalog_requires_exact_model_task_pairing() {
    for (id, task) in [
        ("super-resolution-3x", InferenceTask::SuperResolution),
        ("realesrgan-x4", InferenceTask::SuperResolution),
        ("u2netp", InferenceTask::BackgroundRemoval),
        ("modnet", InferenceTask::BackgroundRemoval),
        ("midas-small", InferenceTask::DepthEstimation),
        ("yolov8n", InferenceTask::Detection),
        ("yolo26n-sem", InferenceTask::Segmentation),
    ] {
        let mut config = config();
        config.model_id = id.to_owned();
        config.task = task;
        validate_resident_config(&config).unwrap();
        config.task = InferenceTask::Generic;
        assert!(validate_resident_config(&config)
            .unwrap_err()
            .contains("requires task"));
    }
}

#[cfg(target_os = "macos")]
#[test]
fn prepared_catalog_attaches_to_native_player_and_retains_alpha_pixels_after_close() {
    use serde_json::json;
    use std::time::{Duration, Instant};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mask.onnx");
    std::fs::write(&path, crate::inference::resident_test_model::mask_model()).unwrap();
    let config = InferenceConfig {
        model_id: "modnet".into(),
        task: InferenceTask::BackgroundRemoval,
        policy: ExecutionPolicy::resident(),
        target_size: 512,
        ..Default::default()
    };
    assert!(PreparedInferenceModel::open_with_policy(
        &directory.path().join("absent.onnx"),
        config.clone()
    )
    .is_err());
    let graph: crate::Graph = serde_json::from_value(json!({"nodes":[
        {"id":"source","type":"input","position":{"x":0,"y":0},"data":{"type":"input","label":"source","shaderCode":"","inputs":[],"outputs":[{"id":"out","label":"image","dataType":"sampler2D","direction":"output"}],"uniforms":{},"inputMode":"image","inputDataType":"sampler2D"}},
        {"id":"model","type":"inference","position":{"x":0,"y":0},"data":{"type":"inference","label":"mask","shaderCode":"","inputs":[{"id":"in","label":"image","dataType":"sampler2D","direction":"input"}],"outputs":[{"id":"out","label":"mask","dataType":"sampler2D","direction":"output"}],"uniforms":{},"inferenceModelId":"modnet","inferenceTask":"background-removal"}},
        {"id":"renderer","type":"renderer","position":{"x":0,"y":0},"data":{"type":"renderer","label":"output","shaderCode":"","inputs":[{"id":"in","label":"image","dataType":"sampler2D","direction":"input"}],"outputs":[],"uniforms":{}}}
    ],"edges":[{"id":"a","source":"source","sourceHandle":"out","target":"model","targetHandle":"in"},{"id":"b","source":"model","sourceHandle":"out","target":"renderer","targetHandle":"in"}]})).unwrap();
    let (mut player, _) = pollster::block_on(crate::player::NativePlayer::new()).unwrap();
    player.set_graph_snapshot(&graph).unwrap();
    player
        .upload_image("source", &[255, 0, 0, 255].repeat(4), 2, 2)
        .unwrap();
    let prepared = PreparedInferenceModel::open(&path, config, false).unwrap();
    prepared.attach(&mut player, "model".into()).unwrap();
    player.start_playback().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while player.render_next().unwrap().width == 0 {
        assert!(Instant::now() < deadline);
    }
    let held = player.try_stable_preview("renderer").unwrap().unwrap();
    held.ready.wait(Duration::from_secs(5)).unwrap();
    let backend = player.backend();
    assert_eq!(
        pollster::block_on(backend.read_texture_rgba(&held.output().texture, 2, 2)).unwrap(),
        [255, 0, 0, 128].repeat(4)
    );
    let metrics = player.inference_metrics();
    assert_eq!(metrics.accepted, 1);
    assert_eq!(
        metrics.readback_bytes
            + metrics.upload_bytes
            + metrics.input_cpu_bytes
            + metrics.output_cpu_bytes,
        0
    );
    player.close().unwrap();
    assert_eq!(
        pollster::block_on(backend.read_texture_rgba(&held.output().texture, 2, 2)).unwrap(),
        [255, 0, 0, 128].repeat(4)
    );
}
#[cfg(target_os = "macos")]
#[test]
fn resident_persistent_working_set_rejects_insufficient_budget_before_submit() {
    pollster::block_on(async {
        let (render, _) = crate::rendering::RenderDevice::new().await.unwrap();
        let backend = render.backend();
        let model = oq_inference::gpu_model::GpuModel::from_onnx(
            &crate::inference::resident_test_model::sr3_model(),
        ).unwrap();
        let mut session = ResidentInferenceSession::Sr3(
            GpuImageSession::prepare(backend.device.clone(), backend.queue.clone(), model).unwrap(),
        );
        assert!(session.set_frame_budget(crate::resources::FrameBudget::new(2, 1).unwrap()).is_err());
        assert_eq!(session.counters().submissions, 0);
    });
}

#[cfg(target_os = "macos")]
#[test]
fn resident_output_credits_follow_live_slots_and_gpu_completion_across_resizes() {
    pollster::block_on(async {
        let (render, _) = crate::rendering::RenderDevice::new().await.unwrap();
        let backend = render.backend();
        let model = oq_inference::gpu_model::GpuModel::from_onnx(
            &crate::inference::resident_test_model::sr3_model(),
        ).unwrap();

        let budget = crate::resources::FrameBudget::new(3, u64::MAX).unwrap();
        let mut session = ResidentInferenceSession::Sr3(
            GpuImageSession::prepare(backend.device.clone(), backend.queue.clone(), model).unwrap(),
        );
        session.set_frame_budget(budget.clone()).unwrap();
        assert!(budget.usage().1 > 0, "persistent scratch and preprocessing buffers are charged");

        let texture = |width, height, rgba: [u8; 4]| {
            let texture = backend.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("resident budget lifecycle input"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            backend.queue.write_texture(texture.as_image_copy(), &rgba.repeat((width * height) as usize),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 4), rows_per_image: Some(height) },
                texture.size());
            texture
        };
        let first_texture = texture(1, 1, [0, 0, 255, 255]);
        let second_texture = texture(2, 1, [0, 255, 0, 255]);
        let third_texture = texture(3, 1, [255, 0, 0, 255]);
        let fourth_texture = texture(4, 1, [255, 255, 255, 255]);
        let submit = |session: &mut ResidentInferenceSession, texture: &wgpu::Texture, width, height| {
            match session.submit_texture(texture, width, height).unwrap() {
                ResidentImage::Gpu(output) => output,
                _ => unreachable!("SR3 always produces its resident GPU image"),
            }
        };
        let first = submit(&mut session, &first_texture, 1, 1);
        let first_pixels = backend.read_texture_rgba(first.texture(), 3, 3).await.unwrap();
        assert!(first_pixels.chunks_exact(4).all(|pixel| pixel.iter().zip([0u8, 0, 255, 255]).all(|(&actual, expected)| actual.abs_diff(expected) <= 1)));
        let second = submit(&mut session, &second_texture, 2, 1);
        let third = submit(&mut session, &third_texture, 3, 1);
        assert_eq!(budget.usage().0, 3);
        assert!(matches!(session.submit_texture(&fourth_texture, 4, 1), Err(oq_inference::gpu::GpuError::Capacity)));
        assert_eq!(backend.read_texture_rgba(first.texture(), 3, 3).await.unwrap(), first_pixels,
            "shape transitions do not overwrite a retained output");
        drop(first);
        let fourth = submit(&mut session, &fourth_texture, 4, 1);
        for (output, width, expected) in [(&second, 6, [0u8, 255, 0, 255]), (&third, 9, [255, 0, 0, 255]), (&fourth, 12, [255, 255, 255, 255])] {
            assert_eq!((output.width(), output.height()), (width, 3));
            let pixels = backend.read_texture_rgba(output.texture(), width, 3).await.unwrap();
            assert!(pixels.chunks_exact(4).all(|pixel| pixel.iter().zip(expected).all(|(&actual, expected)| actual.abs_diff(expected) <= 1)), "resident output {width}x3 changed source color: {pixels:?}");
        }
        drop(session);
        backend.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert_eq!(budget.usage().0, 3, "output leases retain slot credits after session drop");
        let second_pixels = backend.read_texture_rgba(second.texture(), 6, 3).await.unwrap();
        assert!(second_pixels.chunks_exact(4).all(|pixel| pixel.iter().zip([0u8, 255, 0, 255]).all(|(&actual, expected)| actual.abs_diff(expected) <= 1)), "retained green output changed after session close: {second_pixels:?}");
        drop((second, third, fourth));
        backend.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert_eq!(budget.usage(), (0, 0));
    });
}

#[cfg(all(target_os = "macos", feature = "native-video"))]
#[test]
fn catalog_video_keeps_advancing_pixels_with_lookahead_and_retained_preview() {
    use serde_json::json;
    use std::{collections::BTreeSet, io::Write, process::{Command, Stdio}, time::{Duration, Instant}};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("catalog-progress.mp4");
    let mut encoder = Command::new(crate::media::native::find_ffmpeg().unwrap())
        .env_remove("DYLD_LIBRARY_PATH").env_remove("DYLD_FALLBACK_LIBRARY_PATH")
        .args(["-hide_banner", "-loglevel", "error", "-f", "rawvideo", "-pixel_format", "rgb24",
            "-video_size", "64x64", "-framerate", "10", "-i", "-", "-frames:v", "32",
            "-c:v", "libx264", "-qp", "0", "-pix_fmt", "yuv420p", "-y"])
        .arg(&path).stdin(Stdio::piped()).spawn().unwrap();
    let mut input = encoder.stdin.take().unwrap();
    for index in 0..32u8 {
        input.write_all(&[index * 7, 255 - index * 7, 32].repeat(64 * 64)).unwrap();
    }
    drop(input);
    assert!(encoder.wait().unwrap().success());

    let graph: crate::Graph = serde_json::from_value(json!({"nodes":[
        {"id":"source","type":"input","position":{"x":0,"y":0},"data":{"type":"input","label":"video","shaderCode":"","inputs":[],"outputs":[{"id":"out","label":"image","dataType":"sampler2D","direction":"output"}],"uniforms":{},"inputMode":"video","inputDataType":"sampler2D"}},
        {"id":"model","type":"inference","position":{"x":0,"y":0},"data":{"type":"inference","label":"mask","shaderCode":"","inputs":[{"id":"in","label":"image","dataType":"sampler2D","direction":"input"}],"outputs":[{"id":"out","label":"mask","dataType":"sampler2D","direction":"output"}],"uniforms":{},"inferenceModelId":"modnet","inferenceTask":"background-removal"}},
        {"id":"renderer","type":"renderer","position":{"x":0,"y":0},"data":{"type":"renderer","label":"output","shaderCode":"","inputs":[{"id":"in","label":"image","dataType":"sampler2D","direction":"input"}],"outputs":[],"uniforms":{}}}
    ],"edges":[{"id":"a","source":"source","sourceHandle":"out","target":"model","targetHandle":"in"},{"id":"b","source":"model","sourceHandle":"out","target":"renderer","targetHandle":"in"}]})).unwrap();
    let (mut player, _) = pollster::block_on(crate::player::NativePlayer::new()).unwrap();
    player.set_graph_snapshot(&graph).unwrap();
    let info = player.attach_video("source", crate::media::native::NativeVideoConfig {
        kind: crate::media::native::NativeVideoSourceKind::File,
        source: path.to_string_lossy().into_owned(), looping: true, playback_rate: 1.0,
    }).unwrap();
    assert_eq!(info.decoder, "ffmpeg-videotoolbox");
    PreparedInferenceModel::from_memory(&crate::inference::resident_test_model::mask_model(), InferenceConfig {
        model_id: "modnet".into(), task: InferenceTask::BackgroundRemoval,
        policy: ExecutionPolicy::resident(), target_size: 512, ..Default::default()
    }).unwrap().attach(&mut player, "model".into()).unwrap();
    player.start_playback().unwrap();
    let backend = player.backend();
    let mut held = None;
    let mut first_pixels = None;
    let mut pts = BTreeSet::new();
    let mut colors = BTreeSet::new();
    let deadline = Instant::now() + Duration::from_secs(15);
    while pts.len() < 12 || colors.len() < 12 {
        let presented = player.render_next().unwrap();
        if presented.width != 0 {
            let frame = player.try_stable_preview("renderer").unwrap().unwrap();
            let selected = frame.identity.content_stamp.as_ref().unwrap().media_pts_ns.unwrap();
            if pts.insert(selected) {
                frame.ready.wait(Duration::from_secs(5)).unwrap();
                let pixels = pollster::block_on(backend.read_texture_rgba(&frame.output().texture, 64, 64)).unwrap();
                assert!(pixels.chunks_exact(4).all(|pixel| pixel[3] == 128));
                colors.insert(pixels[..3].to_vec());
                if held.is_none() { first_pixels = Some(pixels); held = Some(frame); }
            }
        }
        assert!(Instant::now() < deadline, "catalog stalled: selected PTS {pts:?}, changed colors {colors:?}, accepted {}", player.inference_metrics().accepted);
        std::thread::yield_now();
    }
    let held = held.unwrap();
    assert_eq!(pollster::block_on(backend.read_texture_rgba(&held.output().texture, 64, 64)).unwrap(), first_pixels.unwrap(), "retained first preview was overwritten");
    let metrics = player.inference_metrics();
    assert!(metrics.accepted >= 12);
    assert_eq!(metrics.failed, 0);
    assert_eq!(metrics.readback_bytes + metrics.upload_bytes + metrics.input_cpu_bytes + metrics.output_cpu_bytes, 0);
    assert_eq!(player.video_metrics("source").unwrap().cpu_copy_bytes, 0);
    player.close().unwrap();
}

#[cfg(target_os = "macos")]
#[test]
fn catalog_output_credit_survives_session_and_consumer_drop_until_gpu_completion() {
    pollster::block_on(async {
        let (render, _) = crate::rendering::RenderDevice::new().await.unwrap();
        let backend = render.backend();
        let model = CatalogModel::from_onnx(&crate::inference::resident_test_model::mask_model(), "modnet", 512, 512).unwrap();
        let mut session = ResidentInferenceSession::Catalog(CatalogGpuSession::prepare(
            backend.device.clone(), backend.queue.clone(), model, Default::default()).unwrap());
        let budget = crate::resources::FrameBudget::new(2, u64::MAX).unwrap();
        session.set_frame_budget(budget.clone()).unwrap();
        session.prepare_output_frame(2, 2).unwrap();
        assert_eq!(budget.usage().0, 2);
        let source = backend.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("catalog physical ownership source"),
            size: wgpu::Extent3d { width: 2, height: 2, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[],
        });
        backend.queue.write_texture(source.as_image_copy(), &[0, 255, 0, 255].repeat(4),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(8), rows_per_image: Some(2) }, source.size());
        let ResidentImage::Catalog(output) = session.submit_texture(&source, 2, 2).unwrap() else { unreachable!() };
        assert_eq!(backend.read_texture_rgba(output.texture(), 2, 2).await.unwrap(), [0, 255, 0, 128].repeat(4));
        // The next submission has no CPU wait or readback before all owners leave.
        drop(output);
        let ResidentImage::Catalog(output) = session.submit_texture(&source, 2, 2).unwrap() else { unreachable!() };
        drop(output);
        drop(session);
        assert_eq!(budget.usage().0, 1, "submitted GPU work retains its physical slot charge after cancellation");
        backend.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert_eq!(budget.usage(), (0, 0));
    });
}
