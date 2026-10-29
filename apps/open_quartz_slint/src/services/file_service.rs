use std::path::Path;
use std::sync::Arc;

use base64::Engine as _;

pub fn read_project(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map_err(|error| format!("Cannot read {}: {error}", path.display()))
}

pub fn write_project(path: &Path, contents: &str) -> Result<(), String> {
    std::fs::write(path, contents)
        .map_err(|error| format!("Cannot save {}: {error}", path.display()))
}

const MAX_ENCODED_IMAGE_BYTES: usize = 64 * 1024 * 1024;
const MAX_DECODED_IMAGE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_RAW_FRAMEBUFFER_BYTES: usize = 256 * 1024 * 1024;
const MAX_IMAGE_DIMENSION: u32 = 8192;

#[derive(Clone)]
pub struct ImageAsset {
    pub data_url: String,
    pub file_name: String,
    pub width: u32,
    pub height: u32,
    pub key: u64,
    pub rgba: Arc<Vec<u8>>,
}

pub fn read_image(path: &Path) -> Result<ImageAsset, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("Cannot read image {}: {error}", path.display()))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| format!("Image path has no valid UTF-8 filename: {}", path.display()))?;
    decode_image(&bytes, file_name.to_owned())
}

pub fn decode_image_data_url(data_url: &str, file_name: String) -> Result<ImageAsset, String> {
    let (metadata, encoded) = data_url
        .split_once(',')
        .ok_or_else(|| "Image resource is not a valid data URL".to_owned())?;
    let mime = metadata
        .strip_prefix("data:")
        .and_then(|metadata| metadata.strip_suffix(";base64"))
        .ok_or_else(|| "Image resource must be a base64 data URL".to_owned())?;
    if !matches!(
        mime,
        "image/png" | "image/jpeg" | "image/bmp" | "image/webp"
    ) {
        return Err(format!("Unsupported image MIME type: {mime}"));
    }
    if encoded.len() > MAX_ENCODED_IMAGE_BYTES.saturating_mul(4) / 3 + 4 {
        return Err("Image resource exceeds the 64 MiB encoded file limit".to_owned());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| format!("Cannot decode image resource base64: {error}"))?;
    let mut asset = decode_image(&bytes, file_name)?;
    let actual_mime = asset
        .data_url
        .strip_prefix("data:")
        .and_then(|value| value.split_once(';'))
        .map(|(mime, _)| mime)
        .unwrap_or_default();
    if actual_mime != mime {
        return Err(format!(
            "Image resource MIME type {mime} does not match its encoded format"
        ));
    }
    asset.data_url = data_url.to_owned();
    Ok(asset)
}

fn decode_image(bytes: &[u8], file_name: String) -> Result<ImageAsset, String> {
    if bytes.len() > MAX_ENCODED_IMAGE_BYTES {
        return Err("Image file exceeds the 64 MiB limit".to_owned());
    }
    let format = image::guess_format(bytes)
        .map_err(|error| format!("Cannot identify image format: {error}"))?;
    if !matches!(
        format,
        image::ImageFormat::Png
            | image::ImageFormat::Jpeg
            | image::ImageFormat::Bmp
            | image::ImageFormat::WebP
    ) {
        return Err(format!("Unsupported image format: {format:?}"));
    }
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    limits.max_alloc = Some(MAX_DECODED_IMAGE_BYTES);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|error| format!("Cannot decode image: {error}"))?;
    let rgba = decoded.into_rgba8();
    let (width, height) = rgba.dimensions();
    let data_url = format!(
        "data:{};base64,{}",
        format.to_mime_type(),
        base64::engine::general_purpose::STANDARD.encode(bytes),
    );
    let key = resource_key(&(format as u8, width, height, &data_url));
    Ok(ImageAsset {
        data_url,
        file_name,
        width,
        height,
        key,
        rgba: Arc::new(rgba.into_raw()),
    })
}

#[derive(Clone)]
pub struct FramebufferAsset {
    pub data_url: String,
    pub file_name: String,
    pub format: open_quartz_sdk::FramebufferFormat,
    pub width: u32,
    pub height: u32,
    pub stride: Option<u32>,
    pub key: u64,
    pub rgba: Arc<Vec<u8>>,
}

pub fn read_framebuffer(
    path: &Path,
    format: open_quartz_sdk::FramebufferFormat,
    width: u32,
    height: u32,
    stride: Option<u32>,
) -> Result<FramebufferAsset, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("Cannot read framebuffer {}: {error}", path.display()))?;
    if bytes.len() > MAX_RAW_FRAMEBUFFER_BYTES {
        return Err("Framebuffer file exceeds the 256 MiB limit".to_owned());
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            format!(
                "Framebuffer path has no valid UTF-8 filename: {}",
                path.display()
            )
        })?
        .to_owned();
    let data_url = format!(
        "data:application/octet-stream;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    );
    let rgba = decode_framebuffer_bytes(&bytes, format, width, height, stride)?;
    let key = resource_key(&(format as u8, width, height, stride, &data_url));
    Ok(FramebufferAsset {
        data_url,
        file_name,
        format,
        width,
        height,
        stride,
        key,
        rgba: Arc::new(rgba),
    })
}

