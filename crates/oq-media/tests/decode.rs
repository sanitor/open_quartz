#![cfg(all(target_os = "macos", feature = "videotoolbox"))]
use oq_media::Session;
use std::{
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn ffmpeg() -> Command {
    let path = std::env::var_os("OPEN_QUARTZ_FFMPEG_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../apps/open_quartz_slint/runtime/ffmpeg")
        });
    let mut command = Command::new(path);
    command
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("DYLD_FALLBACK_LIBRARY_PATH");
    command
}

#[test]
fn real_decode_drains_and_reset_preserves_preview_start() {
    let path = std::env::temp_dir().join(format!(
        "oq-vt-{}.mp4",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let status = ffmpeg()
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=64x64:rate=10:duration=1",
            "-c:v",
            "libx264",
            "-bf",
            "2",
            "-pix_fmt",
            "yuv420p",
            "-y",
        ])
        .arg(&path)
        .status()
        .expect("packaged ffmpeg is required for this test");
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
        if let (Some(a), Some(b)) = (previous, frame.frame.pts_ns()) {
            assert!(b >= a);
        }
        previous = frame.frame.pts_ns();
        count += 1;
    }
    assert_eq!(count, 10, "delayed B-frames must be drained");
    assert!(session.next_frame().unwrap().is_none());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn hdr_transfer_is_rejected() {
    let path = std::env::temp_dir().join(format!(
        "oq-vt-hdr-{}.mp4",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let status = ffmpeg()
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=64x64:rate=1:duration=1",
            "-c:v",
            "libx264",
            "-color_trc",
            "smpte2084",
            "-pix_fmt",
            "yuv420p",
            "-y",
        ])
        .arg(&path)
        .status()
        .expect("packaged ffmpeg is required for this test");
    assert!(status.success());
    let mut session = Session::open(&path).unwrap();
    let error = match session.next_frame() {
        Err(error) => error,
        Ok(_) => panic!("HDR frame was accepted"),
    };
    assert_eq!(error.kind, oq_media::ErrorKind::UnsupportedContent);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn linear_transfer_is_rejected() {
    let path = std::env::temp_dir().join(format!(
        "oq-vt-linear-{}.mp4",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let status = ffmpeg()
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=64x64:rate=1:duration=1",
            "-c:v",
            "libx264",
            "-color_trc",
            "linear",
            "-pix_fmt",
            "yuv420p",
            "-y",
        ])
        .arg(&path)
        .status()
        .expect("packaged ffmpeg is required for this test");
    assert!(status.success());
    let mut session = Session::open(&path).unwrap();
    let error = match session.next_frame() {
        Err(error) => error,
        Ok(_) => panic!("linear frame was accepted"),
    };
    assert_eq!(error.kind, oq_media::ErrorKind::UnsupportedContent);
    std::fs::remove_file(path).unwrap();
}

// Generated in-repo test signal, licensed MIT with this crate. No third-party
// footage. SHA-256: dc6d398a48575e0e3d731c9b28aa8eb1e872a0abad83fa09783213bc04c9b5ee
// Reproduce (bundled ffmpeg):
// ffmpeg -v error -f lavfi -i 'nullsrc=size=64x64:rate=10:duration=4,geq=lum=16+5*N:cb=128:cr=128' \
//   -vf 'select=not(eq(mod(n\,7)\,3)),setpts=PTS+2/TB' -vsync vfr \
//   -c:v libx264 -preset medium -crf 12 -g 12 -keyint_min 12 -bf 3 \
//   -x264-params b-adapt=0:scenecut=0 -pix_fmt yuv420p -color_primaries bt709 \
//   -color_trc bt709 -colorspace bt709 -video_track_timescale 1000 source-time.mp4
fn source_time_fixture() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/source-time.mp4")
}

struct OracleFrame {
    pts: i64,
    luma_sum: u64,
}

