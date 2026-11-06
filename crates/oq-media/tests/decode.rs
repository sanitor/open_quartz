#![cfg(all(target_os = "macos", feature = "videotoolbox"))]
use oq_media::Session;
use std::{process::Command, time::{SystemTime, UNIX_EPOCH}};

fn ffmpeg() -> Command {
    let path = std::env::var_os("OPEN_QUARTZ_FFMPEG_PATH").map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/open_quartz_slint/runtime/ffmpeg"));
    let mut command=Command::new(path);
    command.env_remove("DYLD_LIBRARY_PATH").env_remove("DYLD_FALLBACK_LIBRARY_PATH");
    command
}

#[test]
fn real_decode_drains_and_reset_preserves_preview_start() {
    let path = std::env::temp_dir().join(format!("oq-vt-{}.mp4", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    let status = ffmpeg().args(["-v", "error", "-f", "lavfi", "-i", "testsrc2=size=64x64:rate=10:duration=1", "-c:v", "libx264", "-bf", "2", "-pix_fmt", "yuv420p", "-y"]).arg(&path).status().expect("packaged ffmpeg is required for this test");
    assert!(status.success());
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    assert!(Session::open_with_cancel(&path, cancelled).is_err());
    let mut session = Session::open(&path).unwrap();
    assert_eq!(session.actual_pipeline().hardware, None);
    let preview = session.next_frame().unwrap().unwrap();
    assert_eq!(session.actual_pipeline().hardware, Some(true));
    assert!(!preview.frame.pixel_buffer().is_null());
    let preview_pts = preview.frame.pts_ns();
    let retained = preview.frame.try_clone().unwrap();
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut cancellable = Session::open_with_cancel(&path, cancelled.clone()).unwrap();
    cancelled.store(true, std::sync::atomic::Ordering::Release);
    assert!(cancellable.next_frame().is_err());
    session.reset().unwrap();
    let first = session.next_frame().unwrap().unwrap();
    assert_eq!(first.frame.pts_ns(), preview_pts);
    assert_eq!(first.timeline_epoch, 1);
    assert_eq!((retained.width(), retained.height()), (64, 64));
    let mut count = 1;
    let mut previous = first.frame.pts_ns();
    while let Some(frame) = session.next_frame().unwrap() {
        if let (Some(a), Some(b)) = (previous, frame.frame.pts_ns()) { assert!(b >= a); }
        previous = frame.frame.pts_ns();
        count += 1;
    }
    assert_eq!(count, 10, "delayed B-frames must be drained");
    assert!(session.next_frame().unwrap().is_none());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn hdr_transfer_is_rejected() {
    let path = std::env::temp_dir().join(format!("oq-vt-hdr-{}.mp4", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    let status = ffmpeg().args(["-v", "error", "-f", "lavfi", "-i", "testsrc2=size=64x64:rate=1:duration=1", "-c:v", "libx264", "-color_trc", "smpte2084", "-pix_fmt", "yuv420p", "-y"]).arg(&path).status().expect("packaged ffmpeg is required for this test");
    assert!(status.success());
    let mut session = Session::open(&path).unwrap();
    let error = match session.next_frame() { Err(error) => error, Ok(_) => panic!("HDR frame was accepted") };
    assert_eq!(error.kind, oq_media::ErrorKind::UnsupportedContent);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn linear_transfer_is_rejected() {
    let path = std::env::temp_dir().join(format!("oq-vt-linear-{}.mp4", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    let status = ffmpeg().args(["-v", "error", "-f", "lavfi", "-i", "testsrc2=size=64x64:rate=1:duration=1", "-c:v", "libx264", "-color_trc", "linear", "-pix_fmt", "yuv420p", "-y"]).arg(&path).status().expect("packaged ffmpeg is required for this test");
    assert!(status.success());
    let mut session = Session::open(&path).unwrap();
    let error = match session.next_frame() { Err(error) => error, Ok(_) => panic!("linear frame was accepted") };
    assert_eq!(error.kind, oq_media::ErrorKind::UnsupportedContent);
    std::fs::remove_file(path).unwrap();
}