pub fn decode_framebuffer_data_url(
    data_url: &str,
    file_name: String,
    format: open_quartz_sdk::FramebufferFormat,
    width: u32,
    height: u32,
    stride: Option<u32>,
) -> Result<FramebufferAsset, String> {
    let (metadata, encoded) = data_url
        .split_once(',')
        .ok_or_else(|| "Raw framebuffer resource is not a data URL".to_owned())?;
    if metadata != "data:application/octet-stream;base64" {
        return Err("Raw framebuffer resource must use application/octet-stream base64".to_owned());
    }
    if encoded.len() > MAX_RAW_FRAMEBUFFER_BYTES.saturating_mul(4) / 3 + 4 {
        return Err("Framebuffer resource exceeds the 256 MiB file limit".to_owned());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| format!("Cannot decode framebuffer base64: {error}"))?;
    if bytes.len() > MAX_RAW_FRAMEBUFFER_BYTES {
        return Err("Framebuffer resource exceeds the 256 MiB file limit".to_owned());
    }
    let rgba = decode_framebuffer_bytes(&bytes, format, width, height, stride)?;
    let key = resource_key(&(format as u8, width, height, stride, data_url));
    Ok(FramebufferAsset {
        data_url: data_url.to_owned(),
        file_name,
        format,
        width,
        height,
        stride,
        key,
        rgba: Arc::new(rgba),
    })
}

fn decode_framebuffer_bytes(
    bytes: &[u8],
    format: open_quartz_sdk::FramebufferFormat,
    width: u32,
    height: u32,
    stride: Option<u32>,
) -> Result<Vec<u8>, String> {
    if width == 0 || height == 0 {
        return Err("Framebuffer width and height must be positive".to_owned());
    }
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| "Framebuffer dimensions overflow addressable memory".to_owned())?;
    let output_len = pixels
        .checked_mul(4)
        .filter(|length| *length as u64 <= MAX_DECODED_IMAGE_BYTES)
        .ok_or_else(|| "Decoded framebuffer exceeds the 256 MiB limit".to_owned())?;
    let (bytes_per_pixel, channels, is_float) = match format {
        open_quartz_sdk::FramebufferFormat::Rgba8 => (4, 4, false),
        open_quartz_sdk::FramebufferFormat::Rgba32f => (16, 4, true),
        open_quartz_sdk::FramebufferFormat::Rg8 => (2, 2, false),
        open_quartz_sdk::FramebufferFormat::Rg32f => (8, 2, true),
        open_quartz_sdk::FramebufferFormat::R8 => (1, 1, false),
        open_quartz_sdk::FramebufferFormat::R32f => (4, 1, true),
        open_quartz_sdk::FramebufferFormat::Nv12 => {
            if width % 2 != 0 || height % 2 != 0 {
                return Err("NV12 framebuffers require even width and height".to_owned());
            }
            let expected = pixels * 3 / 2;
            if bytes.len() != expected {
                return Err(format!(
                    "NV12 data has {} bytes; expected {expected}",
                    bytes.len()
                ));
            }
            let mut rgba = vec![0; output_len];
            for y in 0..height as usize {
                for x in 0..width as usize {
                    let source = y * width as usize + x;
                    let uv = pixels + (y / 2) * width as usize + (x & !1);
                    let luminance = f32::from(bytes[source]);
                    let u = f32::from(bytes[uv]) - 128.0;
                    let v = f32::from(bytes[uv + 1]) - 128.0;
                    let target = source * 4;
                    rgba[target] = clamp_channel(luminance + 1.402 * v);
                    rgba[target + 1] = clamp_channel(luminance - 0.344 * u - 0.714 * v);
                    rgba[target + 2] = clamp_channel(luminance + 1.772 * u);
                    rgba[target + 3] = 255;
                }
            }
            return Ok(rgba);
        }
    };
    let row_bytes = (width as usize)
        .checked_mul(bytes_per_pixel)
        .ok_or_else(|| "Framebuffer row size overflows addressable memory".to_owned())?;
    let row_stride = stride.map(|value| value as usize).unwrap_or(row_bytes);
    let required = row_stride
        .checked_mul(height as usize - 1)
        .and_then(|prefix| prefix.checked_add(row_bytes))
        .ok_or_else(|| "Framebuffer stride overflows addressable memory".to_owned())?;
    if bytes.len() < required {
        return Err(format!(
            "Framebuffer data has {} bytes; needs at least {required} for {width}×{height}",
            bytes.len()
        ));
    }
    let mut rgba = vec![0; output_len];
    for y in 0..height as usize {
        for x in 0..width as usize {
            let source = y * row_stride + x * bytes_per_pixel;
            let target = (y * width as usize + x) * 4;
            let mut pixel = [0_u8; 4];
            for channel in 0..channels {
                let value = if is_float {
                    f32::from_le_bytes(
                        bytes[source + channel * 4..source + channel * 4 + 4]
                            .try_into()
                            .unwrap(),
                    ) * 255.0
                } else {
                    f32::from(bytes[source + channel])
                };
                pixel[channel] = clamp_channel(value);
            }
            match channels {
                1 => {
                    pixel[1] = pixel[0];
                    pixel[2] = pixel[0];
                    pixel[3] = 255;
                }
                2 => {
                    pixel[2] = 0;
                    pixel[3] = 255;
                }
                3 => pixel[3] = 255,
                _ => {}
            }
            rgba[target..target + 4].copy_from_slice(&pixel);
        }
    }
    Ok(rgba)
}

