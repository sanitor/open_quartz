use super::*;
use crate::media::native::{find_ffmpeg, NativeVideoSourceKind};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn source_upload_retries_failed_callback_and_retention_drops_decoder() {
    let Ok(ffmpeg) = find_ffmpeg() else { return; };
    let directory = std::env::temp_dir().join(format!(
        "open-quartz-sources-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("source.mp4");
    let status = Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "color=c=blue:s=16x16:r=5", "-frames:v", "2", "-pix_fmt", "yuv420p", "-y"])
        .arg(&path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let mut sources = VideoSources::new();
    sources.attach("video", NativeVideoConfig {
        kind: NativeVideoSourceKind::File,
        source: path.to_string_lossy().into_owned(),
        looping: false,
        playback_rate: 1.0,
    }, true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let result = sources.upload_frames(|_, _, _, _| Err("retry upload".to_owned()));
        if let Err(error) = result {
            assert_eq!(error, "retry upload");
            break;
        }
        assert!(Instant::now() < deadline, "preview frame did not arrive");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(sources.metrics("video").unwrap().uploaded_frames, 0);
    let dirty = sources.upload_frames(|node_id, frame, width, height| {
        assert_eq!(node_id, "video");
        assert_eq!((width, height), (16, 16));
        if let NativeVideoFrame::Rgba(rgba) = frame {
            assert_eq!(rgba.len(), 16 * 16 * 4);
        }
        Ok(())
    }).unwrap();
    assert_eq!(dirty, ["video"]);
    assert_eq!(sources.metrics("video").unwrap().uploaded_frames, 1);
    assert!(sources.upload_frames(|_, _, _, _| panic!("uploaded frame repeated")).unwrap().is_empty());
    sources.pause();
    sources.retain(&HashSet::new());
    assert!(sources.node_ids().is_empty());
    assert!(!sources.is_active());
    assert_eq!(sources.frame_counts(), (0, 0));
    assert!(sources.metrics("video").is_err());
    sources.attach("video", NativeVideoConfig {
        kind: NativeVideoSourceKind::File,
        source: path.to_string_lossy().into_owned(),
        looping: false,
        playback_rate: 1.0,
    }, true).unwrap();
    assert_eq!(sources.node_ids(), ["video"]);
    sources.clear();
    assert!(sources.node_ids().is_empty());
    assert!(!sources.is_active());
    assert!(sources.metrics("video").is_err());
    std::fs::remove_dir_all(directory).unwrap();
}
