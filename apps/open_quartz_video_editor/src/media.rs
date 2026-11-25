use crate::document::{fingerprint, new_id, MediaAsset, Rational};
#[cfg(any(target_os = "macos", windows))]
use crate::document::AudioAsset;
#[cfg(any(target_os = "macos", windows))]
use crate::document::HdrTransfer;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits, RgbaImage};
use std::path::{Path, PathBuf};

const MAX_STILL_EDGE: u32 = 8192;

/// Validate the first video frame on hardware or decode a still image for import.
pub fn probe(path: &Path) -> Result<MediaAsset, String> {
    let canonical = path.canonicalize().map_err(|error| format!("Cannot access media: {error}"))?;
    let metadata = std::fs::metadata(&canonical).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("Select a regular local media file".into());
    }
    if still_format(&canonical).is_some() {
        let image = decode_still(&canonical)?;
        return Ok(MediaAsset {
            id: new_id("asset", 0),
            name: canonical.file_name().unwrap_or_default().to_string_lossy().into_owned(),
            path: canonical.to_string_lossy().into_owned(),
            fingerprint: fingerprint(&canonical)?,
            size_bytes: metadata.len(),
            still: true,
            width: image.width(),
            height: image.height(),
            duration_pts: None,
            start_pts: None,
            time_base: Rational::new(1, 1)?,
            fps: None,
            camera_log: None, hdr_transfer: None,
            has_audio: false,
            audio: None,
            online: true,
        });
    }
    #[cfg(any(target_os = "macos", windows))]
    let (audio, name, stored_path) = {
        let streams = oq_media::probe_audio(&canonical).map_err(|error| error.to_string())?;
        let audio = streams.audio.map(audio_asset).transpose()?;
        let name = canonical.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let path = canonical.to_string_lossy().into_owned();
        if !streams.has_video {
            let audio = audio.ok_or("Media file has no supported AAC or PCM stream")?;
            if audio.duration_pts.is_none() {
                return Err("Audio-only files require a known duration".into());
            }
            return Ok(MediaAsset {
                id: new_id("asset", 0), name, path, fingerprint: fingerprint(&canonical)?,
                size_bytes: metadata.len(), still: false, width: 0, height: 0, duration_pts: audio.duration_pts,
                start_pts: Some(audio.start_pts), time_base: audio.time_base, fps: None,
                camera_log: None, hdr_transfer: None, has_audio: true, audio: Some(audio), online: true,
            });
        }
        (audio, name, path)
    };
    #[cfg(any(target_os = "macos", windows))]
    {
        let mut session = oq_media::Session::open(&canonical).map_err(|e| e.to_string())?;
        let info = session.source_info().clone();
        if !matches!(info.codec.as_str(), "h264" | "hevc") {
            return Err("Supported visual files are PNG, JPEG, H.264 and HEVC".into());
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
        if !matches!(frame.frame.frame.format(), oq_media::PixelFormat::Nv12 | oq_media::PixelFormat::P010) {
            return Err("The editor requires an NV12 or P010 hardware video surface".into());
        }
        Ok(MediaAsset {
            id: new_id("asset", 0),
            name,
            path: stored_path,
            fingerprint: fingerprint(&canonical)?,
            size_bytes: metadata.len(),
            still: false,
            width: frame.frame.frame.width(),
            height: frame.frame.frame.height(),
            duration_pts: info.duration_pts,
            start_pts: info.start_pts,
            time_base: Rational::new(info.time_base.numerator, info.time_base.denominator)?,
            fps: info
                .average_frame_rate
                .map(|fps| Rational::new(fps.numerator, fps.denominator))
                .transpose()?,
            camera_log: None,
            hdr_transfer: match frame.frame.frame.color().transfer {
                oq_media::TransferFunction::Pq => Some(HdrTransfer::Pq),
                oq_media::TransferFunction::Hlg => Some(HdrTransfer::Hlg),
                _ => None,
            },
            has_audio: info.has_audio,
            audio,
            online: true,
        })
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Err("Precise video editing requires macOS VideoToolbox or Windows D3D12VA".into())
    }
}