// Independent software decode: showinfo reports presentation PTS; extracted
// luma yields a numeric checksum that distinguishes every generated marker.
fn software_oracle() -> Vec<OracleFrame> {
    let output = ffmpeg()
        .args(["-v", "info", "-copyts", "-c:v", "h264", "-i"])
        .arg(source_time_fixture())
        .args([
            "-an",
            "-vf",
            "showinfo,extractplanes=y",
            "-vsync",
            "0",
            "-f",
            "rawvideo",
            "pipe:1",
        ])
        .output()
        .expect("packaged FFmpeg software decoder is a required test prerequisite");
    let diagnostics = String::from_utf8(output.stderr).unwrap();
    assert!(output.status.success(), "{diagnostics}");
    assert!(diagnostics.contains("time_base: 1/1000"), "{diagnostics}");
    assert!(
        diagnostics.lines().any(|line| line.contains("type:B")),
        "fixture must contain reordered B frames"
    );
    let pts: Vec<i64> = diagnostics
        .lines()
        .filter(|line| line.contains(" n:") && line.contains(" pts:"))
        .map(|line| {
            line.split(" pts:")
                .nth(1)
                .unwrap()
                .split_whitespace()
                .next()
                .unwrap()
                .parse()
                .unwrap()
        })
        .collect();
    assert_eq!(pts.len(), 34);
    assert_eq!(
        pts.first(),
        Some(&2000),
        "fixture must retain its nonzero stream origin"
    );
    assert!(
        pts.windows(2).any(|pair| pair[1] - pair[0] == 200),
        "fixture must be VFR"
    );
    assert_eq!(output.stdout.len(), pts.len() * 64 * 64);
    pts.into_iter()
        .zip(output.stdout.chunks_exact(64 * 64))
        .map(|(pts, luma)| OracleFrame {
            pts,
            luma_sum: luma.iter().map(|value| u64::from(*value)).sum(),
        })
        .collect()
}

#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
    fn CVPixelBufferLockBaseAddress(buffer: *mut std::ffi::c_void, flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(buffer: *mut std::ffi::c_void, flags: u64) -> i32;
    fn CVPixelBufferGetBaseAddressOfPlane(buffer: *mut std::ffi::c_void, plane: usize)
        -> *const u8;
    fn CVPixelBufferGetBytesPerRowOfPlane(buffer: *mut std::ffi::c_void, plane: usize) -> usize;
}

// Readback is strictly a test oracle, never a production pixel-export fallback.
fn hardware_luma_sum(frame: &oq_media::VideoToolboxFrame) -> u64 {
    assert_eq!(frame.format(), oq_media::PixelFormat::Nv12);
    let buffer = frame.pixel_buffer();
    unsafe {
        assert_eq!(CVPixelBufferLockBaseAddress(buffer, 1), 0);
        let base = CVPixelBufferGetBaseAddressOfPlane(buffer, 0);
        let stride = CVPixelBufferGetBytesPerRowOfPlane(buffer, 0);
        let sum = (0..frame.height() as usize)
            .map(|row| {
                std::slice::from_raw_parts(base.add(row * stride), frame.width() as usize)
                    .iter()
                    .map(|value| u64::from(*value))
                    .sum::<u64>()
            })
            .sum();
        assert_eq!(CVPixelBufferUnlockBaseAddress(buffer, 1), 0);
        sum
    }
}

fn assert_oracle(
    selected: &oq_media::SelectedFrame,
    expected: &OracleFrame,
    target: i64,
    epoch: u64,
) {
    assert_eq!(selected.target_ns, target);
    assert_eq!(selected.actual_ns, (expected.pts - 2000) * 1_000_000);
    let raw = selected
        .frame
        .frame
        .timestamp()
        .expect("original presentation PTS is required");
    assert_eq!(raw.pts, expected.pts);
    assert_eq!(
        raw.time_base,
        oq_media::RationalTimeBase {
            numerator: 1,
            denominator: 1000
        }
    );
    assert_eq!(selected.frame.frame.pts_ns(), Some(selected.actual_ns));
    assert_eq!(selected.frame.timeline_epoch, epoch);
    assert_eq!(
        selected.selection,
        if selected.actual_ns == target {
            oq_media::FrameSelection::Exact
        } else {
            oq_media::FrameSelection::Following
        }
    );
    // Hardware and software H.264 rounding may differ by one luma code;
    // adjacent marker frames differ by five, making this oracle discriminating.
    assert!(hardware_luma_sum(&selected.frame.frame).abs_diff(expected.luma_sum) <= 64 * 64);
}

