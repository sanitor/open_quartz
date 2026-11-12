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
                    #[cfg(target_os = "macos")]
                    NativeVideoFrame::VideoToolbox(frame) => {
                        assert_eq!((frame.width(), frame.height()), (width, height))
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
fn initial_preview_decodes_one_real_frame_without_advancing_playback() {
    let ffmpeg = find_ffmpeg().unwrap();
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("open-quartz-preview-{suffix}.mp4"));
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
            "-y",
        ])
        .arg(&path)
        .status()
        .unwrap()
        .success());
    let mut source = NativeVideoSource::open_preview(NativeVideoConfig {
        kind: NativeVideoSourceKind::File,
        source: path.to_string_lossy().into_owned(),
        looping: true,
        playback_rate: 1.0,
    })
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while source.is_active() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        !source.is_active(),
        "one-frame preview decoder kept running"
    );
    assert_eq!(source.metrics().decoded_frames, 1);
    assert!(source
        .upload_latest(&mut |frame, width, height| {
            assert_eq!((width, height), (16, 16));
            match frame {
                NativeVideoFrame::Rgba(rgba) => assert!(rgba.chunks_exact(4).all(|pixel| pixel[0]
                    > 240
                    && pixel[1] < 10
                    && pixel[2] < 10
                    && pixel[3] == 255)),
                #[cfg(all(windows, feature = "native-video-d3d12"))]
                NativeVideoFrame::D3d12(frame) => {
                    assert_eq!((frame.width, frame.height), (width, height))
                }
                #[cfg(target_os = "macos")]
                NativeVideoFrame::VideoToolbox(frame) => {
                    assert_eq!((frame.width(), frame.height()), (width, height))
                }
            }
            Ok(())
        })
        .unwrap());
    assert!(!source
        .upload_latest(&mut |_, _, _| panic!("preview uploaded twice"))
        .unwrap());
    source.pause();
    assert_eq!(
        source.position_seconds, 0.0,
        "preview consumed the start of playback"
    );
    source.resume().unwrap();
    assert_eq!(
        source.position_seconds, 0.0,
        "explicit Play skipped the first frame"
    );
    assert!(!source.preview_only);
    let deadline = Instant::now() + Duration::from_secs(5);
    while source.metrics().decoded_frames < 2 && Instant::now() < deadline {
        source.upload_latest(&mut |_, _, _| Ok(())).unwrap();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        source.metrics().decoded_frames >= 2,
        "explicit Play stayed in preview mode"
    );
    drop(source);
    std::fs::remove_file(path).unwrap();
}

fn unstarted_software_source() -> NativeVideoSource {
    NativeVideoSource {
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
            #[cfg(target_os = "macos")]
            videotoolbox: None,
        })),
        alive: Arc::new(AtomicBool::new(false)),
        decoded_frames: Arc::new(AtomicU64::new(0)),
        uploaded_frames: 0,
        cpu_copy_bytes: Arc::new(AtomicU64::new(0)),
        child: None,
        reader: None,
        stderr_reader: None,
        position_seconds: 0.0,
        uploaded_generation: 0,
        preview_only: false,
        decode_policy: super::DecodePolicy::AllowSoftware,
        decoder_error: Arc::new(Mutex::new(None)),
        #[cfg(target_os = "macos")]
        hardware_position_ns: Arc::new(AtomicU64::new(0)),
        #[cfg(target_os = "macos")]
        decoder_cancelled: Arc::new(AtomicBool::new(false)),
        #[cfg(target_os = "macos")]
        exact_session: None,
        #[cfg(target_os = "macos")]
        exact_cancelled: None,
        #[cfg(target_os = "macos")]
        exact_last_pts_ns: None,
        #[cfg(target_os = "macos")]
        exact_last_target_ns: None,
        #[cfg(target_os = "macos")]
        exact_epoch: 0,
        #[cfg(target_os = "macos")]
        exact_composition_epoch: None,
        #[cfg(target_os = "macos")]
        exact_loop_cycle: 0,
        info: super::NativeVideoInfo {
            width: 1,
            height: 1,
            fps: 30.0,
            decoder: "test".to_owned(),
            #[cfg(all(windows, feature = "native-video-d3d12"))]
            d3d12_eligible: false,
            #[cfg(all(windows, feature = "native-video-d3d12"))]
            d3d12_detach_required: false,
            fallback_reason: None,
        },
    }
}

