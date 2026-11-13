use crate::document::{fingerprint, new_id, MediaAsset, Rational};
use std::path::{Path, PathBuf};

/// Preparation-only metadata and first-frame capability check, never software pixels.
pub fn probe(path: &Path) -> Result<MediaAsset, String> {
    #[cfg(target_os = "macos")]
    {
        let canonical = path
            .canonicalize()
            .map_err(|error| format!("Cannot access media: {error}"))?;
        if !canonical.is_file() {
            return Err("Select a regular local video file".into());
        }
        let metadata = std::fs::metadata(&canonical).map_err(|e| e.to_string())?;
        let mut session = oq_media::Session::open(&canonical).map_err(|e| e.to_string())?;
        let info = session.source_info().clone();
        if !matches!(info.codec.as_str(), "h264" | "hevc") {
            return Err("This editor currently supports H.264 and HEVC video only".into());
        }
        if !info.seekable
            || info.start_pts.is_none()
            || info
                .duration_ns()
                .map_err(|e| e.to_string())?
                .is_none_or(|duration| duration <= 0)
        {
            return Err(
                "Precise editing requires a seekable file with known origin and duration".into(),
            );
        }
        let frame = session
            .frame_at_ns(0, 1)
            .map_err(|e| e.to_string())?
            .ok_or("Video file has no decodable presentation frame")?;
        if frame.frame.frame.format() != oq_media::PixelFormat::Nv12 {
            return Err("The first editor workflow accepts 8-bit SDR video only".into());
        }
        Ok(MediaAsset {
            id: new_id("asset", 0),
            name: canonical
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            path: canonical.to_string_lossy().into_owned(),
            fingerprint: fingerprint(&canonical)?,
            size_bytes: metadata.len(),
            width: frame.frame.frame.width(),
            height: frame.frame.frame.height(),
            duration_pts: info.duration_pts,
            start_pts: info.start_pts,
            time_base: Rational::new(info.time_base.numerator, info.time_base.denominator)?,
            fps: info
                .average_frame_rate
                .map(|fps| Rational::new(fps.numerator, fps.denominator))
                .transpose()?,
            has_audio: info.has_audio,
            online: true,
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        Err(
            "Precise resident editing currently requires macOS VideoToolbox; no CPU fallback"
                .into(),
        )
    }
}

pub fn configure_runtime() -> Result<(), String> {
    if std::env::var_os("OPEN_QUARTZ_FFMPEG_PATH").is_some() {
        return Ok(());
    }
    let mut candidates = Vec::<PathBuf>::new();
    if let Ok(executable) = std::env::current_exe() {
        if let Some(parent) = executable.parent() {
            candidates.push(parent.join("runtime/ffmpeg"));
            candidates.push(parent.join("../Resources/runtime/ffmpeg"));
        }
    }
    // Development-only resolution; distributed runtime is owned by the application bundle.
    candidates
        .push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../open_quartz_slint/runtime/ffmpeg"));
    if let Some(path) = candidates.into_iter().find(|candidate| candidate.is_file()) {
        std::env::set_var("OPEN_QUARTZ_FFMPEG_PATH", path);
        Ok(())
    } else {
        Err(
            "Media runtime not found. Run npm run prepare:desktop before starting the editor"
                .into(),
        )
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    #[test]
    fn probe_records_unused_audio_and_rejects_corrupt_media() {
        configure_runtime().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("audio-video.mp4");
        let ffmpeg = open_quartz::media::native::find_ffmpeg().unwrap();
        let output = std::process::Command::new(ffmpeg)
            .env_remove("DYLD_LIBRARY_PATH")
            .env_remove("DYLD_FALLBACK_LIBRARY_PATH")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=white:s=64x64:r=30",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=44100",
                "-t",
                "1",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-y",
            ])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let asset = probe(&path).unwrap();
        assert!(asset.has_audio);
        assert_eq!((asset.width, asset.height), (64, 64));
        assert_eq!(asset.start_pts, Some(0));
        assert!(asset.online);
        let bad = directory.path().join("broken.mp4");
        std::fs::write(&bad, b"not a video").unwrap();
        assert!(probe(&bad).is_err());
    }
}
