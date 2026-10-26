use super::{
    append_input_args, d3d12_decode_policy, find_ffmpeg, frame_pacing_delay, read_video_thumbnail,
    validate_monotonic_pts, NativeVideoConfig, NativeVideoFrame, NativeVideoSource,
    NativeVideoSourceKind,
};
use std::cell::Cell;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[test]
fn timestamp_pacing_and_policy_edges_are_covered() {
    let pts = Cell::new(None);
    assert!(validate_monotonic_pts(&pts, None).is_ok());
    assert!(validate_monotonic_pts(&pts, Some(10)).is_ok());
    assert!(validate_monotonic_pts(&pts, Some(9)).is_err());
    let start = Instant::now();
    let mut next = start;
    assert_eq!(
        frame_pacing_delay(
            &mut next,
            start + Duration::from_millis(8),
            Duration::from_millis(33)
        ),
        Duration::from_millis(25)
    );
    assert_eq!(
        frame_pacing_delay(
            &mut next,
            start + Duration::from_millis(100),
            Duration::from_millis(33)
        ),
        Duration::ZERO
    );
    assert_eq!(
        d3d12_decode_policy("Video: h264, yuv420p, 1920x1080"),
        (true, true)
    );
    assert_eq!(
        d3d12_decode_policy("Video: hevc, nv12, 640x480"),
        (true, false)
    );
    assert_eq!(
        d3d12_decode_policy("Video: vp9, yuv420p, 640x480"),
        (false, false)
    );
    assert_eq!(
        d3d12_decode_policy("Video: h264, p010, 640x480"),
        (true, true)
    );
}