#[test]
fn decoder_spawn_and_resume_errors_are_propagated() {
    let mut source = unstarted_software_source();
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!(
            "oq-failing-decoder-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(
            &path,
            "#!/bin/sh\nprintf x\necho decode-failed >&2\nexit 9\n",
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        source.ffmpeg = path.clone();
        source.config.source = "0".to_owned();
        source.start_decoder().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let error = loop {
            match source.upload_latest(&mut |_, _, _| Ok(())) {
                Err(error) => break error,
                Ok(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
                Ok(_) => panic!("decoder failure was not reported"),
            }
        };
        assert!(
            error.contains("truncated") && error.contains("decode-failed"),
            "{error}"
        );
        source.pause();
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn hardware_policy_rejects_camera_before_ffmpeg_resolution_or_resource_creation() {
    super::FORCE_FFMPEG_ERROR.with(|forced| forced.set(true));
    for preview in [false, true] {
        let result = NativeVideoSource::open_hardware_required(
            NativeVideoConfig {
                kind: NativeVideoSourceKind::Camera,
                source: "__fail_reader_spawn__".to_owned(),
                looping: false,
                playback_rate: 1.0,
            },
            preview,
        );
        let error = result.err().expect("strict camera opening must fail");
        assert!(error.contains("camera capture"), "{error}");
        assert!(
            error.contains("CPU decode/upload fallback is disabled"),
            "{error}"
        );
        assert!(!error.contains("Injected FFmpeg"), "{error}");
    }
    super::FORCE_FFMPEG_ERROR.with(|forced| forced.set(false));
}

#[cfg(not(any(target_os = "macos", all(windows, feature = "native-video-d3d12"))))]
#[test]
fn hardware_policy_rejects_unsupported_platform_before_accessing_file_or_ffmpeg() {
    let error = NativeVideoSource::open_hardware_required(
        NativeVideoConfig {
            kind: NativeVideoSourceKind::File,
            source: "missing".to_owned(),
            looping: false,
            playback_rate: 1.0,
        },
        false,
    )
    .err()
    .expect("unsupported platform must fail");
    assert!(
        error.contains("no hardware-resident video decoder"),
        "{error}"
    );
    assert!(!error.contains("does not exist"), "{error}");
}

#[test]
fn strict_upload_rejects_rgba_before_callback_or_uploaded_generation_mutation() {
    let mut source = unstarted_software_source();
    source.decode_policy = super::DecodePolicy::HardwareRequired;
    {
        let Ok(mut slot) = source.slot.lock() else {
            panic!("frame slot poisoned");
        };
        slot.generation = 1;
        slot.rgba = vec![0; 4];
    }
    let error = source
        .upload_latest(&mut |_, _, _| panic!("CPU frame reached GPU callback"))
        .unwrap_err();
    assert!(error.contains("CPU RGBA frame"), "{error}");
    assert_eq!(source.uploaded_generation, 0);
    assert_eq!(source.metrics().uploaded_frames, 0);
    assert!(source.slot.lock().is_ok_and(|slot| slot.generation == 1));
    assert!(source.child.is_none() && source.reader.is_none());
}

#[test]
fn strict_backend_failure_never_starts_software_decoder() {
    let mut source = unstarted_software_source();
    source.decode_policy = super::DecodePolicy::HardwareRequired;
    source.config.kind = NativeVideoSourceKind::File;
    source.config.source = "__missing_hardware_video_fixture__".to_owned();
    #[cfg(all(windows, feature = "native-video-d3d12"))]
    {
        source.info.d3d12_eligible = true;
    }
    let error = source.start_decoder().unwrap_err();
    assert!(
        error.contains("CPU decode/upload fallback is disabled"),
        "{error}"
    );
    assert!(
        !error.contains("Cannot start FFmpeg video decoder"),
        "{error}"
    );
    assert!(!source.is_active());
    assert!(source.child.is_none() && source.reader.is_none() && source.stderr_reader.is_none());
    assert!(source
        .slot
        .lock()
        .is_ok_and(|slot| slot.generation == 0 && slot.rgba.is_empty()));
    assert_eq!(source.metrics().cpu_copy_bytes, 0);
    assert!(!source.metrics().hardware_decode);
}

#[cfg(any(target_os = "macos", all(windows, feature = "native-video-d3d12")))]
#[test]
fn hardware_file_playback_and_resume_keep_frames_resident() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/white_64x64.mp4");
    if !path.is_file() {
        return;
    }
    let mut source = NativeVideoSource::open_hardware_required(
        NativeVideoConfig {
            kind: NativeVideoSourceKind::File,
            source: path.to_string_lossy().into_owned(),
            looping: true,
            playback_rate: 1.0,
        },
        false,
    )
    .expect("real hardware decoding is required for the video fixture");
    for playback in 0..2 {
        if playback > 0 {
            source.resume().unwrap();
        }
        let initial = source.metrics().uploaded_frames;
        let deadline = Instant::now() + Duration::from_secs(5);
        while source.metrics().uploaded_frames < initial + 2 {
            source
                .upload_latest(&mut |frame, width, height| {
                    assert_eq!((width, height), (64, 64));
                    match frame {
                        NativeVideoFrame::Rgba(_) => {
                            panic!("hardware playback delivered a CPU frame")
                        }
                        #[cfg(target_os = "macos")]
                        NativeVideoFrame::VideoToolbox(frame) => {
                            assert!(!frame.pixel_buffer().is_null())
                        }
                        #[cfg(all(windows, feature = "native-video-d3d12"))]
                        NativeVideoFrame::D3d12(frame) => {
                            assert_eq!((frame.width, frame.height), (width, height))
                        }
                    }
                    Ok(())
                })
                .unwrap();
            assert!(
                Instant::now() < deadline,
                "hardware playback stopped producing frames"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(source.metrics().hardware_decode);
        assert_eq!(source.metrics().cpu_copy_bytes, 0);
        assert!(source.info().fallback_reason.is_none());
        assert!(source.child.is_none());
        source.pause();
        assert!(!source.is_active());
    }
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
        #[cfg(target_os = "macos")]
        videotoolbox: None,
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
        Arc::new(AtomicU64::new(0)),
        Arc::new(Mutex::new(None)),
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
    let cpu_copy_bytes = Arc::new(AtomicU64::new(0));
    super::read_rgba_frames(
        child.stdout.take().unwrap(),
        Arc::new(Mutex::new(super::FrameSlot {
            generation: 0,
            rgba: Vec::new(),
            #[cfg(target_os = "macos")]
            videotoolbox: None,
        })),
        alive,
        decoded.clone(),
        cpu_copy_bytes.clone(),
        Arc::new(Mutex::new(None)),
        4,
        Duration::ZERO,
    );
    assert_eq!(decoded.load(std::sync::atomic::Ordering::Acquire), 1);
    assert_eq!(cpu_copy_bytes.load(std::sync::atomic::Ordering::Acquire), 4);
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

#[cfg(target_os = "macos")]
#[test]
fn hdr_transfer_is_rejected_before_hardware_decode_without_tone_mapping() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("pq-input.mp4");
    let ffmpeg = find_ffmpeg().expect("bundled FFmpeg required");
    let status = Command::new(&ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:size=64x64:rate=1",
            "-frames:v",
            "1",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-color_trc",
            "smpte2084",
            "-colorspace",
            "bt2020nc",
            "-color_primaries",
            "bt2020",
            "-y",
        ])
        .arg(&path)
        .status()
        .unwrap();
    assert!(status.success(), "HDR-tagged fixture creation failed");
    let config = NativeVideoConfig {
        kind: NativeVideoSourceKind::File,
        source: path.to_string_lossy().into_owned(),
        looping: false,
        playback_rate: 1.0,
    };
    assert!(NativeVideoSource::open_hardware_required(config.clone(), false).is_err());
    assert!(super::probe_video(&ffmpeg, &config).is_err());
}
