//! Bounded, immutable snapshots of live graph textures on their original queue.
use std::fmt;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, OnceLock,
};
use std::time::{Duration, Instant};

use super::{GpuBackend, GpuOutputHandle, RenderDevice, TextureFormat};
use crate::outputs::{FrameImageDescription, FrameSnapshotIdentity};
use crate::resources::{FrameBudget, FrameCredit};


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
    physical_credit: Option<Arc<FrameCredit>>,
}

struct ConsumerLease {
    release_requested: Arc<AtomicBool>,
    _physical_credit: Option<Arc<FrameCredit>>,
    waker: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl Drop for ConsumerLease {
    fn drop(&mut self) {
        // A native texture may be destroyed under wgpu's internal locks. Do not
        // re-enter wgpu here; pool maintenance establishes queue completion.
        self.release_requested.store(true, Ordering::Release);
        if let Some(waker) = &self.waker { waker(); }
    }
}

/// Readiness of the snapshot copy, not permission to reuse its texture.
#[derive(Clone)]
pub struct FrameReady {
    device: Arc<wgpu::Device>,
    submission: wgpu::SubmissionIndex,
    ready: Arc<FrameReadyState>,
}

struct FrameReadyState {
    ready: AtomicBool,
    completed_at: OnceLock<Instant>,
}

impl FrameReady {
    pub fn is_ready(&self) -> bool {
        self.ready.ready.load(Ordering::Acquire)
    }

