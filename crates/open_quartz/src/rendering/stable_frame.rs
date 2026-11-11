//! Bounded, immutable snapshots of live graph textures on their original queue.
use std::fmt;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use super::{GpuBackend, GpuOutputHandle, RenderDevice, TextureFormat};
use crate::outputs::{FrameImageDescription, FrameSnapshotIdentity};

#[derive(Clone, Copy, Debug)]
pub struct StableFrameBudget {
    pub max_frames: usize,
    pub max_bytes: u64,
}

impl Default for StableFrameBudget {
    fn default() -> Self {
        Self {
            max_frames: 3,
            max_bytes: 256 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameCaptureError {
    WouldBlock,
    WrongDevice,
    InvalidSource(String),
    Capacity,
    Device(String),
}

impl fmt::Display for FrameCaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WouldBlock => f.write_str(
                "stable frame pool is full; release a consumer after its queue submission",
            ),
            Self::WrongDevice => f.write_str("stable frame source belongs to another device"),
            Self::InvalidSource(message) => write!(f, "invalid stable frame source: {message}"),
            Self::Capacity => f.write_str("stable frame exceeds the pool byte budget"),
            Self::Device(message) => write!(f, "stable frame device failure: {message}"),
        }
    }
}
impl std::error::Error for FrameCaptureError {}

struct Slot {
    texture: Arc<wgpu::Texture>,
    view: Arc<wgpu::TextureView>,
    bytes: u64,
    busy: Arc<AtomicBool>,
    release_requested: Arc<AtomicBool>,
}

struct ConsumerLease {
    release_requested: Arc<AtomicBool>,
}

impl Drop for ConsumerLease {
    fn drop(&mut self) {
        // A native texture may be destroyed under wgpu's internal locks. Do not
        // re-enter wgpu here; pool maintenance establishes queue completion.
        self.release_requested.store(true, Ordering::Release);
    }
}

/// Readiness of the snapshot copy, not permission to reuse its texture.
#[derive(Clone)]
pub struct FrameReady {
    device: Arc<wgpu::Device>,
    submission: wgpu::SubmissionIndex,
    ready: Arc<AtomicBool>,
}

impl FrameReady {
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    /// Explicit blocking boundary for offline consumers/tests; never call on the UI thread.
    /// A timeout does not release the frame or its producer/consumer leases.
    pub fn wait(&self, timeout: Duration) -> Result<(), FrameCaptureError> {
        if !self.is_ready() {
            self.device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(self.submission.clone()),
                    timeout: Some(timeout),
                })
                .map_err(|error| FrameCaptureError::Device(error.to_string()))?;
        }
        Ok(())
    }
}

/// A GPU snapshot whose pixels remain unchanged while any output lease is retained.
/// Consumers must use the producer's exact device/queue and retain the handle through
/// submission of their last GPU use. External consumers must first establish their own
/// completion before dropping it. Metal texture clones also retain a native drop
/// guard for Slint; other backends must explicitly retain GpuOutputHandle::lease.
#[derive(Clone)]
pub struct StableFrame {
    output: GpuOutputHandle,
    pub image: FrameImageDescription,
    pub identity: FrameSnapshotIdentity,
    pub ready: FrameReady,
}

impl StableFrame {
    pub fn output(&self) -> &GpuOutputHandle {
        &self.output
    }

    pub fn into_output(self) -> GpuOutputHandle {
        self.output
    }
}

/// Pool lifetime is independent of consumer lifetime. No codec or clock lives here.
pub struct StableFramePool {
    backend: Arc<GpuBackend>,
    budget: StableFrameBudget,
    slots: Mutex<Vec<Slot>>,
}

impl StableFramePool {
    pub fn new(
        backend: Arc<GpuBackend>,
        budget: StableFrameBudget,
    ) -> Result<Self, FrameCaptureError> {
        if budget.max_frames == 0 || budget.max_bytes == 0 {
            return Err(FrameCaptureError::InvalidSource(
                "pool limits must be non-zero".into(),
            ));
        }
        Ok(Self {
            backend,
            budget,
            slots: Mutex::new(Vec::new()),
        })
    }

