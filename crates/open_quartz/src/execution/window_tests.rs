use super::*;

fn window_accept(p: &mut FramePipeline, demand: FrameDemand) {
    match p.submit_window(demand) {
        FrameAdmission::Accepted => {}
        FrameAdmission::Rejected { error, .. } => panic!("{error}"),
        FrameAdmission::WouldBlock(_) => panic!("unexpected window backpressure"),
    }
}

#[test]
fn bounded_window_preserves_order_pixels_and_final_epoch_after_cancellation() {
    pollster::block_on(async {
        let mut p = pipeline(device().await, &animated_graph());
        p.configure_window(FrameWindowConfig {
            max_span_ns: 1_000_000_000,
            ..Default::default()
        })
        .unwrap();
        for (id, time) in [(1, 0), (2, 250_000_000), (3, 500_000_000)] {
            let request = demand(&p, id, 1, time, vec![]);
            window_accept(&mut p, request);
        }
        let fourth = demand(&p, 4, 1, 750_000_000, vec![]);
        assert!(matches!(
            p.submit_window(fourth),
            FrameAdmission::WouldBlock(_)
        ));
        assert!(!p.window_needs_refill());
        let first = rendered(complete(&mut p));
        assert_eq!(pixels(&p, &first), [0, 0, 255, 255].repeat(4));
        let second = rendered(complete(&mut p));
        assert_eq!(pixels(&p, &second), [64, 0, 191, 255].repeat(4));
        assert!(p.window_needs_refill());
        assert!(p.cancel_pending());
        let cancel = p.poll_completion().unwrap();
        assert_eq!(cancel.request_id, 3);
        assert!(matches!(cancel.outcome, FrameOutcome::Cancelled));
        drop(first);
        drop(second);
        p.backend()
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let final_target = demand(&p, 1, 2, 125_000_000, vec![]);
        window_accept(&mut p, final_target);
        let final_frame = rendered(complete(&mut p));
        assert_eq!(
            final_frame.identity.content_stamp.as_ref().unwrap().epoch,
            2
        );
        assert_eq!(pixels(&p, &final_frame), [32, 0, 223, 255].repeat(4));
        assert!(p.poll_completion().is_none());
    });
}

#[test]
fn window_rejects_impossible_working_set_and_reclaims_only_after_consumer_release() {
    pollster::block_on(async {
        let mut p = pipeline(device().await, &animated_graph());
        assert!(p
            .configure_window(FrameWindowConfig {
                low_water: 3,
                ..Default::default()
            })
            .is_err());
        p.configure_window(FrameWindowConfig {
            max_bytes: 16,
            ..Default::default()
        })
        .unwrap();
        let request = demand(&p, 1, 1, 0, vec![]);
        assert!(matches!(
            p.submit_window(request),
            FrameAdmission::Rejected { .. }
        ));
        assert!(!p.has_in_flight());
        p.configure_window(FrameWindowConfig {
            max_span_ns: 1_000_000_000,
            ..Default::default()
        })
        .unwrap();
        let mut held = Vec::new();
        for (id, time) in [(1, 0), (2, 250_000_000), (3, 500_000_000)] {
            let request = demand(&p, id, 1, time, vec![]);
            window_accept(&mut p, request);
            held.push(rendered(complete(&mut p)));
        }
        let old = pixels(&p, &held[0]);
        let request = demand(&p, 4, 1, 750_000_000, vec![]);
        window_accept(&mut p, request);
        for _ in 0..5 {
            p.progress();
        }
        assert!(p.poll_completion().is_none());
        assert_eq!(pixels(&p, &held[0]), old);
        drop(held.remove(0));
        let fourth = rendered(complete(&mut p));
        assert_eq!(pixels(&p, &fourth), [191, 0, 64, 255].repeat(4));
        assert_eq!(pixels(&p, &held[0]), [64, 0, 191, 255].repeat(4));
    });
}

