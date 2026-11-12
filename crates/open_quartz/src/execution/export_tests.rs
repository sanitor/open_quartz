use super::*;
use crate::media::native::{NativeVideoConfig, NativeVideoSourceKind};
use crate::outputs::PlaybackCapabilities;
use crate::rendering::RenderDevice;
use oq_media::RationalTimeBase;
use serde_json::json;
use std::time::Duration;

fn graph() -> crate::Graph {
    serde_json::from_value(json!({"nodes":[
        {"id":"video","type":"input","position":{"x":0,"y":0},"data":{"type":"input","label":"video","shaderCode":"","inputMode":"video","inputDataType":"sampler2D","inputs":[],"outputs":[{"id":"out","label":"image","dataType":"sampler2D","direction":"output"}],"uniforms":{}}},
        {"id":"effect","type":"shader","position":{"x":0,"y":0},"data":{"type":"shader","label":"invert","shaderCode":"@fragment fn main(@location(0) uv:vec2f)->@location(0) vec4f { let color=textureSample(image,imageSampler,uv);return vec4f(vec3f(1.0)-color.rgb,1.0);}","inputs":[{"id":"in","label":"image","dataType":"sampler2D","direction":"input"}],"outputs":[{"id":"out","label":"fragColor","dataType":"vec4","direction":"output"}],"uniforms":{},"autoSize":false,"width":64,"height":64}},
        {"id":"renderer","type":"renderer","position":{"x":0,"y":0},"data":{"type":"renderer","label":"renderer","shaderCode":"","inputs":[{"id":"in","label":"image","dataType":"sampler2D","direction":"input"}],"outputs":[],"uniforms":{}}}
    ],"edges":[{"id":"a","source":"video","sourceHandle":"out","target":"effect","targetHandle":"in"},{"id":"b","source":"effect","sourceHandle":"out","target":"renderer","targetHandle":"in"}]})).unwrap()
}
fn fixture() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../oq-media/tests/source-time.mp4")
}
fn prepare(path: &std::path::Path) -> FramePipeline {
    let (device, _) = pollster::block_on(RenderDevice::new()).expect("real hardware GPU required");
    let mut pipeline =
        FramePipeline::from_render_device(device, PlaybackCapabilities { data_paths: vec![] });
    pipeline.set_graph_snapshot(&graph()).unwrap();
    pipeline
        .attach_video(
            "video",
            NativeVideoConfig {
                kind: NativeVideoSourceKind::File,
                source: path.to_string_lossy().into_owned(),
                looping: false,
                playback_rate: 1.0,
            },
        )
        .unwrap();
    pipeline
}
fn config(path: &std::path::Path) -> ExportConfig {
    ExportConfig {
        output: path.to_owned(),
        output_node_id: "renderer".into(),
        frames: 1..13,
        encoder: EncoderConfig {
            width: 64,
            height: 64,
            time_base: RationalTimeBase {
                numerator: 1,
                denominator: 10,
            },
            bitrate: 2_000_000,
            queue_capacity: 3,
            max_b_frames: 0,
            overwrite: false,
        },
        input_interpretation: InputInterpretation::SdrSrgbRgba8Black,
        date: [2026.0, 10.0, 12.0, 0.0],
        mouse: [0.0; 4],
    }
}
fn finish(job: &mut ExportJob) -> ExportStatus {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let state = job.poll();
        if state.is_terminal() {
            return state;
        }
        assert!(Instant::now() < deadline, "export stalled: {state:?}");
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn ffmpeg() -> std::process::Command {
    let mut command = std::process::Command::new(crate::media::native::find_ffmpeg().unwrap());
    command
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("DYLD_FALLBACK_LIBRARY_PATH");
    command
}
fn decoded(path: &std::path::Path) -> Vec<u8> {
    let output = ffmpeg()
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args([
            "-an", "-vsync", "0", "-f", "rawvideo", "-pix_fmt", "rgb24", "-",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
fn frame_time_mapping_keeps_ntsc_rational_without_accumulated_rounding_and_rejects_overflow() {
    let base = RationalTimeBase {
        numerator: 1001,
        denominator: 30000,
    };
    assert_eq!(sample_ns(30000, base).unwrap(), 1_001_000_000_000);
    assert_eq!(sample_ns(0, base).unwrap(), 0);
    assert_eq!(sample_ns(1, base).unwrap(), 33_366_667);
    assert_eq!(sample_ns(2, base).unwrap(), 66_733_333);
    assert!(sample_ns(
        u64::MAX,
        RationalTimeBase {
            numerator: i32::MAX,
            denominator: 1
        }
    )
    .is_err());
    assert!(sample_ns(
        1,
        RationalTimeBase {
            numerator: 0,
            denominator: 1
        }
    )
    .is_err());
}

#[test]
fn resident_decode_effect_nv12_hardware_encode_mp4_has_exact_range_pts_tail_and_pixels() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("export.mp4");
    let mut job = ExportJob::new(prepare(&fixture()), config(&output)).unwrap();
    job.enable_stage_trace(true);
    let state = finish(&mut job);
    assert_eq!(state.phase, ExportPhase::Completed, "{state:?}");
    let report = state.report.unwrap();
    assert!(report.hardware);
    assert_eq!(report.input_frames, 12);
    assert!(report.packets >= 12);
    let bytes = decoded(&output);
    assert_eq!(
        bytes.len(),
        12 * 64 * 64 * 3,
        "drained MP4 must contain exactly the half-open requested range"
    );
    let oracle = ffmpeg()
        .args(["-hide_banner", "-loglevel", "info", "-copyts", "-i"])
        .arg(fixture())
        .args([
            "-an",
            "-vf",
            "showinfo,extractplanes=y",
            "-vsync",
            "0",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "gray",
            "-",
        ])
        .output()
        .unwrap();
    assert!(oracle.status.success());
    let diagnostics = String::from_utf8_lossy(&oracle.stderr);
    let source_pts = diagnostics
        .lines()
        .filter(|line| line.contains(" n:") && line.contains("pts_time:"))
        .map(|line| {
            line.split("pts_time:")
                .nth(1)
                .unwrap()
                .split_whitespace()
                .next()
                .unwrap()
                .parse::<f64>()
                .unwrap()
                - 2.0
        })
        .collect::<Vec<_>>();
    assert_eq!(oracle.stdout.len(), source_pts.len() * 64 * 64);
    let transformed = |value: u8| {
        let encoded = ((f64::from(value) - 16.0) / 219.0).clamp(0.0, 1.0);
        let linear = if encoded < 0.081 {
            encoded / 4.5
        } else {
            ((encoded + 0.099) / 1.099).powf(1.0 / 0.45)
        };
        let srgb = if linear <= 0.0031308 {
            linear * 12.92
        } else {
            1.055 * linear.powf(1.0 / 2.4) - 0.055
        };
        let inverted = 1.0 - srgb;
        let linear = if inverted <= 0.04045 {
            inverted / 12.92
        } else {
            ((inverted + 0.055) / 1.055).powf(2.4)
        };
        255.0
            * if linear < 0.018 {
                linear * 4.5
            } else {
                1.099 * linear.powf(0.45) - 0.099
            }
    };
    for index in 0..12usize {
        let target = (index + 1) as f64 / 10.0;
        let selected = source_pts
            .iter()
            .position(|pts| *pts + 0.000001 >= target)
            .expect("software oracle target");
        let source = &oracle.stdout[selected * 64 * 64..(selected + 1) * 64 * 64];
        let actual = &bytes[index * 64 * 64 * 3..(index + 1) * 64 * 64 * 3];
        let expected_mean =
            source.iter().map(|value| transformed(*value)).sum::<f64>() / source.len() as f64;
        let actual_mean =
            actual.iter().map(|value| f64::from(*value)).sum::<f64>() / actual.len() as f64;
        assert!((actual_mean-expected_mean).abs()<=5.0,"frame {index}, selected sourcePTS {}: encoded effect mean {actual_mean}, independent oracle {expected_mean}",source_pts[selected]);
    }
    let info = ffmpeg()
        .args(["-hide_banner", "-i"])
        .arg(&output)
        .args(["-map", "0:v:0", "-vf", "showinfo", "-f", "null", "-"])
        .output()
        .unwrap();
    assert!(info.status.success());
    let log = String::from_utf8_lossy(&info.stderr);
    let stamps = log
        .lines()
        .filter(|line| line.contains(" n:") && line.contains("pts_time:"))
        .map(|line| {
            line.split("pts_time:")
                .nth(1)
                .unwrap()
                .split_whitespace()
                .next()
                .unwrap()
                .parse::<f64>()
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        stamps.len(),
        12,
        "independent decoder must expose every presentation timestamp: {log}"
    );
    for (index, stamp) in stamps.into_iter().enumerate() {
        assert!((stamp - index as f64 / 10.0).abs() < 0.0001);
    }
    assert!(
        log.contains("bt709"),
        "encoded color tags must be BT.709: {log}"
    );
    let events = job.drain_stage_events();
    assert!(events.iter().any(|event| event.stage == "Nv12Submitted"));
    assert_eq!(
        job.pipeline.video_metrics("video").unwrap().cpu_copy_bytes,
        0
    );
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-results/phase5");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(&output, dir.join("resident-export.mp4")).unwrap();
}

#[test]
fn source_change_and_cancel_leave_existing_target_and_no_committed_partial() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.mp4");
    std::fs::copy(fixture(), &source).unwrap();
    let target = dir.path().join("protected.mp4");
    std::fs::write(&target, b"existing-target").unwrap();
    let mut setup = config(&target);
    setup.encoder.overwrite = true;
    let mut job = ExportJob::new(prepare(&source), setup).unwrap();
    std::fs::write(&source, b"changed-source").unwrap();
    let failed = finish(&mut job);
    assert_eq!(failed.phase, ExportPhase::Failed, "{failed:?}");
    drop(job);
    assert_eq!(std::fs::read(&target).unwrap(), b"existing-target");
    std::fs::copy(fixture(), &source).unwrap();
    for boundary in 0..3 {
        let mut setup = config(&target);
        setup.encoder.overwrite = true;
        let mut job = ExportJob::new(prepare(&source), setup).unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        while boundary > 0 {
            let state = job.poll();
            assert!(!state.is_terminal(), "{state:?}");
            if (boundary == 1 && state.rendered_frames > 0)
                || (boundary == 2 && state.accepted_frames > 0)
            {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        job.cancel();
        assert_eq!(finish(&mut job).phase, ExportPhase::Cancelled);
        drop(job);
        assert_eq!(std::fs::read(&target).unwrap(), b"existing-target");
    }
    assert!(std::fs::read_dir(dir.path()).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".oq-export-")));
}

#[test]
fn invalid_range_dimensions_and_source_target_alias_are_rejected_before_export() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("invalid.mp4");
    let mut invalid = config(&output);
    invalid.frames = 4..4;
    assert!(ExportJob::new(prepare(&fixture()), invalid).is_err());
    assert!(!output.exists());
    let mut invalid = config(&output);
    invalid.encoder.width = 63;
    assert!(ExportJob::new(prepare(&fixture()), invalid).is_err());
    assert!(!output.exists());
    let mut invalid = config(&fixture());
    invalid.encoder.overwrite = true;
    assert!(ExportJob::new(prepare(&fixture()), invalid).is_err());
}
