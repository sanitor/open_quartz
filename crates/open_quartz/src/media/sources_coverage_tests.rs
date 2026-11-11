use super::*;
use crate::media::native::NativeVideoSourceKind;
#[cfg(any(target_os = "macos", all(windows, feature = "native-video-d3d12")))]
use std::time::{Duration, Instant};

#[test]
fn camera_preparation_rejects_cpu_fallback() {
    for preview in [false, true] {
        let error = PreparedVideoSource::open(NativeVideoConfig {
            kind: NativeVideoSourceKind::Camera,
            source: "__fail_reader_spawn__".to_owned(),
            looping: false,
            playback_rate: 1.0,
        }, preview).err().expect("camera preparation must reject CPU fallback");
        assert!(error.contains("camera capture"), "{error}");
        assert!(error.contains("CPU decode/upload fallback is disabled"), "{error}");
    }
}

#[cfg(not(any(target_os = "macos", all(windows, feature = "native-video-d3d12"))))]
#[test]
fn prepared_file_rejects_unsupported_backend_before_file_access() {
    let error = PreparedVideoSource::open(NativeVideoConfig {
        kind: NativeVideoSourceKind::File,
        source: "missing".to_owned(),
        looping: false,
        playback_rate: 1.0,
    }, true).err().expect("unsupported backend must reject before file access");
    assert!(error.contains("no hardware-resident video decoder"), "{error}");
}

#[cfg(any(target_os = "macos", all(windows, feature = "native-video-d3d12")))]
#[test]
fn hardware_source_upload_retries_and_tracks_lifecycle() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/white_64x64.mp4");
    if !path.is_file() { return; }
    let config = NativeVideoConfig {
        kind: NativeVideoSourceKind::File,
        source: path.to_string_lossy().into_owned(),
        looping: false,
        playback_rate: 1.0,
    };
    let mut sources = VideoSources::new();
    let prepared = PreparedVideoSource::open(config.clone(), true)
        .expect("fixture must open through the real hardware backend, never software fallback");
    let info = sources.attach_prepared("video", prepared);
    assert_eq!((info.width, info.height), (64, 64));
    assert!(info.fallback_reason.is_none());
    #[cfg(target_os = "macos")]
    assert_eq!(info.decoder, "ffmpeg-videotoolbox");
    #[cfg(all(windows, feature = "native-video-d3d12"))]
    assert_eq!(info.decoder, "ffmpeg-d3d12va");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let result = sources.upload_frames(|_, _, _, _| Err("retry upload".to_owned()));
        if let Err(error) = result {
            assert_eq!(error, "retry upload");
            break;
        }
        assert!(Instant::now() < deadline, "hardware preview frame did not arrive");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(sources.metrics("video").unwrap().uploaded_frames, 0);
    let dirty = sources.upload_frames(|node_id, frame, width, height| {
        assert_eq!(node_id, "video");
        assert_eq!((width, height), (64, 64));
        match frame {
            NativeVideoFrame::Rgba(_) => panic!("production source delivered a CPU frame"),
            #[cfg(target_os = "macos")]
            NativeVideoFrame::VideoToolbox(frame) => {
                assert_eq!((frame.width(), frame.height()), (width, height));
                assert!(!frame.pixel_buffer().is_null());
            }
            #[cfg(all(windows, feature = "native-video-d3d12"))]
            NativeVideoFrame::D3d12(frame) => assert_eq!((frame.width, frame.height), (width, height)),
        }
        Ok(())
    }).unwrap();
    assert_eq!(dirty, ["video"]);
    let metrics = sources.metrics("video").unwrap();
    assert_eq!(metrics.uploaded_frames, 1);
    assert_eq!(metrics.cpu_copy_bytes, 0);
    assert!(metrics.hardware_decode);
    assert!(sources.upload_frames(|_, _, _, _| panic!("uploaded frame repeated")).unwrap().is_empty());
    sources.pause();
    sources.retain(&HashSet::new());
    assert!(sources.node_ids().is_empty());
    assert!(!sources.is_active());
    assert_eq!(sources.frame_counts(), (0, 0));
    assert!(sources.metrics("video").is_err());
    sources.attach_prepared("video", PreparedVideoSource::open(config, true).unwrap());
    assert_eq!(sources.node_ids(), ["video"]);
    sources.clear();
    assert!(sources.node_ids().is_empty());
    assert!(!sources.is_active());
    assert!(sources.metrics("video").is_err());
}