fn still_format(path: &Path) -> Option<ImageFormat> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some(ImageFormat::Png),
        "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
        _ => None,
    }
}

pub(crate) fn decode_still(path: &Path) -> Result<RgbaImage, String> {
    let decode_error = |error| format!("Cannot decode still image: {error}");
    let mut reader = ImageReader::open(path)
        .and_then(ImageReader::with_guessed_format)
        .map_err(|error| error.to_string())?;
    if !matches!(reader.format(), Some(ImageFormat::Png | ImageFormat::Jpeg)) {
        return Err("Cannot decode still image. Select a PNG or JPEG file".into());
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_STILL_EDGE);
    limits.max_image_height = Some(MAX_STILL_EDGE);
    reader.limits(limits.clone());
    let mut decoder = reader.into_decoder().map_err(decode_error)?;
    let (width, height) = decoder.dimensions();
    limits.check_dimensions(width, height).map_err(decode_error)?;
    let orientation = decoder.orientation().map_err(decode_error)?;
    let mut image = DynamicImage::from_decoder(decoder).map_err(decode_error)?;
    image.apply_orientation(orientation);
    Ok(image.into_rgba8())
}

#[cfg(any(target_os = "macos", windows))]
fn audio_asset(info: oq_media::AudioSourceInfo) -> Result<AudioAsset, String> {
    Ok(AudioAsset {
        stream: info.stream_index,
        time_base: Rational::new(info.time_base.numerator, info.time_base.denominator)?,
        start_pts: info.start_pts, duration_pts: info.duration_pts, rate: info.sample_rate,
        channels: info.channels, codec: info.codec,
    })
}

#[cfg(any(target_os = "macos", windows))]
pub fn audio_metadata(path: &Path) -> Result<Option<AudioAsset>, String> {
    oq_media::probe_audio(path).map_err(|error| error.to_string())?.audio.map(audio_asset).transpose()
}

#[cfg(target_os = "macos")]
pub fn proxy_encoder_path() -> Result<PathBuf, String> {
    let installed = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|parent| parent.join("../Resources/runtime/ffmpeg")));
    let development = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../open_quartz_slint/runtime/ffmpeg-media/bin/ffmpeg");
    installed
        .into_iter()
        .chain([development])
        .find(|path| path.is_file())
        .ok_or_else(|| "Proxy encoder unavailable. Run npm run prepare:desktop".into())
}

#[cfg(windows)]
pub fn proxy_encoder_path() -> Result<PathBuf, String> {
    open_quartz::media::native::find_ffmpeg()
}

pub fn configure_runtime() -> Result<(), String> {
    if std::env::var_os("OPEN_QUARTZ_FFMPEG_PATH").is_some() {
        return Ok(());
    }
    let mut candidates = Vec::<PathBuf>::new();
    let ffmpeg = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    if let Ok(executable) = std::env::current_exe() {
        if let Some(parent) = executable.parent() {
            #[cfg(windows)]
            candidates.push(parent.join(ffmpeg));
            candidates.push(parent.join("runtime").join(ffmpeg));
            candidates.push(parent.join("../Resources/runtime").join(ffmpeg));
        }
    }
    // Development-only resolution; distributed runtime is owned by the application bundle.
    candidates
        .push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../open_quartz_slint/runtime").join(ffmpeg));
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

#[cfg(all(test, any(target_os = "macos", windows)))]
mod tests {
    use super::*;
    #[test]
    fn probe_records_unused_audio_and_rejects_corrupt_media() {
        configure_runtime().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("audio-video.mp4");
        let video = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/white_h264_high_64x64.mp4");
        let ffmpeg = open_quartz::media::native::find_ffmpeg().unwrap();
        let output = std::process::Command::new(ffmpeg)
            .env_remove("DYLD_LIBRARY_PATH")
            .env_remove("DYLD_FALLBACK_LIBRARY_PATH")
            .args(["-hide_banner", "-loglevel", "error", "-i"])
            .arg(video)
            .args([
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=44100",
                "-t",
                "1",
                "-c:v",
                "copy",
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
