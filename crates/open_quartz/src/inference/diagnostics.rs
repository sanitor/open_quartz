//! Explicit, feature-gated regression snapshots. Normal playback never captures these.
use std::sync::Arc;
use super::{RequestStamp, ResidentInferenceSession, TensorElementType};
use super::tasks::ResidentImage;
use crate::rendering::{GpuOutputHandle, TextureFormat};

/// Same-queue copies from one production inference submission, published only
/// after that request is accepted by the graph. Mapping is the consumer's opt-in.
pub struct InferenceDiagnosticSnapshot {
    pub source: GpuOutputHandle,
    pub output: GpuOutputHandle,
    pub input: Arc<wgpu::Buffer>,
    /// GPU export storage is always FP32, including integer-valued UInt8 class IDs.
    pub raw: Arc<wgpu::Buffer>,
    pub raw_shape: Vec<i64>,
    /// Original selected ONNX output dtype, not the export storage dtype.
    pub raw_dtype: TensorElementType,
    pub detections: Option<Arc<wgpu::Buffer>>,
    pub depth_range: Option<Arc<wgpu::Buffer>>,
    pub logits: Option<Arc<wgpu::Buffer>>,
    pub logits_shape: Option<Vec<i64>>,
    pub stamp: RequestStamp,
    pub model_id: String,
    pub source_node_id: String,
}

pub(crate) fn capture(session: &ResidentInferenceSession, image: &ResidentImage,
    source: &wgpu::Texture, source_node_id: &str, model_id: &str, stamp: RequestStamp,
    raw_dtype: TensorElementType,
) -> Result<InferenceDiagnosticSnapshot, String> {
    let owner = session.device_owner();
    let device = owner.device();
    let queue = owner.queue();
    if source.format() != wgpu::TextureFormat::Rgba8Unorm
        || !source.usage().contains(wgpu::TextureUsages::COPY_SRC) {
        return Err("inference diagnostics require an explicit COPY_SRC RGBA8 source".into());
    }
    let texture = Arc::new(device.create_texture(&wgpu::TextureDescriptor {
        label: Some("explicit same-frame inference source snapshot"),
        size: source.size(), mip_level_count: 1, sample_count: 1,
        dimension: wgpu::TextureDimension::D2, format: source.format(),
        usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    }));
    let sampler = Arc::new(device.create_sampler(&wgpu::SamplerDescriptor::default()));
    let source_handle = GpuOutputHandle {
        view: Arc::new(texture.create_view(&Default::default())), texture,
        sampler: sampler.clone(), width: source.width(), height: source.height(),
        format: TextureFormat::Rgba8Unorm, lease: None,
    };
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("explicit inference diagnostics, outside production frame metrics"),
    });
    encoder.copy_texture_to_texture(source.as_image_copy(), source_handle.texture.as_image_copy(), source.size());
    let copy_buffer = |encoder: &mut wgpu::CommandEncoder, buffer: &wgpu::Buffer, size: u64| {
        let copy = Arc::new(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("explicit inference tensor snapshot"), size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }));
        encoder.copy_buffer_to_buffer(buffer, 0, &copy, 0, size);
        copy
    };
    let (input, raw, raw_shape, detections, depth_range) = match (session, image) {
        (ResidentInferenceSession::Sr3(session), ResidentImage::Gpu(_)) => (
            copy_buffer(&mut encoder, session.diagnostic_input_buffer(), 224 * 224 * 4),
            copy_buffer(&mut encoder, session.diagnostic_raw_buffer(), 9 * 224 * 224 * 4), vec![1, 9, 224, 224], None, None,
        ),
        (ResidentInferenceSession::Catalog(session), ResidentImage::Catalog(output)) => (
            { let buffer = session.input_buffer(); copy_buffer(&mut encoder, &buffer, buffer.size()) },
            { let buffer = output.raw_result_buffer(); copy_buffer(&mut encoder, &buffer, buffer.size()) },
            session.diagnostic_output_shape().iter().map(|&dimension| dimension as i64).collect(),
            output.detection_buffer().map(|buffer| copy_buffer(&mut encoder, &buffer, buffer.size())),
            if model_id == "midas-small" { output.depth_range_buffer().map(|buffer| copy_buffer(&mut encoder, &buffer, buffer.size())) } else { None },
        ),
        _ => return Err("inference diagnostic session/output ownership mismatch".into()),
    };
    let (logits, logits_shape) = match (session, image) {
        (ResidentInferenceSession::Catalog(session), ResidentImage::Catalog(output)) => (
            output.diagnostic_logits_buffer().map(|buffer| copy_buffer(&mut encoder, &buffer, buffer.size())),
            session.diagnostic_logits_shape().map(|shape| shape.iter().map(|&dimension| dimension as i64).collect()),
        ),
        _ => (None, None),
    };
    queue.submit([encoder.finish()]);
    Ok(InferenceDiagnosticSnapshot {
        source: source_handle, output: image.handle(sampler), input, raw, raw_shape,
        raw_dtype, detections, depth_range, logits, logits_shape, stamp,
        model_id: model_id.to_owned(), source_node_id: source_node_id.to_owned(),
    })
}