    /// Host observation of the queue completion callback, not a hardware timestamp.
    /// None unless the producer enabled stage tracing before capture.
    pub fn completed_at(&self) -> Option<Instant> {
        self.ready.completed_at.get().copied()
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
    pub(crate) fn belongs_to(&self, backend: &GpuBackend) -> bool {
        Arc::ptr_eq(&self.ready.device, &backend.device)
    }

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
    frame_budget: Option<Arc<FrameBudget>>,
    waker: Option<Arc<dyn Fn() + Send + Sync>>,
    trace_enabled: bool,
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
            frame_budget: None,
            waker: None,
            trace_enabled: false,
        })
    }

    pub(crate) fn set_trace_enabled(&mut self, enabled: bool) { self.trace_enabled = enabled; }


    pub fn with_frame_budget(
        backend: Arc<GpuBackend>,
        budget: StableFrameBudget,
        frame_budget: Arc<FrameBudget>,
    ) -> Result<Self, FrameCaptureError> {
        let mut pool = Self::new(backend, budget)?;
        pool.frame_budget = Some(frame_budget);
        Ok(pool)
    }

    pub fn set_frame_budget(&mut self, frame_budget: Arc<FrameBudget>) -> Result<(), FrameCaptureError> {
        if let Some(existing) = &self.frame_budget {
            if Arc::ptr_eq(existing, &frame_budget) { return Ok(()); }
            return Err(FrameCaptureError::InvalidSource("stable pool already has another shared frame budget".into()));
        }
        if !self.slots.get_mut().expect("stable frame pool lock poisoned").is_empty() {
            return Err(FrameCaptureError::InvalidSource("cannot attach budget after stable allocation".into()));
        }
        self.frame_budget = Some(frame_budget);
        Ok(())
    }

    pub(crate) fn clear_allocations(&mut self) {
        self.slots.get_mut().expect("stable frame pool lock poisoned").clear();
    }

    fn allocate_slot(&self, width: u32, height: u32, format: TextureFormat, bytes: u64) -> Result<Slot, FrameCaptureError> {
        let physical_credit = if let Some(frame_budget) = &self.frame_budget {
            let (max_surfaces, max_bytes) = frame_budget.limits();
            if max_surfaces < 1 || bytes > max_bytes { return Err(FrameCaptureError::Capacity); }
            let credit = self.backend.acquire_texture_credit(frame_budget, bytes)
                .map_err(FrameCaptureError::Device)?
                .ok_or(FrameCaptureError::WouldBlock)?;
            Some(Arc::new(credit))
        } else { None };
        let target = self.backend.create_target(width, height, format);
        Ok(Slot {
            texture: target.texture,
            view: target.view,
            bytes,
            busy: Arc::new(AtomicBool::new(false)),
            release_requested: Arc::new(AtomicBool::new(false)),
            physical_credit,
        })
    }

    /// Reserve physical output allocations before decoder look-ahead can consume
    /// their working-set capacity. No image copy or GPU submission is performed.
    pub fn prepare_slots(&self, width: u32, height: u32, format: TextureFormat, count: usize) -> Result<(), FrameCaptureError> {
        let dimension_limit = self.backend.device.limits().max_texture_dimension_2d;
        if width == 0 || height == 0 || width > dimension_limit || height > dimension_limit {
            return Err(FrameCaptureError::InvalidSource("invalid stable frame dimensions".into()));
        }
        let bytes_per_pixel = match format { TextureFormat::Rgba8Unorm => 4, TextureFormat::Rgba32Float => 16 };
        let bytes = u64::from(width).checked_mul(u64::from(height)).and_then(|pixels| pixels.checked_mul(bytes_per_pixel))
            .ok_or(FrameCaptureError::Capacity)?;
        let total = bytes.checked_mul(count as u64).ok_or(FrameCaptureError::Capacity)?;
        if count > self.budget.max_frames || total > self.budget.max_bytes { return Err(FrameCaptureError::Capacity); }
        if let Some(budget) = &self.frame_budget {
            let (max_surfaces, max_bytes) = budget.limits();
            if count > max_surfaces || total > max_bytes { return Err(FrameCaptureError::Capacity); }
        }
        self.poll_releases()?;
        let mut slots = self.slots.lock().expect("stable frame pool lock poisoned");
        let compatible = |slot: &Slot| slot.texture.width() == width && slot.texture.height() == height && slot.texture.format() == format.into();
        slots.retain(|slot| slot.busy.load(Ordering::Acquire) || compatible(slot));
        let mut present = slots.iter().filter(|slot| compatible(slot)).count();
        while present < count {
            let allocated: u64 = slots.iter().map(|slot| slot.bytes).sum();
            if slots.len() >= self.budget.max_frames || bytes > self.budget.max_bytes - allocated {
                return Err(FrameCaptureError::WouldBlock);
            }
            slots.push(self.allocate_slot(width, height, format, bytes)?);
            present += 1;
        }
        Ok(())
    }

    pub fn set_waker(&mut self, waker: Arc<dyn Fn() + Send + Sync>) {
        self.waker = Some(waker);
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
                    let waker = self.waker.clone();
                    self.backend.queue.on_submitted_work_done(move || {
                        busy.store(false, Ordering::Release);
                        if let Some(waker) = waker { waker(); }
                    });
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
        let (texture, view, release_requested, physical_credit) = {
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
                slots.push(self.allocate_slot(source.width, source.height, source.format, bytes)?);
                slots.len() - 1
            };
            let slot = &slots[index];
            slot.busy.store(true, Ordering::Release);
            (
                slot.texture.clone(),
                slot.view.clone(),
                slot.release_requested.clone(),
                slot.physical_credit.clone(),
            )
        };
        let lease = Arc::new(ConsumerLease {
            release_requested,
            _physical_credit: physical_credit,
            waker: self.waker.clone(),
        });
        let ready = Arc::new(FrameReadyState { ready: AtomicBool::new(false), completed_at: OnceLock::new() });
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
        let waker = self.waker.clone();
        let trace_enabled = self.trace_enabled;
        self.backend.queue.on_submitted_work_done(move || {
            if trace_enabled { let _ = completed.completed_at.set(Instant::now()); }
            completed.ready.store(true, Ordering::Release);
            // Preserve the actual external source owner until the copy has completed.
            drop(source);
            drop(producer_lease);
            if let Some(waker) = waker { waker(); }
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
    let alias = backend.leased_metal_texture(texture, lease)
        .map_err(|_| FrameCaptureError::WrongDevice)?;
    let view = Arc::new(alias.create_view(&Default::default()));
    Ok((alias, view))
}

#[cfg(test)]
#[path = "stable_frame_tests.rs"]
mod tests;