#[test]
fn source_time_access_matches_software_pts_and_frame_markers() {
    let oracle = software_oracle();
    let mut session = Session::open(source_time_fixture()).unwrap();
    let info = session.source_info().clone();
    assert_eq!(info.start_pts, Some(2000));
    assert!(info.duration_pts.is_some());
    assert!(info.average_frame_rate.is_some());
    assert!(info.seekable);
    let mut source_epoch = None;
    for target in [
        0,
        1,
        499_999_999,
        500_000_000,
        500_000_001,
        1_000_000_000,
        1_999_999_999,
        2_000_000_000,
        2_000_000_001,
        3_900_000_000,
        3_900_000_001,
    ] {
        let expected = oracle
            .iter()
            .position(|frame| (frame.pts - 2000) * 1_000_000 >= target);
        let selected = session.frame_at_ns(target, 0).unwrap();
        match (expected, selected) {
            (Some(index), Some(selected)) => {
                assert_oracle(&selected, &oracle[index], target, 0);
                assert_eq!(
                    selected.frame.sequence,
                    index as u64 + 1,
                    "forward access must preserve the sequential decoder cursor"
                );
                assert_eq!(
                    *source_epoch.get_or_insert(selected.frame.source_epoch),
                    selected.frame.source_epoch
                );
                let repeated = session.frame_at_ns(target, 0).unwrap().unwrap();
                assert_eq!(
                    repeated.frame.sequence, selected.frame.sequence,
                    "repeated target must not decode/seek"
                );
                assert_eq!(
                    repeated.frame.frame.pixel_buffer(),
                    selected.frame.frame.pixel_buffer()
                );
            }
            (None, None) => {}
            _ => panic!("source-time selection disagrees with independent oracle at {target}"),
        }
    }
    assert!(session.frame_at_ns(4_000_000_000, 0).unwrap().is_none());
    assert!(session.frame_at_ns(4_000_000_000, 0).unwrap().is_none());
    let error = session.frame_at_ns(500_000_000, 0).err().unwrap();
    assert_eq!(error.kind, oq_media::ErrorKind::DiscontinuousTarget);
    for (epoch, target) in [
        (1, 2_000_000_000),
        (2, 500_000_000),
        (3, 0),
        (4, 2_000_000_001),
    ] {
        let selected = session.frame_at_ns(target, epoch).unwrap().unwrap();
        let expected = oracle
            .iter()
            .find(|frame| (frame.pts - 2000) * 1_000_000 >= target)
            .unwrap();
        assert_oracle(&selected, expected, target, epoch);
        assert_eq!(Some(selected.frame.source_epoch), source_epoch);
    }
    assert_eq!(
        session.frame_at_ns(0, 3).err().unwrap().kind,
        oq_media::ErrorKind::Cancelled
    );
    assert_eq!(
        session.frame_at_ns(-1, 4).err().unwrap().kind,
        oq_media::ErrorKind::InvalidTarget
    );
    session.reset().unwrap();
    let first = session.frame_at_ns(0, 5).unwrap().unwrap();
    assert_oracle(&first, &oracle[0], 0, 5);
    assert_eq!(Some(first.frame.source_epoch), source_epoch);
    // Sequential delivery after source-time selection resumes at the next frame.
    let next = session.next_frame().unwrap().unwrap();
    assert_eq!(next.frame.timestamp().unwrap().pts, oracle[1].pts);
    assert_eq!(next.sequence, 2);
}

#[test]
fn source_time_cancellation_and_malformed_input_are_errors() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut session = Session::open_with_cancel(source_time_fixture(), cancelled.clone()).unwrap();
    session.frame_at_ns(500_000_000, 0).unwrap().unwrap();
    cancelled.store(true, Ordering::Release);
    assert_eq!(
        session.frame_at_ns(500_000_000, 0).err().unwrap().kind,
        oq_media::ErrorKind::Cancelled,
        "cache must not bypass cancellation"
    );
    assert_eq!(
        session.frame_at_ns(0, 1).err().unwrap().kind,
        oq_media::ErrorKind::Cancelled,
        "seek must not bypass cancellation"
    );
    let path = std::env::temp_dir().join(format!(
        "oq-malformed-{}.mp4",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&path, b"not a media container").unwrap();
    assert!(Session::open(&path).is_err());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn non_regular_source_is_rejected_before_seeking() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let fifo = std::env::temp_dir().join(format!("oq-nonseekable-{stamp}.fifo"));
    assert!(Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap()
        .success());
    assert!(Session::open(&fifo).is_err());
    std::fs::remove_file(fifo).unwrap();
}
