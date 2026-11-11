#![cfg(feature = "native-video")]

use std::env;
use std::fs;
use std::process::{self, Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::evaluation::EngineEvent;
use crate::media::native::{find_ffmpeg, NativeVideoSourceKind, NativeVideoConfig};
use crate::project::model::InputMode;

use super::*;

fn planned_dirty_node_count(host: &mut NativePlayer, input: &mut SessionFrameInput) -> u32 {
    host.pipeline.session.drain_events();
    host.pipeline.session.evaluate(&FrameEvaluationInput {
        stamp: FrameStamp { epoch: 1, frame: input.now_ns, timeline_ns: input.now_ns, deadline_ns: input.now_ns },
        previous_timeline_ns: input.now_ns.saturating_sub(16_666_667),
        date: input.date, mouse: input.mouse, resolution: input.resolution,
    }).unwrap();
    input.now_ns += 16_666_667;
    host.pipeline.session.drain_commands();
    let counts: Vec<_> = host.pipeline.session
        .drain_events()
        .into_iter()
        .filter_map(|event| match event {
            EngineEvent::FramePlanned { dirty_node_count, .. } => Some(dirty_node_count),
            _ => None,
        })
        .collect();
    assert_eq!(counts.len(), 1, "session must plan exactly one frame");
    counts[0]
}

#[test]
fn video_to_image_same_id_drops_decoder_and_preserves_image_textures() {
    let Ok(ffmpeg) = find_ffmpeg() else {
        return;
    };
    let directory = env::temp_dir().join(format!(
        "open-quartz-video-lifecycle-{}-{}",
        process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("source.mp4");
    let status = Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=64x64:r=10",
            "-frames:v",
            "8",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-y",
        ])
        .arg(&path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "failed to generate the video fixture");

    let nodes: Vec<_> = [("source", "video"), ("unchanged", "image")]
        .into_iter()
        .map(|(id, mode)| {
            serde_json::json!({
                "id": id,
                "type": "input",
                "position": {"x": 0, "y": 0},
                "data": {
                    "type": "input",
                    "label": id,
                    "shaderCode": "",
                    "inputs": [],
                    "outputs": [{
                        "id": "out",
                        "label": "output",
                        "dataType": "sampler2D",
                        "direction": "output"
                    }],
                    "uniforms": {},
                    "inputMode": mode,
                    "inputDataType": "sampler2D"
                }
            })
        })
        .collect();
    let mut graph: Graph = serde_json::from_value(serde_json::json!({
        "nodes": nodes,
        "edges": []
    }))
    .unwrap();
    let (mut host, _) = pollster::block_on(NativePlayer::new()).unwrap();
    host.set_graph_snapshot(&graph).unwrap();
    let stable_pixels = [
        255, 255, 0, 255, 0, 255, 255, 255, 255, 0, 255, 255, 255, 255, 255, 255,
    ];
    host.upload_image("unchanged", &stable_pixels, 2, 2).unwrap();
    let unchanged_texture = host.preview_source("unchanged").unwrap();
    let info = host
        .attach_video(
            "source",
            NativeVideoConfig {
                kind: NativeVideoSourceKind::File,
                source: path.to_string_lossy().into_owned(),
                looping: true,
                playback_rate: 1.0,
            },
        )
        .unwrap();
    assert_eq!((info.width, info.height), (64, 64));
    host.start_playback().unwrap();
    assert_eq!(host.videos.node_ids(), ["source"]);
    assert!(host.videos.is_active());

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        host.upload_video_frames().unwrap();
        if host.video_metrics("source").unwrap().uploaded_frames > 0 {
            break;
        }
        assert!(Instant::now() < deadline, "decoded video frame did not arrive");
        thread::sleep(Duration::from_millis(10));
    }
    let metrics = host.video_metrics("source").unwrap();
    assert!(metrics.decoded_frames > 0);
    assert!(metrics.uploaded_frames > 0);
    let (video_pixels, width, height) = host.pipeline.render_device.read_output_rgba("source").unwrap();
    assert_eq!((width, height), (64, 64));
    assert_eq!(video_pixels.len(), 64 * 64 * 4);
    assert!(host.videos.is_active(), "looping decoder must still be active");

    for detach in [false, true] {
        let request = crate::execution::FrameDemand {
            request_id: if detach { 2 } else { 1 }, graph_revision: host.revision(),
            output_node_id: "source".into(), sources: vec![],
            input: FrameEvaluationInput {
                stamp: FrameStamp { epoch: 1, frame: if detach { 2 } else { 1 }, timeline_ns: 0, deadline_ns: 100 },
                previous_timeline_ns: 0, date: [0.0; 4], mouse: [0.0; 4], resolution: [64.0, 64.0, 1.0],
            },
        };
        assert!(matches!(host.pipeline.submit(request), crate::execution::FrameAdmission::Accepted));
        if detach { host.detach_video("source").unwrap(); }
        else {
            host.attach_video("source", NativeVideoConfig {
                kind: NativeVideoSourceKind::File, source: path.to_string_lossy().into_owned(),
                looping: true, playback_rate: 1.0,
            }).unwrap();
        }
        assert!(matches!(host.pipeline.poll_completion().unwrap().outcome, crate::execution::FrameOutcome::Cancelled));
        host.pipeline.progress();
        assert!(host.pipeline.poll_completion().is_none());
        assert!(!host.has_output_texture("source"));
    }
    host.attach_video("source", NativeVideoConfig {
        kind: NativeVideoSourceKind::File, source: path.to_string_lossy().into_owned(),
        looping: true, playback_rate: 1.0,
    }).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !host.has_output_texture("source") {
        host.upload_video_frames().unwrap();
        assert!(Instant::now() < deadline, "replacement video did not publish a frame");
        thread::sleep(Duration::from_millis(10));
    }

    let mut input = SessionFrameInput {
        now_ns: 1_000_000_000,
        date: [0.0; 4],
        mouse: [0.0; 4],
        resolution: [16.0, 16.0, 1.0],
    };
    planned_dirty_node_count(&mut host, &mut input);
    assert_eq!(planned_dirty_node_count(&mut host, &mut input), 1);

    let old_revision = host.pipeline.session.revision();
    let old_state = host.pipeline.session.state();
    let old_plan = serde_json::to_value(host.pipeline.session.execution_plan().unwrap()).unwrap();
    let old_source_generation = host.pipeline.session.node_generation("source").unwrap();
    let old_image_generation = host.pipeline.session.node_generation("unchanged").unwrap();
    let old_metrics = host.video_metrics("source").unwrap();
    let old_video_texture = host.preview_source("source").unwrap();
    let mut candidate_graph = graph.clone();
    let mut candidate_only = candidate_graph.nodes.iter()
        .find(|node| node.id == "unchanged").unwrap().clone();
    candidate_only.id = "candidate-only".to_owned();
    candidate_graph.nodes.push(candidate_only);
    let mut candidate = host.prepare_graph(&candidate_graph).unwrap();
    candidate.stage_remove_texture("unchanged");
    candidate.stage_detach_video("source");
    let candidate_pixels = [0, 0, 0, 255, 64, 64, 64, 255, 128, 128, 128, 255, 0, 0, 0, 255];
    candidate.stage_image("unchanged", &candidate_pixels, 2, 2).unwrap();
    assert!(candidate
        .stage_video(
            "source",
            NativeVideoConfig {
                kind: NativeVideoSourceKind::File,
                source: directory.join("missing.mp4").to_string_lossy().into_owned(),
                looping: true,
                playback_rate: 1.0,
            },
            false,
        )
        .is_err());
    drop(candidate);

    assert_eq!(host.session.revision(), old_revision);
    assert_eq!(host.session.state(), old_state);
    assert_eq!(
        serde_json::to_value(host.session.execution_plan().unwrap()).unwrap(),
        old_plan
    );
    assert_eq!(host.session.node_generation("source").unwrap(), old_source_generation);
    assert_eq!(host.session.node_generation("unchanged").unwrap(), old_image_generation);
    assert!(host.session.node_generation("candidate-only").is_err());
    let retained_metrics = host.video_metrics("source").unwrap();
    // The live decoder may advance while staging, but its upload counters must not reset.
    assert!(retained_metrics.decoded_frames >= old_metrics.decoded_frames);
    assert_eq!(retained_metrics.uploaded_frames, old_metrics.uploaded_frames);
    assert_eq!(retained_metrics.cpu_copy_bytes, old_metrics.cpu_copy_bytes);
    assert_eq!(retained_metrics.hardware_decode, old_metrics.hardware_decode);
    assert_eq!(host.videos.node_ids(), ["source"]);
    assert!(host.videos.is_active());
    assert!(Arc::ptr_eq(
        &old_video_texture.texture,
        &host.preview_source("source").unwrap().texture
    ));
    assert_eq!(
        host.render_device.read_output_rgba("source").unwrap(),
        (video_pixels.clone(), 64, 64)
    );
    assert!(Arc::ptr_eq(
        &unchanged_texture.texture,
        &host.preview_source("unchanged").unwrap().texture
    ));
    assert_eq!(
        host.render_device.read_output_rgba("unchanged").unwrap(),
        (stable_pixels.to_vec(), 2, 2),
        "discarded staged image edits must not change the live texture"
    );

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        host.upload_video_frames().unwrap();
        if host.video_metrics("source").unwrap().uploaded_frames > old_metrics.uploaded_frames {
            break;
        }
        assert!(Instant::now() < deadline, "old decoder stopped after failed staging");
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(host.session.revision(), old_revision);
    assert_eq!(host.videos.node_ids(), ["source"]);
    assert!(host.videos.is_active());
    assert_eq!(planned_dirty_node_count(&mut host, &mut input), 1);

    let mut replacement = host.prepare_graph(&graph).unwrap();
    replacement.stage_video("source", NativeVideoConfig {
        kind: NativeVideoSourceKind::File,
        source: path.to_string_lossy().into_owned(),
        looping: false,
        playback_rate: 1.0,
    }, true).unwrap();
    assert!(host.has_output_texture("source"), "staging must preserve active pixels");
    host.commit_graph(replacement).unwrap();
    assert!(!host.has_output_texture("source"), "replacement preview must wait for its own decoded frame");
    assert_eq!(host.video_metrics("source").unwrap().uploaded_frames, 0);
    assert_eq!(host.render_device.read_output_rgba("unchanged").unwrap().0, stable_pixels);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !host.has_output_texture("source") {
        host.upload_video_frames().unwrap();
        assert!(Instant::now() < deadline, "replacement preview frame did not arrive");
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(host.video_metrics("source").unwrap().uploaded_frames, 1);

    graph.nodes.iter_mut().find(|node| node.id == "source").unwrap().data.input_mode =
        Some(InputMode::Image);
    host.set_graph_snapshot(&graph).unwrap();
    assert!(!host.has_output_texture("source"), "retyped video must not retain stale decoded pixels");
    assert!(host.video_metrics("source").is_err());
    assert!(!host.videos.node_ids().iter().any(|id| id == "source"));
    assert!(host.videos.node_ids().is_empty());
    assert!(!host.videos.is_active());
    assert_eq!(host.videos.frame_counts(), (0, 0));
    assert!(Arc::ptr_eq(
        &unchanged_texture.texture,
        &host.preview_source("unchanged").unwrap().texture
    ));
    assert_eq!(
        host.render_device.read_output_rgba("unchanged").unwrap(),
        (stable_pixels.to_vec(), 2, 2)
    );

    let image_pixels = [
        255, 0, 0, 255, 0, 255, 0, 255, 255, 128, 0, 255, 128, 0, 128, 255,
    ];
    host.upload_image("source", &image_pixels, 2, 2).unwrap();
    assert_eq!(
        host.render_device.read_output_rgba("source").unwrap(),
        (image_pixels.to_vec(), 2, 2)
    );
    // Consume graph replacement and image-upload dirtiness before checking video signals.
    planned_dirty_node_count(&mut host, &mut input);

    // Decoder drop is synchronous, so repeated polls require no delay.
    for _ in 0..8 {
        host.upload_video_frames().unwrap();
        assert_eq!(
            host.render_device.read_output_rgba("source").unwrap(),
            (image_pixels.to_vec(), 2, 2),
            "an old video frame replaced the new image"
        );
        assert_eq!(
            host.render_device.read_output_rgba("unchanged").unwrap(),
            (stable_pixels.to_vec(), 2, 2)
        );
        assert!(Arc::ptr_eq(
            &unchanged_texture.texture,
            &host.preview_source("unchanged").unwrap().texture
        ));
        assert!(host.video_metrics("source").is_err());
        assert!(host.videos.node_ids().is_empty());
        assert!(!host.videos.is_active());
        assert_eq!(host.videos.frame_counts(), (0, 0));
        assert_eq!(
            planned_dirty_node_count(&mut host, &mut input),
            0,
            "the removed video source must not keep the session dirty"
        );
    }

    drop(host);
    fs::remove_dir_all(directory).unwrap();
}