    pub fn budget(&self) -> StableFrameBudget {
        self.budget
    }

    fn poll_releases(&self) -> Result<(), FrameCaptureError> {
        self.backend
            .device
            .poll(wgpu::PollType::Poll)
            .map_err(|error| FrameCaptureError::Device(error.to_string()))?;
        {
            let slots = self.slots.lock().expect("stable frame pool lock poisoned");
            for slot in slots.iter() {
                if slot.release_requested.swap(false, Ordering::AcqRel) {
                    let busy = slot.busy.clone();
                    self.backend
                        .queue
                        .on_submitted_work_done(move || busy.store(false, Ordering::Release));
                }
            }
        }
        self.backend
            .device
            .poll(wgpu::PollType::Poll)
            .map_err(|error| FrameCaptureError::Device(error.to_string()))?;
        Ok(())
    }

    /// Capture a RenderDevice output without consuming or modifying the borrowed source.
    /// WouldBlock accepts no input and encodes/submits no copy. Other device outputs are
    /// rejected before looking up a texture or consuming pool capacity.
    pub fn try_capture(
        &self,
        source: &RenderDevice,
        node_id: &str,
        identity: FrameSnapshotIdentity,
    ) -> Result<StableFrame, FrameCaptureError> {
        if !Arc::ptr_eq(&self.backend, &source.backend()) {
            return Err(FrameCaptureError::WrongDevice);
        }
        let output = source.output_handle(node_id).ok_or_else(|| {
            FrameCaptureError::InvalidSource(format!("output {node_id} has no texture"))
        })?;
        if identity.output_node_id != node_id {
            return Err(FrameCaptureError::InvalidSource(
                "snapshot identity names another output".into(),
            ));
        }
        self.capture(output, identity)
    }

    fn capture(
        &self,
        source: GpuOutputHandle,
        identity: FrameSnapshotIdentity,
    ) -> Result<StableFrame, FrameCaptureError> {
        let size = source.texture.size();
        let format: wgpu::TextureFormat = source.format.into();
        if source.width == 0
            || source.height == 0
            || size.width != source.width
            || size.height != source.height
            || size.depth_or_array_layers != 1
            || source.texture.dimension() != wgpu::TextureDimension::D2
            || source.texture.sample_count() != 1
            || source.texture.format() != format
            || !source
                .texture
                .usage()
                .contains(wgpu::TextureUsages::COPY_SRC)
        {
            return Err(FrameCaptureError::InvalidSource(
                "dimensions, format or COPY_SRC usage disagree with the texture".into(),
            ));
        }
        let bytes_per_pixel = match source.format {
            TextureFormat::Rgba8Unorm => 4,
            TextureFormat::Rgba32Float => 16,
        };
        let bytes = u64::from(source.width)
            .checked_mul(u64::from(source.height))
            .and_then(|pixels| pixels.checked_mul(bytes_per_pixel))
            .ok_or(FrameCaptureError::Capacity)?;
        if bytes > self.budget.max_bytes {
            return Err(FrameCaptureError::Capacity);
        }
        self.poll_releases()?;
        let (texture, view, release_requested) = {
            let mut slots = self.slots.lock().expect("stable frame pool lock poisoned");
            let reusable = slots.iter().position(|slot| {
                !slot.busy.load(Ordering::Acquire)
                    && slot.texture.size() == size
                    && slot.texture.format() == format
            });
            let index = if let Some(index) = reusable {
                index
            } else {
                // Reclaim idle incompatible sizes before charging a new allocation.
                slots.retain(|slot| slot.busy.load(Ordering::Acquire));
                let allocated: u64 = slots.iter().map(|slot| slot.bytes).sum();
                if slots.len() >= self.budget.max_frames
                    || bytes > self.budget.max_bytes - allocated
                {
                    return Err(FrameCaptureError::WouldBlock);
                }
                let target = self
                    .backend
                    .create_target(source.width, source.height, source.format);
                slots.push(Slot {
                    texture: target.texture,
                    view: target.view,
                    bytes,
                    busy: Arc::new(AtomicBool::new(false)),
                    release_requested: Arc::new(AtomicBool::new(false)),
                });
                slots.len() - 1
            };
            let slot = &slots[index];
            slot.busy.store(true, Ordering::Release);
            (
                slot.texture.clone(),
                slot.view.clone(),
                slot.release_requested.clone(),
            )
        };
        let lease = Arc::new(ConsumerLease { release_requested });
        let ready = Arc::new(AtomicBool::new(false));
        let mut encoder =
            self.backend
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("open-quartz-stable-frame-copy"),
                });
        encoder.copy_texture_to_texture(
            source.texture.as_image_copy(),
            texture.as_image_copy(),
            size,
        );
        let submission = self.backend.queue.submit([encoder.finish()]);
        let producer_lease = lease.clone();
        let completed = ready.clone();
        let sampler = source.sampler.clone();
        let source_format = source.format;
        self.backend.queue.on_submitted_work_done(move || {
            completed.store(true, Ordering::Release);
            // Preserve the actual external source owner until the copy has completed.
            drop(source);
            drop(producer_lease);
        });
        #[cfg(target_os = "macos")]
        let (texture, view) = {
            drop(view);
            leased_metal_texture(&self.backend, texture, lease.clone())?
        };
        let output = GpuOutputHandle {
            texture,
            view,
            sampler,
            lease: Some(lease),
            width: size.width,
            height: size.height,
            format: source_format,
        };
        Ok(StableFrame {
            image: FrameImageDescription {
                width: output.width,
                height: output.height,
                color_space: identity.color_space,
                alpha_mode: identity.alpha_mode,
            },
            output,
            identity,
            ready: FrameReady {
                device: self.backend.device.clone(),
                submission,
                ready,
            },
        })
    }
}