#[test]
fn config_ffmpeg_thumbnail_and_device_argument_edges_are_covered() {
    assert!(NativeVideoSource::open(NativeVideoConfig {
        kind: NativeVideoSourceKind::File,
        source: "missing".to_owned(),
        looping: false,
        playback_rate: 1.0
    })
    .is_err());
    assert!(NativeVideoSource::open(NativeVideoConfig {
        kind: NativeVideoSourceKind::Camera,
        source: "0".to_owned(),
        looping: false,
        playback_rate: f64::NAN
    })
    .is_err());
    assert!(read_video_thumbnail("missing").is_err());
    let mut command = Command::new("ffmpeg");
    append_input_args(
        &mut command,
        &NativeVideoConfig {
            kind: NativeVideoSourceKind::Camera,
            source: "2".to_owned(),
            looping: false,
            playback_rate: 1.0,
        },
        0.0,
    );
    assert!(!command.get_args().collect::<Vec<_>>().is_empty());
    assert!(
        super::select_ffmpeg(&[std::path::PathBuf::from("definitely-missing-ffmpeg")]).is_err()
    );
    let mut candidates = Vec::new();
    super::add_executable_candidate(&mut candidates, None);
    assert!(candidates.is_empty());
    super::add_executable_candidate(&mut candidates, Some(std::env::current_exe().unwrap()));
    assert_eq!(candidates.len(), 1);
    let ffmpeg = find_ffmpeg().unwrap();
    std::env::set_var("OPEN_QUARTZ_FFMPEG_PATH", &ffmpeg);
    assert_eq!(find_ffmpeg().unwrap(), ffmpeg);
    std::env::remove_var("OPEN_QUARTZ_FFMPEG_PATH");
    let invalid = std::env::temp_dir().join("open-quartz-invalid-thumbnail.txt");
    std::fs::write(&invalid, "not a video").unwrap();
    assert!(read_video_thumbnail(&invalid.to_string_lossy()).is_err());
    assert!(super::probe_video(
        &ffmpeg,
        &NativeVideoConfig {
            kind: NativeVideoSourceKind::File,
            source: invalid.to_string_lossy().into_owned(),
            looping: false,
            playback_rate: 1.0
        }
    )
    .is_err());
    assert!(NativeVideoSource::open(NativeVideoConfig {
        kind: NativeVideoSourceKind::File,
        source: invalid.to_string_lossy().into_owned(),
        looping: false,
        playback_rate: 1.0
    })
    .is_err());
    super::FORCE_FFMPEG_ERROR.with(|forced| forced.set(true));
    assert!(NativeVideoSource::open(NativeVideoConfig {
        kind: NativeVideoSourceKind::File,
        source: invalid.to_string_lossy().into_owned(),
        looping: false,
        playback_rate: 1.0
    })
    .is_err());
    assert!(read_video_thumbnail(&invalid.to_string_lossy()).is_err());
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    assert!(super::list_video_devices().is_err());
    super::FORCE_FFMPEG_ERROR.with(|forced| forced.set(false));
    assert!(NativeVideoSource::open(NativeVideoConfig {
        kind: NativeVideoSourceKind::Camera,
        source: "__fail_reader_spawn__".to_owned(),
        looping: false,
        playback_rate: 1.0
    })
    .is_err());
    assert!(super::read_video_thumbnail_with(
        std::path::Path::new("definitely-missing-ffmpeg"),
        &invalid.to_string_lossy()
    )
    .is_err());
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    assert!(
        super::list_video_devices_with(std::path::Path::new("definitely-missing-ffmpeg")).is_err()
    );
    let _ = std::fs::remove_file(invalid);
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    assert!(super::list_video_devices().is_ok());
    #[cfg(target_os = "macos")]
    assert!(super::parse_video_devices("ignored").is_empty());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let failing = std::env::temp_dir().join(format!("open-quartz-failing-ffmpeg-{suffix}"));
        std::fs::write(&failing, "#!/bin/sh\nexit 1\n").unwrap();
        std::fs::set_permissions(&failing, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            super::select_ffmpeg(&[failing.clone(), ffmpeg.clone()]).unwrap(),
            ffmpeg
        );
        let no_stream = std::env::temp_dir().join(format!("open-quartz-no-stream-{suffix}"));
        std::fs::write(&no_stream, "#!/bin/sh\necho no-video >&2\n").unwrap();
        std::fs::set_permissions(&no_stream, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(super::probe_video(
            &no_stream,
            &NativeVideoConfig {
                kind: NativeVideoSourceKind::Camera,
                source: "0".to_owned(),
                looping: false,
                playback_rate: 1.0
            }
        )
        .is_err());
        let no_size = std::env::temp_dir().join(format!("open-quartz-no-size-{suffix}"));
        std::fs::write(
            &no_size,
            "#!/bin/sh\necho 'Video: h264, yuv420p, 30 fps' >&2\n",
        )
        .unwrap();
        std::fs::set_permissions(&no_size, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(super::probe_video(
            &no_size,
            &NativeVideoConfig {
                kind: NativeVideoSourceKind::Camera,
                source: "0".to_owned(),
                looping: false,
                playback_rate: 1.0
            }
        )
        .is_err());
        assert!(super::probe_video(
            std::path::Path::new("definitely-missing-ffmpeg"),
            &NativeVideoConfig {
                kind: NativeVideoSourceKind::Camera,
                source: "0".to_owned(),
                looping: false,
                playback_rate: 1.0
            }
        )
        .is_err());
        for path in [failing, no_stream, no_size] {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[test]
fn native_file_decoder_runs_and_handles_upload_pause_resume() {
    let ffmpeg = find_ffmpeg().unwrap();
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("open-quartz-video-{suffix}.mp4"));
    assert!(Command::new(&ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=16x16:r=10",
            "-t",
            "0.4",
            "-pix_fmt",
            "yuv420p",
            "-y"
        ])
        .arg(&path)
        .status()
        .unwrap()
        .success());
    assert!(read_video_thumbnail(&path.to_string_lossy())
        .unwrap()
        .starts_with(&[0xff, 0xd8]));
    let mut source = NativeVideoSource::open(NativeVideoConfig {
        kind: NativeVideoSourceKind::File,
        source: path.to_string_lossy().into_owned(),
        looping: true,
        playback_rate: 1.0,
    })
    .unwrap();
    source.resume().unwrap();
    assert_eq!((source.info().width, source.info().height), (16, 16));
    let rejection_deadline = Instant::now() + Duration::from_secs(5);
    let mut rejected = false;
    while Instant::now() < rejection_deadline && !rejected {
        if let Err(error) = source.upload_latest(&mut |_, _, _| Err("reject".to_owned())) {
            assert_eq!(error, "reject");
            rejected = true;
        } else {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    assert!(rejected);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut frames = 0;
    while Instant::now() < deadline && frames < 2 {
        source
            .upload_latest(&mut |frame, width, height| {
                match frame {
                    NativeVideoFrame::Rgba(rgba) => {
                        assert_eq!(rgba.len(), width as usize * height as usize * 4);
                    }
                    #[cfg(all(windows, feature = "native-video-d3d12"))]
                    NativeVideoFrame::D3d12(frame) => {
                        assert_eq!((frame.width, frame.height), (width, height));
                    }
                }
                frames += 1;
                Ok(())
            })
            .unwrap();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(frames >= 2);
    assert!(source.metrics().decoded_frames >= source.metrics().uploaded_frames);
    source.pause();
    assert!(!source.is_active());
    let uploaded = source.metrics().uploaded_frames;
    source.resume().unwrap();
    assert!(source.is_active());
    let resume_deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < resume_deadline && source.metrics().uploaded_frames == uploaded {
        source.upload_latest(&mut |_, _, _| Ok(())).unwrap();
        std::thread::sleep(Duration::from_millis(20));
    }
    source.pause();
    drop(source);

    let mut finite = NativeVideoSource::open(NativeVideoConfig {
        kind: NativeVideoSourceKind::File,
        source: path.to_string_lossy().into_owned(),
        looping: false,
        playback_rate: 4.0,
    })
    .unwrap();
    let eof_deadline = Instant::now() + Duration::from_secs(5);
    while finite.is_active() && Instant::now() < eof_deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!finite.is_active());
    finite.pause();
    let slot = finite.slot.clone();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = slot.lock().unwrap();
        panic!("poison frame slot");
    }));
    assert!(finite.upload_latest(&mut |_, _, _| Ok(())).is_err());
    let _ = std::fs::remove_file(path);
}

#[test]
fn decoder_spawn_and_resume_errors_are_propagated() {
    let mut source = NativeVideoSource {
        ffmpeg: std::path::PathBuf::from("definitely-missing-ffmpeg"),
        config: NativeVideoConfig {
            kind: NativeVideoSourceKind::Camera,
            source: "0".to_owned(),
            looping: false,
            playback_rate: 1.0,
        },
        slot: Arc::new(Mutex::new(super::FrameSlot {
            generation: 0,
            rgba: Vec::new(),
            #[cfg(all(windows, feature = "native-video-d3d12"))]
            d3d12: None,
        })),
        alive: Arc::new(AtomicBool::new(false)),
        decoded_frames: Arc::new(AtomicU64::new(0)),
        uploaded_frames: 0,
        cpu_copy_bytes: 0,
        child: None,
        reader: None,
        position_seconds: 0.0,
        uploaded_generation: 0,
        info: super::NativeVideoInfo {
            width: 1,
            height: 1,
            fps: 30.0,
            decoder: "test".to_owned(),
            d3d12_eligible: false,
            d3d12_detach_required: false,
        },
    };
    source.slot.lock().unwrap().generation = 1;
    assert!(source
        .upload_latest(&mut |frame, _, _| {
            #[allow(irrefutable_let_patterns)]
            let NativeVideoFrame::Rgba(rgba) = frame
            else {
                unreachable!()
            };
            assert!(rgba.is_empty());
            Ok(())
        })
        .unwrap());
    assert!(source.start_decoder().is_err());
    source
        .alive
        .store(false, std::sync::atomic::Ordering::Release);
    assert!(source.resume().is_err());
    source.ffmpeg = find_ffmpeg().unwrap();
    source.config.source = "__fail_reader_spawn__".to_owned();
    source
        .alive
        .store(false, std::sync::atomic::Ordering::Release);
    assert!(source.start_decoder().is_err());
    source.pause();
}

#[cfg(unix)]
#[test]
fn rgba_reader_handles_poisoned_slot_and_zero_interval() {
    use std::process::Stdio;
    let mut child = Command::new("/bin/sh")
        .args(["-c", "printf '\\000\\000\\000\\000'"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let slot = Arc::new(Mutex::new(super::FrameSlot {
        generation: 0,
        rgba: Vec::new(),
    }));
    let poisoned = slot.clone();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = poisoned.lock().unwrap();
        panic!("poison reader slot");
    }));
    let alive = Arc::new(AtomicBool::new(true));
    super::read_rgba_frames(
        child.stdout.take().unwrap(),
        slot,
        alive.clone(),
        Arc::new(AtomicU64::new(0)),
        4,
        Duration::ZERO,
    );
    assert!(!alive.load(std::sync::atomic::Ordering::Acquire));
    let _ = child.wait();

    let mut child = Command::new("/bin/sh")
        .args(["-c", "printf '\\000\\000\\000\\000'"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let alive = Arc::new(AtomicBool::new(true));
    let decoded = Arc::new(AtomicU64::new(0));
    super::read_rgba_frames(
        child.stdout.take().unwrap(),
        Arc::new(Mutex::new(super::FrameSlot {
            generation: 0,
            rgba: Vec::new(),
        })),
        alive,
        decoded.clone(),
        4,
        Duration::ZERO,
    );
    assert_eq!(decoded.load(std::sync::atomic::Ordering::Acquire), 1);
    let _ = child.wait();
}

#[cfg(target_os = "macos")]
#[test]
fn parses_avfoundation_devices() {
    let devices = super::parse_video_devices("[0] FaceTime HD Camera\n[1] Capture Screen 0");
    assert_eq!(devices.len(), 2);
    assert_eq!(devices[0].label, "FaceTime HD Camera");
}

#[cfg(target_os = "windows")]
#[test]
fn parses_directshow_devices() {
    let devices = super::parse_video_devices(
        r#""Integrated Camera" (video)
"Microphone" (audio)"#,
    );
    assert_eq!(devices.len(), 1);
}
