#![cfg(all(target_os = "macos", feature = "videotoolbox"))]

use oq_media::{
    EncodeAdmission, EncodeFrame, EncoderConfig, EncoderStatus, EncoderSurface, Mp4Encoder,
    RationalTimeBase,
};
use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

static NEXT: AtomicU64 = AtomicU64::new(1);
#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
    fn CVPixelBufferLockBaseAddress(buffer: *mut c_void, flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(buffer: *mut c_void, flags: u64) -> i32;
    fn CVPixelBufferGetBaseAddressOfPlane(buffer: *mut c_void, plane: usize) -> *mut u8;
    fn CVPixelBufferGetBytesPerRowOfPlane(buffer: *mut c_void, plane: usize) -> usize;
}

fn temp_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "oq-{label}-{}-{}.mp4",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}
fn config(overwrite: bool) -> EncoderConfig {
    EncoderConfig {
        width: 64,
        height: 64,
        time_base: RationalTimeBase {
            numerator: 1,
            denominator: 30,
        },
        bitrate: 300_000,
        queue_capacity: 3,
        max_b_frames: 0,
        overwrite,
    }
}
fn black_surface() -> EncoderSurface {
    let surface = EncoderSurface::new(64, 64).unwrap();
    let buffer = surface.pixel_buffer();
    unsafe {
        assert_eq!(CVPixelBufferLockBaseAddress(buffer, 0), 0);
        let y = CVPixelBufferGetBaseAddressOfPlane(buffer, 0);
        let y_stride = CVPixelBufferGetBytesPerRowOfPlane(buffer, 0);
        let uv = CVPixelBufferGetBaseAddressOfPlane(buffer, 1);
        let uv_stride = CVPixelBufferGetBytesPerRowOfPlane(buffer, 1);
        assert!(!y.is_null() && !uv.is_null());
        for row in 0..64 {
            std::ptr::write_bytes(y.add(row * y_stride), 16, 64);
        }
        for row in 0..32 {
            std::ptr::write_bytes(uv.add(row * uv_stride), 128, 64);
        }
        assert_eq!(CVPixelBufferUnlockBaseAddress(buffer, 0), 0);
    }
    surface
}

#[test]
fn hardware_encoder_writes_and_drains_mp4_with_expected_packets() {
    let path = temp_path("encode");
    let mut encoder =
        Mp4Encoder::open(&path, config(false)).expect("required VideoToolbox encoder must open");
    for pts in 0..3 {
        let mut pending = Some(EncodeFrame {
            surface: black_surface(),
            pts,
            duration: 1,
        });
        while let Some(frame) = pending.take() {
            match encoder.submit(frame).unwrap() {
                EncodeAdmission::Accepted => {}
                EncodeAdmission::WouldBlock(frame) => {
                    pending = Some(frame);
                    thread::sleep(Duration::from_millis(1));
                }
            }
        }
    }
    while !encoder.finish().unwrap() {
        thread::sleep(Duration::from_millis(1));
    }
    let report = loop {
        match encoder.poll().unwrap() {
            EncoderStatus::ReadyToCommit(report) => break report,
            EncoderStatus::Failed(error) => panic!("hardware MP4 encode failed: {error}"),
            EncoderStatus::Cancelled => panic!("encoder cancelled unexpectedly"),
            _ => thread::sleep(Duration::from_millis(1)),
        }
    };
    assert_eq!(report.input_frames, 3);
    assert_eq!(report.packets, 3);
    assert!(report.hardware);
    assert!(encoder.commit().unwrap());
    loop {
        match encoder.poll().unwrap() {
            EncoderStatus::Completed(final_report) => {
                assert_eq!(final_report.packets, 3);
                break;
            }
            EncoderStatus::Failed(error) => panic!("MP4 commit failed: {error}"),
            EncoderStatus::Cancelled => panic!("encoder cancelled during commit"),
            _ => thread::sleep(Duration::from_millis(1)),
        }
    }
    let mut input =
        ffmpeg_next::format::input(&path).expect("committed file must be a readable MP4");
    let stream = input
        .streams()
        .best(ffmpeg_next::media::Type::Video)
        .expect("MP4 video stream");
    let stream_index = stream.index();
    let stream_tb = stream.time_base();
    let mut pts = Vec::new();
    for (packet_stream, packet) in input.packets() {
        if packet_stream.index() == stream_index {
            pts.push(packet.pts().expect("packet PTS"));
        }
    }
    assert_eq!(pts.len(), 3);
    let input_pts: Vec<i64> = pts
        .into_iter()
        .map(|value| {
            let seconds = value as f64 * f64::from(stream_tb.0) / f64::from(stream_tb.1);
            (seconds * 30.0).round() as i64
        })
        .collect();
    assert_eq!(input_pts, vec![0, 1, 2]);
    drop(input);
    drop(encoder);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn cancellation_preserves_preexisting_target() {
    let path = temp_path("cancel");
    let prior = b"preserve existing export";
    std::fs::write(&path, prior).unwrap();
    let mut cfg = config(true);
    let encoder =
        Mp4Encoder::open(&path, cfg.clone()).expect("required VideoToolbox encoder must open");
    encoder.cancel();
    loop {
        match encoder.poll().unwrap() {
            EncoderStatus::Cancelled => break,
            EncoderStatus::Failed(error) => panic!("encoder failed before cancellation: {error}"),
            _ => thread::sleep(Duration::from_millis(1)),
        }
    }
    assert_eq!(std::fs::read(&path).unwrap(), prior);
    cfg.overwrite = false;
    assert!(Mp4Encoder::open(&path, cfg).is_err());
    std::fs::remove_file(path).unwrap();
}
