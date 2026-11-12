
#[cfg(test)]
use std::sync::atomic::AtomicU8;
use std::sync::{Arc, OnceLock};
use std::sync::atomic::{AtomicUsize, Ordering};

use wgpu::{Device, Extent3d, Features, Queue, TextureUsages};

use super::readback::ReadbackStagingRing;
use super::target::{RenderTarget, TextureFormat};
use crate::resources::{FrameBudget, FrameCredit};

static NEXT_GPU_ALLOCATION: AtomicUsize = AtomicUsize::new(1);

#[derive(Clone)]
pub struct TextureHandle {
    pub texture: Arc<wgpu::Texture>,
    pub view: Arc<wgpu::TextureView>,
    pub sampler: Arc<wgpu::Sampler>,
    /// Keeps an external pooled texture reserved while this handle is alive.
    pub lease: Option<Arc<dyn std::any::Any + Send + Sync>>,
    pub width: u32,
    pub height: u32,
    pub format: TextureFormat,
}

pub struct GpuBackend {
    pub device: Arc<Device>,
    pub queue: Arc<Queue>,
    pub(crate) readback_ring: Arc<ReadbackStagingRing>,
    frame_budget: OnceLock<Arc<FrameBudget>>,
    #[cfg(test)]
    pub(crate) readback_fault: AtomicU8,
    #[cfg(windows)]
    pub(crate) p010_converter: Arc<OnceLock<(wgpu::BindGroupLayout, wgpu::RenderPipeline)>>,
    #[cfg(all(target_os = "macos", feature = "native-video"))]
    pub(crate) videotoolbox_importer: OnceLock<Result<super::videotoolbox::VideoToolboxImporter, String>>,
}

impl GpuBackend {
    pub fn from_device(device: Device, queue: Queue) -> Self {
        Self {
            device: Arc::new(device),
            queue: Arc::new(queue),
            readback_ring: Arc::new(ReadbackStagingRing::new()),
            frame_budget: OnceLock::new(),
            #[cfg(test)]
            readback_fault: AtomicU8::new(0),
            #[cfg(windows)]
            p010_converter: Arc::new(OnceLock::new()),
            #[cfg(all(target_os = "macos", feature = "native-video"))]
            videotoolbox_importer: OnceLock::new(),
        }
    }

    pub fn from_shared(device: Arc<Device>, queue: Arc<Queue>) -> Self {
        Self {
            device,
            queue,
            readback_ring: Arc::new(ReadbackStagingRing::new()),
            frame_budget: OnceLock::new(),
            #[cfg(test)]
            readback_fault: AtomicU8::new(0),
            #[cfg(windows)]
            p010_converter: Arc::new(OnceLock::new()),
            #[cfg(all(target_os = "macos", feature = "native-video"))]
            videotoolbox_importer: OnceLock::new(),
        }
    }

    /// All pipelines on this backend share one immutable physical coordinator.
    /// Separate wrappers built with from_shared require explicit shared injection.
    pub fn frame_budget(&self) -> Arc<FrameBudget> {
        Arc::clone(self.frame_budget.get_or_init(|| FrameBudget::new(64, 512 * 1024 * 1024).expect("valid default frame budget")))
    }

    /// Inject before creating a pipeline; rebinding could strand allocations in
    /// another coordinator retained by an existing session.
    pub fn set_frame_budget(&self, budget: Arc<FrameBudget>) -> Result<(), String> {
        let existing = self.frame_budget.get_or_init(|| budget.clone());
        if Arc::ptr_eq(existing, &budget) { Ok(()) }
        else { Err("GPU backend already has another shared frame budget".into()) }
    }

    /// Allocation generations cannot alias a recycled native pointer whose old
    /// external owner still conservatively holds a physical credit.
    pub(crate) fn acquire_texture_credit(&self, budget: &Arc<FrameBudget>, bytes: u64) -> Result<Option<FrameCredit>, String> {
        let id = NEXT_GPU_ALLOCATION.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| "GPU allocation identity exhausted")?;
        budget.try_acquire((1, id), 1, bytes)
    }

    /// Attach external ownership to the actual native resource, not just the
    /// executor handle. wgpu queue/bind-group retention then retains the owner.
    #[cfg(target_os = "macos")]
    pub(crate) fn leased_metal_texture(
        &self,
        texture: Arc<wgpu::Texture>,
        lease: Arc<dyn std::any::Any + Send + Sync>,
    ) -> Result<Arc<wgpu::Texture>, String> {
        use objc2::{rc::Retained, runtime::ProtocolObject};
        use objc2_metal::{MTLTexture, MTLTextureType};
        let raw: Retained<ProtocolObject<dyn MTLTexture>> = {
            let native = unsafe { texture.as_hal::<wgpu::hal::api::Metal>() }
                .ok_or("native texture lease requires Metal")?;
            unsafe { Retained::retain(native.raw_handle() as *const _ as *mut _) }
                .ok_or("live Metal texture could not be retained")?
        };
        let size = texture.size();
        let format = texture.format();
        let usage = texture.usage();
        // Same-device/queue alias: no image allocation, mapping or CPU copy.
        let native = unsafe {
            wgpu::hal::metal::Device::texture_from_raw(raw, format, MTLTextureType::Type2D, 1, 1, size.into(),
                Some(Box::new(move || drop((texture, lease)))))
        };
        let alias = unsafe {
            self.device.create_texture_from_hal::<wgpu::hal::api::Metal>(native, &wgpu::TextureDescriptor {
                label: Some("open-quartz-leased-texture"), size, mip_level_count: 1, sample_count: 1,
                dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[],
            }, wgpu::TextureUses::RESOURCE)
        };
        Ok(Arc::new(alias))
    }

    pub fn create_target(&self, width: u32, height: u32, format: TextureFormat) -> RenderTarget {
        RenderTarget::new(&self.device, width, height, format)
    }

    /// Materialize a graph target only after its physical credit is reserved.
    /// Stable-frame pools use `create_target` because they own their own charge.
    pub(crate) fn create_frame_target(
        &self,
        width: u32,
        height: u32,
        format: TextureFormat,
        credit: FrameCredit,
    ) -> Result<RenderTarget, String> {
        let mut target = self.create_target(width, height, format);
        let lease: Arc<dyn std::any::Any + Send + Sync> = Arc::new(credit);
        #[cfg(target_os = "macos")]
        {
            target.texture = self.leased_metal_texture(target.texture, lease.clone())?;
            target.view = Arc::new(target.texture.create_view(&Default::default()));
        }
        target.lease = Some(lease);
        Ok(target)
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
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        TextureHandle {
            texture: Arc::new(texture),
            view: Arc::new(view),
            sampler: Arc::new(sampler),
            lease: None,
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