fn clamp_channel(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

fn resource_key<T: std::hash::Hash>(value: &T) -> u64 {
    use std::hash::Hasher;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}
#[cfg(test)]
mod tests {
    use super::{decode_framebuffer_data_url, decode_image_data_url, read_project, write_project};
    use base64::Engine as _;
    use open_quartz_sdk::{
        DataType, Environment, FramebufferFormat, GraphCommand, InputMode, NodeFactoryRequest,
        NodeId, OpenQuartz,
    };
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn png_data_url() -> String {
        let image = image::RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 128]).unwrap();
        let mut encoded = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(encoded.into_inner())
        )
    }

    #[test]
    fn image_data_url_decodes_exact_rgba_and_rejects_mime_spoofing() {
        let source = png_data_url();
        let image = decode_image_data_url(&source, "tiny.png".into()).unwrap();
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(image.file_name, "tiny.png");
        assert_eq!(image.rgba.as_slice(), &[255, 0, 0, 255, 0, 255, 0, 128]);
        assert_eq!(
            decode_image_data_url(&image.data_url, "again.png".into())
                .unwrap()
                .rgba,
            image.rgba
        );
        let spoofed = source.replacen("image/png", "image/jpeg", 1);
        assert!(matches!(
            decode_image_data_url(&spoofed, "bad.jpg".into()),
            Err(error) if error.contains("does not match")
        ));
        assert!(decode_image_data_url("data:image/png;base64,AA==", "bad.png".into()).is_err());
    }

    #[test]
    fn imported_image_resource_survives_project_save_and_reload() {
        let sdk = OpenQuartz::new(Environment::headless());
        let mut project = sdk.create_project("Image resource");
        let (node, _) = project
            .create_graph_node(
                NodeFactoryRequest::Input {
                    position: None,
                    data_type: DataType::Sampler2d,
                    input_mode: Some(InputMode::Image),
                },
                project.graph_revision(),
            )
            .unwrap();
        let asset = decode_image_data_url(&png_data_url(), "tiny.png".into()).unwrap();
        let mut data = node.data;
        data.image_data_url = Some(asset.data_url.clone());
        data.image_width = Some(asset.width);
        data.image_height = Some(asset.height);
        data.extra.insert(
            "imageFileName".into(),
            serde_json::Value::String(asset.file_name),
        );
        project
            .apply_graph_command(
                GraphCommand::UpdateNodeData {
                    node_id: NodeId::new(&node.id),
                    data,
                },
                project.graph_revision(),
            )
            .unwrap();

        let path = test_path();
        let contents = serde_json::to_string_pretty(&project.to_file()).unwrap();
        write_project(&path, &contents).unwrap();
        let reopened = sdk
            .open_project_json(&read_project(&path).unwrap())
            .unwrap();
        std::fs::remove_file(path).unwrap();

        let restored = reopened
            .graph()
            .nodes()
            .iter()
            .find(|item| item.id == node.id)
            .unwrap();
        assert_eq!(restored.data.image_width, Some(2));
        assert_eq!(restored.data.image_height, Some(1));
        assert_eq!(restored.data.extra["imageFileName"], "tiny.png");
        let persisted_url = restored.data.image_data_url.as_deref().unwrap();
        assert_eq!(
            decode_image_data_url(persisted_url, "reopened.png".into())
                .unwrap()
                .rgba,
            asset.rgba
        );
    }

    #[test]
    fn imported_framebuffer_resource_survives_project_save_and_reload() {
        let sdk = OpenQuartz::new(Environment::headless());
        let mut project = sdk.create_project("Framebuffer resource");
        let (node, _) = project
            .create_graph_node(
                NodeFactoryRequest::Input {
                    position: None,
                    data_type: DataType::Sampler2d,
                    input_mode: Some(InputMode::Framebuffer),
                },
                project.graph_revision(),
            )
            .unwrap();
        let raw = [255_u8, 0, 0, 255, 7, 7, 7, 7, 0, 255, 0, 255];
        let data_url = format!(
            "data:application/octet-stream;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(raw),
        );
        let asset = decode_framebuffer_data_url(
            &data_url,
            "padded.raw".into(),
            FramebufferFormat::Rgba8,
            1,
            2,
            Some(8),
        )
        .unwrap();
        let mut data = node.data;
        data.raw_data_url = Some(asset.data_url.clone());
        data.fb_format = Some(asset.format);
        data.fb_width = Some(asset.width);
        data.fb_height = Some(asset.height);
        data.fb_stride = asset.stride;
        data.extra.insert(
            "rawFileName".into(),
            serde_json::Value::String(asset.file_name),
        );
        project
            .apply_graph_command(
                GraphCommand::UpdateNodeData {
                    node_id: NodeId::new(&node.id),
                    data,
                },
                project.graph_revision(),
            )
            .unwrap();

        let path = test_path();
        let contents = serde_json::to_string_pretty(&project.to_file()).unwrap();
        write_project(&path, &contents).unwrap();
        let reopened = sdk
            .open_project_json(&read_project(&path).unwrap())
            .unwrap();
        std::fs::remove_file(path).unwrap();

        let restored = reopened
            .graph()
            .nodes()
            .iter()
            .find(|item| item.id == node.id)
            .unwrap();
        assert_eq!(restored.data.fb_format, Some(FramebufferFormat::Rgba8));
        assert_eq!(
            (restored.data.fb_width, restored.data.fb_height),
            (Some(1), Some(2))
        );
        assert_eq!(restored.data.fb_stride, Some(8));
        assert_eq!(restored.data.extra["rawFileName"], "padded.raw");
        let reopened_asset = decode_framebuffer_data_url(
            restored.data.raw_data_url.as_deref().unwrap(),
            "reopened.raw".into(),
            restored.data.fb_format.unwrap(),
            restored.data.fb_width.unwrap(),
            restored.data.fb_height.unwrap(),
            restored.data.fb_stride,
        )
        .unwrap();
        assert_eq!(
            reopened_asset.rgba.as_slice(),
            &[255, 0, 0, 255, 0, 255, 0, 255]
        );
    }

    #[test]
    fn raw_rgba_framebuffer_respects_stride_and_nv12_conversion() {
        let raw = [255_u8, 0, 0, 255, 9, 9, 9, 9, 0, 255, 0, 255];
        let url = format!(
            "data:application/octet-stream;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(raw)
        );
        let decoded = decode_framebuffer_data_url(
            &url,
            "padded.raw".into(),
            FramebufferFormat::Rgba8,
            1,
            2,
            Some(8),
        )
        .unwrap();
        assert_eq!((decoded.width, decoded.height), (1, 2));
        assert_eq!(decoded.format, FramebufferFormat::Rgba8);
        assert_eq!(decoded.stride, Some(8));
        assert_eq!(decoded.rgba.as_slice(), &[255, 0, 0, 255, 0, 255, 0, 255]);
        let nv12 = [80_u8, 80, 80, 80, 128, 128];
        let url = format!(
            "data:application/octet-stream;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(nv12)
        );
        let decoded = decode_framebuffer_data_url(
            &url,
            "gray.nv12".into(),
            FramebufferFormat::Nv12,
            2,
            2,
            None,
        )
        .unwrap();
        assert_eq!(
            decoded.rgba.as_slice(),
            &[80, 80, 80, 255, 80, 80, 80, 255, 80, 80, 80, 255, 80, 80, 80, 255]
        );
        assert!(decode_framebuffer_data_url(
            &url,
            "bad.raw".into(),
            FramebufferFormat::Rgba8,
            2,
            2,
            None
        )
        .is_err());
    }

    static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

    fn test_path() -> PathBuf {
        let sequence = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "open-quartz-slint-file-service-{}-{sequence}.json",
            std::process::id()
        ))
    }

    #[test]
    fn writes_and_reads_project_contents() {
        let path = test_path();
        write_project(&path, "{\"version\":\"0.4.0\"}").unwrap();
        assert_eq!(read_project(&path).unwrap(), "{\"version\":\"0.4.0\"}");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn reports_missing_project_path() {
        let path = test_path();
        let _ = std::fs::remove_file(&path);
        let error = read_project(&path).unwrap_err();
        assert!(error.starts_with("Cannot read "));
    }
}
