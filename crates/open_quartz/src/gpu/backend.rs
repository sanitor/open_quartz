pub const FULLSCREEN_VERT_WITH_UV: &str = r#"
struct VertexOutput {
    @builtin(position) position: vec4f,
    @location(0) v_uv: vec2f,
}

@vertex
fn main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let x = f32(i32(vertex_index) / 2) * 4.0 - 1.0;
    let y = f32(i32(vertex_index) % 2) * 4.0 - 1.0;
    var output: VertexOutput;
    output.position = vec4f(x, y, 0.0, 1.0);
    output.v_uv = vec2f((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return output;
}
"#;

pub const BLIT_FRAG: &str = r#"
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
    return textureSample(tex, samp, v_uv);
}
"#;

use std::sync::{Arc, OnceLock};

use wgpu::{Device, Extent3d, Features, Queue, TextureUsages};

use super::readback::ReadbackStagingRing;
use super::target::{RenderTarget, TextureFormat};

#[derive(Clone)]
pub struct TextureHandle {
    pub texture: Arc<wgpu::Texture>,
    pub view: Arc<wgpu::TextureView>,
    pub sampler: Arc<wgpu::Sampler>,
    pub width: u32,
    pub height: u32,
    pub format: TextureFormat,
}

pub struct GpuBackend {
    pub device: Arc<Device>,
    pub queue: Arc<Queue>,
    pub(crate) readback_ring: Arc<ReadbackStagingRing>,
    #[cfg(windows)]
    pub(crate) p010_converter: Arc<OnceLock<(wgpu::BindGroupLayout, wgpu::RenderPipeline)>>,
}

impl GpuBackend {
    pub fn from_device(device: Device, queue: Queue) -> Self {
        Self {
            device: Arc::new(device),
            queue: Arc::new(queue),
            readback_ring: Arc::new(ReadbackStagingRing::new()),
            #[cfg(windows)]
            p010_converter: Arc::new(OnceLock::new()),
        }
    }

    pub fn from_shared(device: Arc<Device>, queue: Arc<Queue>) -> Self {
        Self {
            device,
            queue,
            readback_ring: Arc::new(ReadbackStagingRing::new()),
            #[cfg(windows)]
            p010_converter: Arc::new(OnceLock::new()),
        }
    }

    pub fn create_target(&self, width: u32, height: u32, format: TextureFormat) -> RenderTarget {
        RenderTarget::new(&self.device, width, height, format)
    }

    pub fn create_texture(&self, width: u32, height: u32, format: TextureFormat) -> TextureHandle {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("open-quartz-texture"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: format.into(),
            usage: TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC
                | TextureUsages::COPY_DST
                | TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("open-quartz-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        TextureHandle {
            texture: Arc::new(texture),
            view: Arc::new(view),
            sampler: Arc::new(sampler),
            width,
            height,
            format,
        }
    }

    pub fn upload_rgba(&self, texture: &TextureHandle, rgba: &[u8]) -> Result<(), String> {
        let expected = texture.width as usize * texture.height as usize * 4;
        if rgba.len() != expected {
            return Err(format!(
                "RGBA byte length {} does not match {}x{} texture",
                rgba.len(),
                texture.width,
                texture.height
            ));
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(texture.width * 4),
                rows_per_image: Some(texture.height),
            },
            Extent3d {
                width: texture.width,
                height: texture.height,
                depth_or_array_layers: 1,
            },
        );
        Ok(())
    }

    pub fn blit(&self, source: &TextureHandle, target: &RenderTarget) {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("open-quartz-blit"),
            });
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &source.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &target.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            Extent3d {
                width: source.width.min(target.width),
                height: source.height.min(target.height),
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
    }

    pub fn supported_features(&self) -> Features {
        self.device.features()
    }
}
