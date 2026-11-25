#[cfg(test)]
mod tests {
    use super::super::*;
    use std::time::Instant;
    fn wait(bridge: &PreviewBridge, ticket: u64) -> PreviewUpdate {
        let limit = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(update) = bridge.take_update() {
                if update.ticket == ticket {
                    return update;
                }
            }
            assert!(Instant::now() < limit, "preview request did not complete");
            thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    #[ignore = "requires native hardware video playback"]
    fn shared_pipeline_displays_real_video_black_gaps_and_retains_previous_frame_after_cut() {
        crate::media::configure_runtime().unwrap();
        let (mut bridge, _configuration, _) = PreviewBridge::new().unwrap();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/white_h264_high_64x64.mp4");
        let asset = crate::media::probe(&path).unwrap();
        let mut request = PreviewRequest { managed_output: false, managed_view: false, layers: Vec::new(),
            ticket: 1,
            revision: 4,
            frame: 0,
            source_ns: 0,
            width: 128,
            height: 64,
            asset: Some(asset),
            transform: ClipTransform::default(),
            pipeline: ClipPipeline::default(),
        };
        bridge.request(request.clone()).unwrap();
        let first = wait(&bridge, 1);
        assert_eq!(first.cpu_copy_bytes, 0);
        assert_eq!(first.actual_pts_ns, Some(0));
        let image = first.image.unwrap();
        let pixels = pollster::block_on(bridge.backend.read_texture_rgba(
            &image.output().texture,
            128,
            64,
        ))
        .unwrap();
        assert!(pixels.chunks_exact(4).enumerate().all(|(index, p)| {
            if index % 128 < 32 || index % 128 >= 96 {
                p == [0, 0, 0, 255]
            } else {
                p[0] > 240 && p[1] > 240 && p[2] > 240 && p[3] == 255
            }
        }));
        request.ticket = 2;
        request.frame = 15;
        request.asset = None;
        bridge.request(request.clone()).unwrap();
        let gap = wait(&bridge, 2).image.unwrap();
        let black = pollster::block_on(bridge.backend.read_texture_rgba(
            &gap.output().texture,
            128,
            64,
        ))
        .unwrap();
        assert!(black.chunks_exact(4).all(|p| p == [0, 0, 0, 255]));
        assert_eq!(
            pollster::block_on(
                bridge
                    .backend
                    .read_texture_rgba(&image.output().texture, 128, 64)
            )
            .unwrap(),
            pixels
        );
        drop((image, gap));
        request.ticket = 3;
        bridge.request(request).unwrap();
        assert!(wait(&bridge, 3).image.is_ok());
        bridge.shutdown();
    }
    #[test]
    #[ignore = "requires native hardware video playback"]
    fn obsolete_seek_is_never_published_as_newer_request_and_offline_is_an_error() {
        crate::media::configure_runtime().unwrap();
        let (mut bridge, _configuration, _) = PreviewBridge::new().unwrap();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/oq-media/tests/source-time.mp4");
        let mut asset = crate::media::probe(&path).unwrap();
        let mut request = PreviewRequest { managed_output: false, managed_view: false, layers: Vec::new(),
            ticket: 1,
            revision: 7,
            frame: 0,
            source_ns: 1_200_000_000,
            width: 64,
            height: 64,
            asset: Some(asset.clone()),
            transform: ClipTransform::default(),
            pipeline: ClipPipeline::default(),
        };
        bridge.request(request.clone()).unwrap();
        request.ticket = 2;
        request.source_ns = 100_000_000;
        request.frame = 3;
        bridge.request(request.clone()).unwrap();
        let selected = wait(&bridge, 2);
        assert_eq!((selected.revision, selected.frame), (7, 3));
        assert_eq!(selected.actual_pts_ns, Some(100_000_000));
        assert_eq!(selected.cpu_copy_bytes, 0);
        drop(selected);
        asset.online = false;
        request.ticket = 3;
        request.asset = Some(asset);
        bridge.request(request).unwrap();
        assert!(wait(&bridge, 3).image.err().unwrap().contains("offline"));
        bridge.shutdown();
    }
    fn render_request(pipeline: &mut FramePipeline, request: &PreviewRequest) -> StableFrame {
        let demand = FrameDemand {
            request_id: request.ticket,
            graph_revision: pipeline.revision(),
            output_node_id: OUTPUT_ID.into(),
            sources: Vec::new(),
            input: FrameEvaluationInput {
                stamp: FrameStamp {
                    epoch: 1,
                    frame: request.ticket,
                    timeline_ns: request.source_ns,
                    deadline_ns: 0,
                },
                previous_timeline_ns: request.source_ns,
                date: [0.0; 4],
                mouse: [0.0; 4],
                resolution: [request.width as f32, request.height as f32, 1.0],
            },
        };
        match pipeline.submit(demand) {
            FrameAdmission::Accepted => {}
            FrameAdmission::Rejected { error, .. } => panic!("{error}"),
            FrameAdmission::WouldBlock(_) => panic!("unexpected preview backpressure"),
        }
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            pipeline.progress();
            if let Some(completion) = pipeline.poll_completion() {
                return match completion.outcome {
                    FrameOutcome::Submitted(frame) => {
                        frame.ready.wait(Duration::from_secs(5)).unwrap();
                        frame
                    }
                    FrameOutcome::Failed(error) => panic!("{error}"),
                    FrameOutcome::Cancelled => panic!("preview was cancelled"),
                };
            }
            assert!(Instant::now() < deadline, "hardware preview did not complete");
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn pixels(pipeline: &FramePipeline, frame: &StableFrame) -> Vec<u8> {
        pollster::block_on(pipeline.backend().read_texture_rgba(
            &frame.output().texture, frame.image.width, frame.image.height,
        )).unwrap()
    }

    #[test]
    #[ignore = "requires native hardware video playback"]
    fn clip_effect_pixels_are_isolated_and_graph_edits_reuse_the_hardware_source() {
        crate::media::configure_runtime().unwrap();
        let render = pollster::block_on(RenderDevice::new()).unwrap().0;
        let mut pipeline = FramePipeline::from_render_device(
            render, PlaybackCapabilities { data_paths: vec![] },
        );
        let asset = crate::media::probe(&std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/white_h264_high_64x64.mp4")).unwrap();
        let mut request = PreviewRequest { managed_output: false, managed_view: false, layers: Vec::new(),
            ticket: 1, revision: 1, frame: 0, source_ns: 0,
            width: 128, height: 64, asset: Some(asset),
            transform: ClipTransform::default(), pipeline: ClipPipeline::default(),
        };
        let mut configured = None;
        assert!(configure(&mut pipeline, &mut configured, &request).unwrap().0);
        let original = render_request(&mut pipeline, &request);
        let original_pixels = pixels(&pipeline, &original);
        let original_metrics = pipeline.video_metrics(SOURCE_ID).unwrap();
        assert!(original_metrics.hardware_decode);
        assert!(original_metrics.uploaded_frames > 0);
        assert_eq!(original_metrics.cpu_copy_bytes, 0);

        let id = request.pipeline.add_serial(SOURCE_ID,
            "@group(0) @binding(2) var<uniform> strength:f32; @fragment fn main(@location(0) uv:vec2f)->@location(0) vec4f { let c=textureSample(image,imageSampler,uv); return vec4f(c.rgb*strength,1.0); }",
            "Exposure stage").unwrap();
        let mut edited = request.pipeline.graph().clone();
        edited.nodes.iter_mut().find(|node| node.id == id).unwrap()
            .data.uniforms.insert("strength".into(), serde_json::json!(0.25));
        request.pipeline.replace_graph(edited).unwrap();
        request.ticket += 1;
        assert!(configure(&mut pipeline, &mut configured, &request).unwrap().0);
        // Restaging VideoToolbox resets these counters; committing a new graph
        // must leave the exact live source in place before the next demand.
        assert_eq!(pipeline.video_metrics(SOURCE_ID).unwrap().uploaded_frames,
            original_metrics.uploaded_frames);
        let quarter = render_request(&mut pipeline, &request);
        assert_eq!(quarter.identity.content_stamp.as_ref().unwrap().media_pts_ns, Some(0));
        assert!(pixels(&pipeline, &quarter).chunks_exact(4).enumerate().all(|(index, p)| {
            if index % 128 < 32 || index % 128 >= 96 { p == [0, 0, 0, 255] }
            else { (61..=66).contains(&p[0]) && p[0] == p[1] && p[1] == p[2] && p[3] == 255 }
        }));
        drop(quarter);

        let mut edited = request.pipeline.graph().clone();
        let stage = edited.nodes.iter_mut().find(|node| node.id == id).unwrap();
        stage.position.x += 120.0;
        stage.data.label = "Renamed stage".into();
        request.pipeline.replace_graph(edited).unwrap();
        let revision = pipeline.revision();
        assert!(!configure(&mut pipeline, &mut configured, &request).unwrap().0);
        assert_eq!(pipeline.revision(), revision);

        let mut edited = request.pipeline.graph().clone();
        edited.nodes.iter_mut().find(|node| node.id == id).unwrap()
            .data.uniforms.insert("strength".into(), serde_json::json!(0.5));
        request.pipeline.replace_graph(edited).unwrap();
        let before = pipeline.video_metrics(SOURCE_ID).unwrap().uploaded_frames;
        request.ticket += 1;
        assert!(configure(&mut pipeline, &mut configured, &request).unwrap().0);
        assert_eq!(pipeline.video_metrics(SOURCE_ID).unwrap().uploaded_frames, before);
        let half = render_request(&mut pipeline, &request);
        assert!(pixels(&pipeline, &half).chunks_exact(4).enumerate().all(|(index, p)| {
            if index % 128 < 32 || index % 128 >= 96 { p == [0, 0, 0, 255] }
            else { (125..=130).contains(&p[0]) && p[0] == p[1] && p[1] == p[2] }
        }));

        // A different clip with the same asset and a default pipeline must not
        // inherit its neighbour's effect, nor mutate an already leased frame.
        request.pipeline = ClipPipeline::default();
        request.ticket += 1;
        let before = pipeline.video_metrics(SOURCE_ID).unwrap().uploaded_frames;
        assert!(configure(&mut pipeline, &mut configured, &request).unwrap().0);
        assert_eq!(pipeline.video_metrics(SOURCE_ID).unwrap().uploaded_frames, before);
        let clean = render_request(&mut pipeline, &request);
        assert_eq!(pixels(&pipeline, &clean), original_pixels);
        assert_eq!(pixels(&pipeline, &original), original_pixels);
        assert_eq!(pipeline.video_metrics(SOURCE_ID).unwrap().cpu_copy_bytes, 0);
        drop((half, clean));

        let before = pipeline.video_metrics(SOURCE_ID).unwrap().uploaded_frames;
        request.transform.exposure = -1.0;
        assert!(configure(&mut pipeline, &mut configured, &request).unwrap().0);
        assert_eq!(pipeline.video_metrics(SOURCE_ID).unwrap().uploaded_frames, before);

        // Both a fingerprint change at the same path and a different file path
        // are source changes, unlike any of the graph-only edits above.
        request.asset.as_mut().unwrap().fingerprint.push_str("-changed");
        assert!(configure(&mut pipeline, &mut configured, &request).unwrap().0);
        assert_eq!(pipeline.video_metrics(SOURCE_ID).unwrap().uploaded_frames, 0);
        request.ticket += 1;
        drop(render_request(&mut pipeline, &request));
        request.asset = Some(crate::media::probe(&std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/gray_h264_high_64x64.mp4")).unwrap());
        request.transform = ClipTransform::default();
        assert!(configure(&mut pipeline, &mut configured, &request).unwrap().0);
        assert_eq!(pipeline.video_metrics(SOURCE_ID).unwrap().uploaded_frames, 0);
        request.ticket += 1;
        let gray = render_request(&mut pipeline, &request);
        assert!(pixels(&pipeline, &gray).chunks_exact(4).enumerate().all(|(index, p)| {
            if index % 128 < 32 || index % 128 >= 96 { p == [0, 0, 0, 255] }
            else { (80..180).contains(&p[0]) && p[0].abs_diff(p[1]) <= 4 && p[0].abs_diff(p[2]) <= 4 }
        }));
    }

    #[test]
    #[ignore = "requires native hardware video playback"]
    fn effects_follow_native_size_and_contain_the_actual_upstream_aspect() {
        crate::media::configure_runtime().unwrap();
        let (mut bridge, _configuration, _) = PreviewBridge::new().unwrap();
        let asset = crate::media::probe(&std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/white_h264_high_64x64.mp4")).unwrap();
        let mut clip_pipeline = ClipPipeline::default();
        let id = clip_pipeline.add_serial(SOURCE_ID,
            "@fragment fn main(@location(0) uv:vec2f)->@location(0) vec4f { return textureSample(image,imageSampler,uv)*vec4f(0.0,1.0,0.0,1.0); }",
            "Green").unwrap();
        let mut request = PreviewRequest { managed_output: false, managed_view: false, layers: Vec::new(),
            ticket: 1, revision: 1, frame: 0, source_ns: 0,
            width: 128, height: 64, asset: Some(asset),
            transform: ClipTransform::default(), pipeline: clip_pipeline,
        };
        bridge.request(request.clone()).unwrap();
        let native = wait(&bridge, 1).image.unwrap();
        let native_pixels = pollster::block_on(bridge.backend.read_texture_rgba(
            &native.output().texture, 128, 64,
        )).unwrap();
        assert!(native_pixels.chunks_exact(4).enumerate().all(|(index, p)| {
            if index % 128 < 32 || index % 128 >= 96 { p == [0, 0, 0, 255] }
            else { p[0] == 0 && p[1] > 240 && p[2] == 0 && p[3] == 255 }
        }));
        let mut sized = request.pipeline.graph().clone();
        let effect = sized.nodes.iter_mut().find(|node| node.id == id).unwrap();
        effect.data.auto_size = Some(false);
        effect.data.width = Some(128);
        effect.data.height = Some(32);
        request.pipeline.replace_graph(sized).unwrap();
        request.ticket = 2;
        bridge.request(request).unwrap();
        let resized = wait(&bridge, 2).image.unwrap();
        let resized_pixels = pollster::block_on(bridge.backend.read_texture_rgba(
            &resized.output().texture, 128, 64,
        )).unwrap();
        assert!(resized_pixels.chunks_exact(4).enumerate().all(|(index, p)| {
            if index / 128 < 16 || index / 128 >= 48 { p == [0, 0, 0, 255] }
            else { p[0] == 0 && p[1] > 240 && p[2] == 0 && p[3] == 255 }
        }));
        bridge.shutdown();
    }

    #[test]
    fn gpu_candidate_preflight_reports_errors_without_replacing_the_live_preview() {
        let (mut bridge, _configuration, _) = PreviewBridge::new().unwrap();
        let mut request = PreviewRequest { managed_output: false, managed_view: false, layers: Vec::new(),
            ticket: 1, revision: 1, frame: 0, source_ns: 0,
            width: 128, height: 64, asset: None,
            transform: ClipTransform::default(), pipeline: ClipPipeline::default(),
        };
        bridge.request(request.clone()).unwrap();
        let previous = wait(&bridge, 1).image.unwrap();
        // This pipeline is model-valid; only the GPU owner's device limit
        // rejects its requested output dimensions during asynchronous preflight.
        let candidate = ClipPipeline::default();
        let oversized = bridge.backend.device.limits().max_texture_dimension_2d + 1;
        let failed = bridge.validate_pipeline(candidate, oversized, 64).unwrap();
        assert!(failed.recv_timeout(Duration::from_secs(15)).unwrap().is_err());
        let valid = bridge.validate_pipeline(ClipPipeline::default(), 128, 64).unwrap();
        assert!(valid.recv_timeout(Duration::from_secs(15)).unwrap().is_ok());
        request.ticket = 2;
        bridge.request(request).unwrap();
        let next = wait(&bridge, 2).image.unwrap();
        assert_eq!(next.identity.graph_revision, previous.identity.graph_revision);
        let previous_pixels = pollster::block_on(bridge.backend.read_texture_rgba(
            &previous.output().texture, 128, 64,
        )).unwrap();
        let next_pixels = pollster::block_on(bridge.backend.read_texture_rgba(
            &next.output().texture, 128, 64,
        )).unwrap();
        assert_eq!(next_pixels, previous_pixels);
        bridge.shutdown();
    }
    #[test]
    #[ignore = "requires native hardware video playback"]
    fn worker_play_snapshot_reports_progress_and_exact_seek_cancels_its_window() {
        crate::media::configure_runtime().unwrap();
        let (mut bridge, _configuration, _) = PreviewBridge::new().unwrap();
        let asset = crate::media::probe(&std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/white_h264_high_64x64.mp4")).unwrap();
        let mut document = crate::document::EditorDocument::default();
        document.data.width = 64; document.data.height = 64;
        let id = asset.id.clone();
        document.add_asset(asset.clone()).unwrap();
        document.add_clip(&id).unwrap();
        bridge.play(PlaybackRequest { ticket: 10, revision: document.revision,
            start_frame: 0, project: document.data.clone(), #[cfg(any(target_os = "macos", windows))] audio_clock: None }).unwrap();
        let first = wait(&bridge, 10);
        assert_eq!(first.frame, 0);
        assert_eq!(first.actual_pts_ns, Some(0));
        drop(first);
        let deadline = Instant::now() + Duration::from_secs(5);
        let initial = loop {
            let progress = bridge.progress();
            if progress.ticket == 10 && progress.presented > 0 { break progress; }
            assert!(Instant::now() < deadline);
            thread::yield_now();
        };
        assert!(initial.submitted >= 2, "playback must prefill before its first presentation");
        assert!(initial.queue_peak <= initial.window_high_water);
        bridge.request(PreviewRequest { managed_output: false, managed_view: false, layers: Vec::new(), ticket: 11, revision: document.revision,
            frame: 3, source_ns: 100_000_000, width: 64, height: 64, asset: Some(asset),
            transform: ClipTransform::default(), pipeline: ClipPipeline::default() }).unwrap();
        let exact = wait(&bridge, 11);
        assert_eq!((exact.frame, exact.actual_pts_ns), (3, Some(100_000_000)));
        assert!(exact.epoch > initial.epoch);
        assert_eq!(exact.cpu_copy_bytes, 0);
        drop(exact);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let progress = bridge.progress();
            if progress.ticket == 11 && !progress.preparing {
                assert!(!progress.playing);
                assert_eq!(progress.frame, 3);
                assert!(progress.queue_peak <= 3);
                break;
            }
            assert!(Instant::now() < deadline);
            thread::yield_now();
        }
        assert!(bridge.take_update().is_none(), "cancelled play cannot republish after exact completion");
        bridge.shutdown();
    }
    #[test]
    #[ignore = "requires native hardware video playback"]
    #[cfg(any(target_os = "macos", windows))]
    fn playback_crosses_gap_and_cut_epochs_and_retires_the_exact_last_frame() {
        crate::media::configure_runtime().unwrap();
        let (mut bridge, _configuration, _) = PreviewBridge::new().unwrap();
        let asset = crate::media::probe(&std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/white_h264_high_64x64.mp4")).unwrap();
        let mut document = crate::document::EditorDocument::default();
        document.data.width = 64; document.data.height = 64;
        let id = asset.id.clone();
        document.add_asset(asset).unwrap(); document.add_clip(&id).unwrap();
        document.data.fps = Rational::new(10, 1).unwrap();
        document.data.tracks[0].clips[0].duration = 2;
        let mut next = document.data.tracks[0].clips[0].clone();
        next.id = "next-clip".into(); next.timeline_start = 4;
        next.transform.exposure = -1.0;
        document.data.tracks[0].clips.push(next);
        let clock = Arc::new(crate::audio_output::AudioClock::default());
        bridge.play(PlaybackRequest { ticket: 1, revision: document.revision,
            start_frame: 0, project: document.data, audio_clock: Some(clock.clone()) }).unwrap();
        let first = wait(&bridge, 1);
        let first_epoch = first.epoch;
        assert!(first.image.is_ok(), "{}", first.image.err().unwrap());
        drop(first);
        // Hold the media clock at the first gap frame until its GPU pixels
        // have been observed, regardless of decoder and runner scheduling.
        let samples_per_frame = u64::from(crate::document::SAMPLE_RATE) / 10;
        clock.set_played_samples(2 * samples_per_frame);
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut saw_gap = false;
        loop {
            if let Some(update) = bridge.take_update() {
                let frame = update.image.unwrap();
                assert_eq!((frame.image.width, frame.image.height), (64, 64));
                if (2..4).contains(&update.frame) {
                    assert_eq!(update.actual_pts_ns, None);
                    assert!(update.epoch > first_epoch);
                    assert!(pollster::block_on(bridge.backend.read_texture_rgba(
                        &frame.output().texture, 64, 64)).unwrap().chunks_exact(4).all(|pixel| pixel == [0, 0, 0, 255]));
                    saw_gap = true;
                    clock.set_played_samples(5 * samples_per_frame);
                }
                if update.frame == 5 {
                    assert_eq!(update.actual_pts_ns, Some(100_000_000));
                    assert!(update.epoch >= first_epoch + 2);
                    assert!(pollster::block_on(bridge.backend.read_texture_rgba(
                        &frame.output().texture, 64, 64)).unwrap().chunks_exact(4).all(|pixel| (125..=130).contains(&pixel[0]) && pixel[3] == 255));
                    break;
                }
            }
            assert!(Instant::now() < deadline, "worker did not retire the exact final cut frame: {}", serde_json::to_string(&bridge.progress()).unwrap());
            thread::sleep(Duration::from_millis(2));
        }
        assert!(saw_gap);
        bridge.shutdown();
    }
    #[test]
    #[ignore = "requires native hardware video playback"]
    fn two_native_sources_keep_independent_pts_and_blend_in_linear_light() {
        crate::media::configure_runtime().unwrap();
        let (mut bridge, _, _) = PreviewBridge::new().unwrap();
        let asset = crate::media::probe(&std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/white_h264_high_64x64.mp4")).unwrap();
        let tint = |color: &str| {
            let mut pipeline = ClipPipeline::default();
            let code = format!("@fragment fn main(@location(0) uv:vec2f)->@location(0) vec4f {{
                let c = textureSample(image,imageSampler,uv);
                return vec4f(vec3f({color}) * c.r, 1.0);
            }}");
            pipeline.add_serial(SOURCE_ID, &code, "Tint").unwrap();
            pipeline
        };
        let mut document = crate::document::EditorDocument::default();
        document.data.width = 128;
        document.data.height = 64;
        document.add_asset(asset.clone()).unwrap();
        let background = document.add_clip(&asset.id).unwrap();
        let foreground_track = document.add_track(document::DEFAULT_TRACK).unwrap();
        let foreground = document.place_clip(&asset.id, &foreground_track, Some(0)).unwrap();
        let start_pts = asset.start_pts.unwrap();
        let two_tenths = i64::from(asset.time_base.denominator) /
            (i64::from(asset.time_base.numerator) * 5);
        document.set_timing(&background, 0, 5, start_pts + two_tenths).unwrap();
        document.set_timing(&foreground, 0, 5, start_pts).unwrap();
        document.set_clip_pipeline(&background, tint("0.0, 0.0, 1.0"), document.revision).unwrap();
        document.set_clip_pipeline(&foreground, tint("1.0, 0.0, 0.0"), document.revision).unwrap();
        document.set_transform(&background, ClipTransform { scale: 2.0, ..Default::default() }, document.revision).unwrap();
        document.set_transform(&foreground, ClipTransform { opacity: 0.5, ..Default::default() }, document.revision).unwrap();

        let exact = |bridge: &PreviewBridge, document: &crate::document::EditorDocument, ticket| {
            bridge.request(sequence_request(&document.data, ticket, document.revision, 0, |_| None, None).unwrap()).unwrap();
            wait(bridge, ticket)
        };
        let read = |frame: &StableFrame| pollster::block_on(bridge.backend.read_texture_rgba(
            &frame.output().texture, 128, 64)).unwrap();
        let sample = |pixels: &[u8], x: usize| {
            let offset = (32 * 128 + x) * 4;
            <[u8; 4]>::try_from(&pixels[offset..offset + 4]).unwrap()
        };

        let update = exact(&bridge, &document, 1);
        assert_eq!(update.layer_pts_ns, [("Video 2".into(), Some(0)), ("Video 1".into(), Some(200_000_000))]);
        assert_eq!(update.cpu_copy_bytes, 0);
        let frame = update.image.unwrap();
        let pixels = read(&frame);
        assert_eq!(sample(&pixels, 20), [0, 0, 255, 255], "contain padding reveals the lower layer");
        let [r, g, b, a] = sample(&pixels, 64);
        assert!((185..=190).contains(&r) && (185..=190).contains(&b) && g == 0 && a == 255, "{r}, {g}, {b}, {a}");

        bridge.play(PlaybackRequest { ticket: 2, revision: document.revision,
            start_frame: 0, project: document.data.clone(), #[cfg(any(target_os = "macos", windows))] audio_clock: None }).unwrap();
        let played = wait(&bridge, 2);
        assert_eq!(played.frame, 0);
        assert_eq!(read(&played.image.unwrap()), pixels);

        document.reorder_track(document::DEFAULT_TRACK, &foreground_track, false).unwrap();
        let reordered = exact(&bridge, &document, 3);
        assert_eq!(sample(&read(&reordered.image.unwrap()), 64), [0, 0, 255, 255]);
        assert_eq!(read(&frame), pixels, "later composition must preserve the displayed lease");
        bridge.shutdown();
    }

}