#[cfg(all(feature = "native-video", target_os = "macos"))]
#[test]
fn video_window_prefetches_future_pts_and_cut_prewarm_survives_graph_commit() {
    pollster::block_on(async {
        let mut graph: Graph =
            serde_json::from_value(json!({"nodes":[input_node("image")],"edges":[]})).unwrap();
        graph.nodes[0].data.input_mode = Some(crate::project::model::InputMode::Video);
        let mut p = pipeline(device().await, &graph);
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../oq-media/tests/source-time.mp4");
        let config = NativeVideoConfig {
            kind: NativeVideoSourceKind::File,
            source: path.to_string_lossy().into_owned(),
            looping: false,
            playback_rate: 1.0,
        };
        p.attach_video("image", config.clone()).unwrap();
        p.configure_window(FrameWindowConfig {
            max_span_ns: 1_000_000_000,
            ..Default::default()
        })
        .unwrap();
        for (id, time) in [(1, 0), (2, 100_000_000), (3, 200_000_000)] {
            let mut request = demand(&p, id, 1, time, vec![]);
            request.output_node_id = "image".into();
            window_accept(&mut p, request);
        }
        let first = rendered(complete(&mut p));
        let first_pixels = pixels(&p, &first);
        let second = rendered(complete(&mut p));
        assert_eq!(
            second.identity.content_stamp.as_ref().unwrap().media_pts_ns,
            Some(100_000_000)
        );
        assert_ne!(pixels(&p, &second), first_pixels);
        let third = rendered(complete(&mut p));
        assert_eq!(
            third.identity.content_stamp.as_ref().unwrap().media_pts_ns,
            Some(200_000_000)
        );
        assert_eq!(p.video_metrics("image").unwrap().cpu_copy_bytes, 0);
        let mut prepared = p.prepare_graph(&graph).unwrap();
        let next_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/white_64x64.mp4");
        prepared
            .stage_video(
                "image",
                NativeVideoConfig {
                    source: next_path.to_string_lossy().into_owned(),
                    ..config
                },
                false,
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !prepared.prewarm_videos(0, 2).unwrap() {
            p.progress();
            assert!(Instant::now() < deadline, "cut preparation stalled");
            std::thread::yield_now();
        }
        assert_eq!(pixels(&p, &first), first_pixels);
        drop(second);
        drop(third);
        p.commit_graph(prepared).unwrap();
        let mut request = demand(&p, 1, 2, 0, vec![]);
        request.output_node_id = "image".into();
        window_accept(&mut p, request);
        let cut = rendered(complete(&mut p));
        assert_eq!(
            cut.identity.content_stamp.as_ref().unwrap().media_pts_ns,
            Some(0)
        );
        assert_ne!(pixels(&p, &cut), first_pixels);
        assert!(pixels(&p, &cut)
            .chunks_exact(4)
            .all(|pixel| pixel[..3].iter().all(|value| *value >= 250)));
        assert_eq!(pixels(&p, &first), first_pixels);
        assert_eq!(p.video_metrics("image").unwrap().cpu_copy_bytes, 0);
    });
}

#[test]
fn native_feedback_pause_discards_source_lookahead_without_advancing_unpresented_history() {
    pollster::block_on(async {
        let graph: Graph = serde_json::from_value(json!({"nodes":[shader("output",
            "@fragment fn main(@location(0) uv:vec2f)->@location(0) vec4f {return textureSample(previousFrame,previousFrameSampler,uv)+vec4f(0.25,0.0,0.0,1.0);}", json!([]))],"edges":[]})).unwrap();
        let (mut player, _) = crate::NativePlayer::new().await.unwrap();
        player.set_graph_snapshot(&graph).unwrap();
        player.start_playback().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while player.render_next().unwrap().width == 0 {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        let first = player.try_stable_preview("output").unwrap().unwrap();
        player.pause().unwrap();
        first.ready.wait(Duration::from_secs(5)).unwrap();
        let red = player
            .backend()
            .read_texture_rgba(&first.output().texture, 2, 2)
            .await
            .unwrap();
        assert!(red.chunks_exact(4).all(|pixel| pixel == [64, 0, 0, 255]));
        player.resume().unwrap();
        while player.render_next().unwrap().width == 0 {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        let second = player.try_stable_preview("output").unwrap().unwrap();
        second.ready.wait(Duration::from_secs(5)).unwrap();
        let next = player
            .backend()
            .read_texture_rgba(&second.output().texture, 2, 2)
            .await
            .unwrap();
        assert!(next.chunks_exact(4).all(|pixel| pixel == [128, 0, 0, 255]));
        assert_eq!(
            player
                .backend()
                .read_texture_rgba(&first.output().texture, 2, 2)
                .await
                .unwrap(),
            red
        );
    });
}

#[test]
fn sharing_pipelines_reject_impossible_working_sets_and_progress_after_slow_session_disposes() {
    pollster::block_on(async {
        let first_device = device().await;
        let budget = crate::resources::FrameBudget::new(7, 7 * 16).unwrap();
        first_device
            .backend()
            .set_frame_budget(budget.clone())
            .unwrap();
        let mut first = pipeline(first_device, &animated_graph());
        first.configure_window(Default::default()).unwrap();
        let request = demand(&first, 1, 1, 0, vec![]);
        window_accept(&mut first, request);
        let held = rendered(complete(&mut first));
        let saved = pixels(&first, &held);
        let second_device = device().await;
        second_device
            .backend()
            .set_frame_budget(budget.clone())
            .unwrap();
        let mut second = pipeline(second_device, &animated_graph());
        second.configure_window(Default::default()).unwrap();
        let request = demand(&second, 1, 1, 0, vec![]);
        window_accept(&mut second, request);
        let other = rendered(complete(&mut second));
        let request = demand(&second, 2, 1, 16_666_667, vec![]);
        window_accept(&mut second, request);
        let another = rendered(complete(&mut second));
        assert_eq!(budget.usage().0, 6);
        assert_eq!(pixels(&first, &held), saved);
        first.dispose().unwrap();
        assert!(
            budget.usage().0 >= 4,
            "held native output must retain its allocation"
        );
        assert_eq!(pixels(&first, &held), saved);
        drop(held);
        first
            .backend()
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        drop((other, another));
        let request = demand(&second, 3, 1, 33_333_334, vec![]);
        window_accept(&mut second, request);
        let frame = rendered(complete(&mut second));
        assert_eq!(frame.identity.evaluation_stamp.as_ref().unwrap().frame, 3);
        assert!(budget.usage().0 <= 7);
        second.dispose().unwrap();
        drop(frame);
        second
            .backend()
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        assert_eq!(budget.usage(), (0, 0));
    });
}

#[cfg(target_os = "macos")]
#[test]
fn prefill_presentation_delay_preserves_inflight_inference_stamp_and_finishes_once() {
    pollster::block_on(async {
        let graph: Graph = serde_json::from_value(json!({"nodes":[
            shader("source", "@fragment fn main()->@location(0) vec4f {return vec4f(0.0,0.0,1.0,1.0);}", json!([])),
            {"id":"model","type":"inference","position":{"x":0,"y":0},"data":{"type":"inference","label":"SR3","shaderCode":"","inputs":texture_port(),"outputs":[{"id":"out","label":"image","dataType":"sampler2D","direction":"output"}],"uniforms":{},"inferenceModelId":"super-resolution-3x","inferenceTask":"super-resolution"}},
            {"id":"output","type":"renderer","position":{"x":0,"y":0},"data":{"type":"renderer","label":"output","shaderCode":"","inputs":texture_port(),"outputs":[],"uniforms":{}}}
        ],"edges":[{"id":"a","source":"source","sourceHandle":"out","target":"model","targetHandle":"in"},{"id":"b","source":"model","sourceHandle":"out","target":"output","targetHandle":"in"}]})).unwrap();
        let mut p = pipeline(device().await, &graph);
        let backend = p.backend();
        let model = oq_inference::gpu_model::GpuModel::from_onnx(
            &crate::inference::resident_test_model::sr3_model(),
        )
        .unwrap();
        let session = oq_inference::gpu::GpuImageSession::prepare(
            backend.device.clone(),
            backend.queue.clone(),
            model,
        )
        .unwrap();
        p.load_resident_inference(
            "model".into(),
            crate::inference::ResidentInferenceSession::Sr3(session),
            InferenceConfig {
                model_id: "super-resolution-3x".into(),
                task: crate::inference::InferenceTask::SuperResolution,
                target_size: 224,
                policy: crate::inference::ExecutionPolicy::resident(),
                ..Default::default()
            },
        )
        .unwrap();
        p.configure_window(Default::default()).unwrap();
        let request = demand(&p, 1, 1, 0, vec![]);
        let original = request.input.stamp.clone();
        window_accept(&mut p, request);
        p.progress();
        assert!(p.pending.as_ref().unwrap().waiting_inference.is_some());
        p.delay_window_deadlines(100_000_000);
        let completion = complete(&mut p);
        assert_eq!(
            completion.stamp, original,
            "presentation delay must not rewrite accepted content identity"
        );
        assert_eq!(
            p.presentation_deadline(&completion.stamp),
            original.deadline_ns + 100_000_000
        );
        let frame = rendered(completion);
        assert!(pixels(&p, &frame)
            .chunks_exact(4)
            .all(|pixel| pixel == [0, 0, 255, 255]));
        assert_eq!(frame.identity.evaluation_stamp.as_ref(), Some(&original));
        p.progress();
        assert!(p.poll_completion().is_none());
        assert_eq!(p.inference_metrics().accepted, 1);
    });
}