/// Slint Image clones only the wgpu texture, not GpuOutputHandle::lease. A native
/// alias attaches that lease to the resource Slint really retains, including
/// offscreen snapshots where a window's rendering notifier may not be invoked.
#[cfg(target_os = "macos")]
fn leased_metal_texture(
    backend: &GpuBackend,
    texture: Arc<wgpu::Texture>,
    lease: Arc<ConsumerLease>,
) -> Result<(Arc<wgpu::Texture>, Arc<wgpu::TextureView>), FrameCaptureError> {
    use objc2::{rc::Retained, runtime::ProtocolObject};
    use objc2_metal::{MTLTexture, MTLTextureType};
    let raw: Retained<ProtocolObject<dyn MTLTexture>> = {
        // SAFETY: a read-only HAL guard is released before importing its retained
        // Metal resource. The owner and queue are the pool's exact shared context.
        let native = unsafe { texture.as_hal::<wgpu::hal::api::Metal>() }
            .ok_or(FrameCaptureError::WrongDevice)?;
        unsafe { Retained::retain(native.raw_handle() as *const _ as *mut _) }
            .expect("live Metal texture must be retainable")
    };
    let size = texture.size();
    let format = texture.format();
    // SAFETY: the resource remains owned by the pool and the closure throughout
    // this alias's lifetime. All access stays on the same queue. No CPU pixels or
    // extra Metal texture allocation is involved; only a wgpu ownership wrapper.
    let native = unsafe {
        wgpu::hal::metal::Device::texture_from_raw(
            raw,
            format,
            MTLTextureType::Type2D,
            1,
            1,
            size.into(),
            Some(Box::new(move || drop((texture, lease)))),
        )
    };
    let alias = unsafe {
        backend
            .device
            .create_texture_from_hal::<wgpu::hal::api::Metal>(
                native,
                &wgpu::TextureDescriptor {
                    label: Some("open-quartz-leased-stable-frame"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC
                        | wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                },
                wgpu::TextureUses::RESOURCE,
            )
    };
    let view = Arc::new(alias.create_view(&Default::default()));
    Ok((Arc::new(alias), view))
}

#[cfg(test)]
#[path = "stable_frame_tests.rs"]
mod tests;
