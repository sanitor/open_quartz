//! Resident FP32 image inference on the caller's wgpu device and queue.
//!
//! The supported ONNX graph is compiled and validated before GPU allocation.
//! Initialization uploads immutable weights; every image conversion, convolution,
//! activation and channel-to-space operation thereafter stays on the GPU. Scratch
//! buffers have STORAGE usage only. There is deliberately no readback or CPU path.
//!
//! Keep each [`GpuOutput`] lease until all consumers have submitted their commands
//! to its owner's queue. Dropping it then permits reuse: same-queue ordering protects
//! both in-flight inference and consumption without a CPU/GPU wait. Cloning a raw
//! texture handle is not a substitute for keeping the output lease.

use crate::gpu_model::{ConvLayer, GpuModel, ModelPlan};
use std::{
    fmt,
    future::Future,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
    time::{Duration, Instant},
};

const SIDE: u32 = 224;
const PIXELS: u64 = SIDE as u64 * SIDE as u64;
const MAX_CHANNELS: u64 = 64;
const SCRATCH_BYTES: u64 = PIXELS * MAX_CHANNELS * 4;
const PLANES_BYTES: u64 = PIXELS * 3 * 4;
const INPUT_CACHE_CAPACITY: usize = 8;
// Triple buffering bounds resident output memory even if consumers retain leases.
const MAX_OUTPUT_SLOTS: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuError {
    UnsupportedModel(String),
    UnsupportedBackend(String),
    InvalidImage(String),
    WrongDevice,
    WrongSession,
    DeviceLimit(String),
    /// All three resident output slots are leased. Release a lease only after
    /// its consumers submit on the owner's queue, then retry; no output is reused
    /// or dropped implicitly when this backpressure error is returned.
    Capacity,
    Validation(String),
}
impl fmt::Display for GpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedModel(message) => write!(f, "unsupported resident model: {message}"),
            Self::UnsupportedBackend(message) => write!(f, "unsupported resident backend: {message}"),
            Self::InvalidImage(message) => write!(f, "invalid resident image: {message}"),
            Self::WrongDevice => f.write_str("resident image belongs to another wgpu device"),
            Self::WrongSession => f.write_str("resident input binding belongs to another inference session"),
            Self::DeviceLimit(message) => write!(f, "resident inference device limit: {message}"),
            Self::Capacity => f.write_str("resident output capacity exhausted: release a consumer lease after its queue submission"),
            Self::Validation(message) => write!(f, "resident inference wgpu validation: {message}"),
        }
    }
}
impl std::error::Error for GpuError {}

/// Actual native device/queue ownership, not a guessed numeric device identifier.
#[derive(Clone, Debug)]
pub struct GpuDeviceOwner {
    pub(crate) device: Arc<wgpu::Device>,
    pub(crate) queue: Arc<wgpu::Queue>,
}
impl GpuDeviceOwner {
    pub fn device(&self) -> &Arc<wgpu::Device> { &self.device }
    pub fn queue(&self) -> &Arc<wgpu::Queue> { &self.queue }
    pub fn same_device(&self, other: &Self) -> bool {
        self.device.as_ref() == other.device.as_ref()
    }
    pub fn matches_device(&self, device: &wgpu::Device) -> bool {
        self.device.as_ref() == device
    }
}

/// Reusable validated native texture binding. The pixels are never copied to RAM.
#[derive(Clone, Debug)]
pub struct GpuInput {
    owner: GpuDeviceOwner,
    session: Arc<()>,
    texture: wgpu::Texture,
    width: u32,
    height: u32,
    binding: wgpu::BindGroup,
}
impl GpuInput {
    pub fn texture(&self) -> &wgpu::Texture { &self.texture }
    pub fn width(&self) -> u32 { self.width }
    pub fn height(&self) -> u32 { self.height }
    pub fn device_owner(&self) -> &GpuDeviceOwner { &self.owner }
}

#[derive(Debug)]
struct OutputSlot {
    texture: Arc<wgpu::Texture>,
    view: Arc<wgpu::TextureView>,
    binding: wgpu::BindGroup,
    width: u32,
    height: u32,
}

/// A lease on a resident RGBA8 output. Retain it through consumer submission.
#[derive(Clone, Debug)]
pub struct GpuOutput {
    slot: Arc<OutputSlot>,
    owner: GpuDeviceOwner,
    submission: wgpu::SubmissionIndex,
}
impl GpuOutput {
    pub fn texture(&self) -> &wgpu::Texture { &self.slot.texture }
    pub fn view(&self) -> &wgpu::TextureView { &self.slot.view }
    pub fn texture_arc(&self) -> Arc<wgpu::Texture> { Arc::clone(&self.slot.texture) }
    pub fn view_arc(&self) -> Arc<wgpu::TextureView> { Arc::clone(&self.slot.view) }
    /// Retain the existing pool slot through downstream native render handles.
    /// This clones/coerces its Arc without allocating a wrapper around the output.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn lease(&self) -> Arc<dyn std::any::Any + Send + Sync> {
        self.slot.clone()
    }
    pub fn width(&self) -> u32 { self.slot.width }
    pub fn height(&self) -> u32 { self.slot.height }
    pub fn device_owner(&self) -> &GpuDeviceOwner { &self.owner }
    pub fn submission_index(&self) -> &wgpu::SubmissionIndex { &self.submission }
}

/// Host timings measure encoding/submission, never GPU completion. GPU timing is
/// absent because production inference neither maps buffers nor reads timestamps.
#[derive(Clone, Copy, Debug, Default)]
pub struct GpuCounters {
    pub submissions: u64,
    pub compute_dispatches: u64,
    pub host_image_bytes: u64,
    pub weight_upload_bytes: u64,
    pub scratch_bytes: u64,
    pub last_wall_time: Duration,
    pub total_wall_time: Duration,
    pub gpu_time: Option<Duration>,
}

struct ConvPass {
    pipeline: wgpu::ComputePipeline,
    binding: wgpu::BindGroup,
    output_channels: u32,
}

pub struct GpuImageSession {
    owner: GpuDeviceOwner,
    identity: Arc<()>,
    plan: ModelPlan,
    preprocess: wgpu::ComputePipeline,
    reconstruct: wgpu::ComputePipeline,
    planes: wgpu::Buffer,
    final_convolution: wgpu::Buffer,
    passes: Vec<ConvPass>,
    // Bind groups also retain resources; explicit handles document session lifetime.
    _resident_buffers: Vec<wgpu::Buffer>,
    input_cache: Vec<GpuInput>,
    output_pool: Vec<Arc<OutputSlot>>,
    counters: GpuCounters,
}

impl GpuImageSession {
    /// Prepare with the matching device/queue pair supplied by the renderer.
    /// Only a real Metal adapter is accepted; no software/alternate backend can
    /// be reported as the resident Metal execution provider.
    /// wgpu does not expose a queue's parent device; the pair must originate from
    /// the same adapter `request_device` call. Native input texture ownership is
    /// independently validated by `bind_input` before any inference submission.
    pub fn prepare(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        model: GpuModel,
    ) -> Result<Self, GpuError> {
        validate_model(&model)?;
        let adapter = device.adapter_info();
        #[cfg(target_os = "macos")]
        // SAFETY: inspect only backend presence; never access or destroy the HAL
        // resource. The temporary guard is immediately dropped, which is allowed.
        let native_metal = unsafe { device.as_hal::<wgpu::hal::api::Metal>() }.is_some();
        #[cfg(not(target_os = "macos"))]
        let native_metal = false;
        if !native_metal || adapter.backend != wgpu::Backend::Metal || adapter.device_type == wgpu::DeviceType::Cpu {
            return Err(GpuError::UnsupportedBackend(format!("requires native hardware Metal on macOS, got {:?} {:?}", adapter.backend, adapter.device_type)));
        }
        validate_limits(&device)?;

        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let preprocess = pipeline(&device, "resident BT601 preprocessing", include_str!("gpu/preprocess.wgsl"));
        let reconstruct = pipeline(&device, "resident pixelshuffle reconstruction", include_str!("gpu/reconstruct.wgsl"));
        let planes = scratch(&device, "resident Y Cb Cr", PLANES_BYTES);
        let first = scratch(&device, "resident convolution A", SCRATCH_BYTES);
        let second = scratch(&device, "resident convolution B", SCRATCH_BYTES);
        let mut buffers = Vec::with_capacity(9);
        let mut passes = Vec::with_capacity(4);
        let mut weight_upload_bytes = 0;
        for (index, layer) in model.layers.iter().enumerate() {
            // Geometry is validated and immutable. Specialization removes dynamic
            // kernel/channel indexing without changing FP32 weights or arithmetic.
            let source = format!(
                "const INPUT_CHANNELS: u32 = {}u;\nconst OUTPUT_CHANNELS: u32 = {}u;\nconst KERNEL: u32 = {}u;\nconst PADDING: u32 = {}u;\nconst RELU: bool = {};\n{}",
                layer.in_channels, layer.out_channels, layer.kernel[0], layer.pads[0],
                layer.relu, include_str!("gpu/convolution.wgsl"),
            );
            let convolution = pipeline(&device, "resident tiled FP32 convolution", &source);
            let layout = convolution.get_bind_group_layout(0);
            let weights = upload_f32(&device, "resident immutable weights", &layer.weights)?;
            let bias = upload_f32(&device, "resident immutable bias", &layer.bias)?;
            weight_upload_bytes += (layer.weights.len() as u64 + layer.bias.len() as u64) * 4;
            let input = if index == 0 { &planes } else if index % 2 == 1 { &first } else { &second };
            let output = if index % 2 == 0 { &first } else { &second };
            let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("resident convolution bindings"),
                layout: &layout,
                entries: &[
                    buffer_entry(0, input), buffer_entry(1, output),
                    buffer_entry(2, &weights), buffer_entry(3, &bias),
                ],
            });
            passes.push(ConvPass { pipeline: convolution, binding, output_channels: layer.out_channels });
            buffers.extend([weights, bias]);
        }
        finish_validation(scope)?;
        buffers.push(first);
        Ok(Self {
            owner: GpuDeviceOwner { device, queue },
            identity: Arc::new(()),
            plan: model.plan,
            preprocess,
            reconstruct,
            planes,
            final_convolution: second,
            passes,
            _resident_buffers: buffers,
            input_cache: Vec::with_capacity(INPUT_CACHE_CAPACITY),
            output_pool: Vec::new(),
            counters: GpuCounters {
                weight_upload_bytes,
                scratch_bytes: 2 * SCRATCH_BYTES + PLANES_BYTES,
                ..GpuCounters::default()
            },
        })
    }

    pub fn model_plan(&self) -> &ModelPlan { &self.plan }
    pub fn device_owner(&self) -> &GpuDeviceOwner { &self.owner }
    pub fn counters(&self) -> GpuCounters { self.counters }
    /// Explicit regression diagnostics only: pre-PixelShuffle NCHW [1,9,224,224].
    #[cfg(feature = "gpu-diagnostics")]
    pub fn diagnostic_raw_buffer(&self) -> &wgpu::Buffer { &self.final_convolution }
    #[cfg(feature = "gpu-diagnostics")]
    pub fn diagnostic_input_buffer(&self) -> &wgpu::Buffer { &self.planes }

    /// Register a native texture once, then reuse the resulting binding per frame.
    /// wgpu's native bind-group validation proves device identity before submission;
    /// the wrapper subsequently preserves that proof without trusting a raw ID.
    pub fn bind_input(&self, texture: &wgpu::Texture, width: u32, height: u32) -> Result<GpuInput, GpuError> {
        self.validate_image(texture, width, height)?;
        let device = self.owner.device();
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("resident validated source image"),
            layout: &self.preprocess.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                buffer_entry(1, &self.planes),
            ],
        });
        finish_validation(scope)?;
        Ok(GpuInput {
            owner: self.owner.clone(), session: Arc::clone(&self.identity),
            texture: texture.clone(), width, height, binding,
        })
    }

    /// Convenience path for renderers with stable or rotating native texture handles.
    /// The bounded binding cache avoids recreating views/groups for those textures.
    pub fn submit_texture(&mut self, texture: &wgpu::Texture, width: u32, height: u32) -> Result<GpuOutput, GpuError> {
        let input = if let Some(input) = self.input_cache.iter().find(|input| {
            input.texture == *texture && input.width == width && input.height == height
        }) {
            input.clone()
        } else {
            let input = self.bind_input(texture, width, height)?;
            if self.input_cache.len() == INPUT_CACHE_CAPACITY { self.input_cache.remove(0); }
            self.input_cache.push(input.clone());
            input
        };
        self.submit(&input)
    }

    /// Encode the entire network and submit on the owner's queue. Completion is
    /// synchronized by the returned submission index and same-queue ordering, not
    /// by claiming that host submission time is GPU inference time.
    pub fn submit(&mut self, input: &GpuInput) -> Result<GpuOutput, GpuError> {
        if !self.owner.same_device(&input.owner) { return Err(GpuError::WrongDevice); }
        if !Arc::ptr_eq(&self.identity, &input.session) { return Err(GpuError::WrongSession); }
        let started = Instant::now();
        let width = input.width.checked_mul(3).ok_or_else(|| GpuError::InvalidImage("output width overflow".into()))?;
        let height = input.height.checked_mul(3).ok_or_else(|| GpuError::InvalidImage("output height overflow".into()))?;
        let output = self.output_slot(width, height)?;
        let mut encoder = self.owner.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("resident super-resolution full network"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("resident BT601 RGB to planar"), timestamp_writes: None,
            });
            pass.set_pipeline(&self.preprocess);
            pass.set_bind_group(0, &input.binding, &[]);
            pass.dispatch_workgroups(SIDE / 8, SIDE / 8, 1);
        }
        for layer in &self.passes {
            // Separate passes make read/write transitions explicit between layers.
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("resident convolution and activation"), timestamp_writes: None,
            });
            pass.set_pipeline(&layer.pipeline);
            pass.set_bind_group(0, &layer.binding, &[]);
            // Shader tiles 32 adjacent pixels × 32 output channels, one row at a time.
            pass.dispatch_workgroups(SIDE / 32, SIDE, layer.output_channels.div_ceil(32));
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("resident pixelshuffle BT601 RGB reconstruction"), timestamp_writes: None,
            });
            pass.set_pipeline(&self.reconstruct);
            pass.set_bind_group(0, &output.binding, &[]);
            pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
        }
        let submission = self.owner.queue.submit([encoder.finish()]);
        self.counters.submissions = self.counters.submissions.saturating_add(1);
        self.counters.compute_dispatches = self.counters.compute_dispatches.saturating_add(6);
        self.counters.last_wall_time = started.elapsed();
        self.counters.total_wall_time = self.counters.total_wall_time.saturating_add(self.counters.last_wall_time);
        Ok(GpuOutput { slot: output, owner: self.owner.clone(), submission })
    }

    fn validate_image(&self, texture: &wgpu::Texture, width: u32, height: u32) -> Result<(), GpuError> {
        if width == 0 || height == 0 { return Err(GpuError::InvalidImage("empty dimensions".into())); }
        if texture.dimension() != wgpu::TextureDimension::D2 || texture.depth_or_array_layers() != 1
            || texture.sample_count() != 1 || texture.mip_level_count() != 1
            || texture.width() != width || texture.height() != height
        {
            return Err(GpuError::InvalidImage("requires matching single-layer, single-sample, single-mip D2 texture".into()));
        }
        if !matches!(texture.format(), wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm) {
            return Err(GpuError::InvalidImage("requires byte-domain RGBA8/BGRA8 unorm; sRGB and tensor inputs are unsupported".into()));
        }
        if !texture.usage().contains(wgpu::TextureUsages::TEXTURE_BINDING) {
            return Err(GpuError::InvalidImage("source requires TEXTURE_BINDING usage".into()));
        }
        let output_width = width.checked_mul(3).ok_or_else(|| GpuError::InvalidImage("output width overflow".into()))?;
        let output_height = height.checked_mul(3).ok_or_else(|| GpuError::InvalidImage("output height overflow".into()))?;
        let limits = self.owner.device.limits();
        for side in [output_width, output_height] {
            if side > limits.max_texture_dimension_2d || side > u32::MAX / 672 {
                return Err(GpuError::DeviceLimit("3x output exceeds texture dimension or shader indexing limit".into()));
            }
            if side.div_ceil(8) > limits.max_compute_workgroups_per_dimension {
                return Err(GpuError::DeviceLimit("output exceeds compute dispatch limit".into()));
            }
        }
        Ok(())
    }

    fn output_slot(&mut self, width: u32, height: u32) -> Result<Arc<OutputSlot>, GpuError> {
        // A slot becomes reusable only after the last consumer lease is released.
        if let Some(slot) = self.output_pool.iter().find(|slot| {
            slot.width == width && slot.height == height && Arc::strong_count(slot) == 1
        }) {
            return Ok(Arc::clone(slot));
        }
        // Keep current-size slots and outstanding leases; discard retired sizes.
        self.output_pool.retain(|slot| (slot.width == width && slot.height == height) || Arc::strong_count(slot) > 1);
        // Allocation is capped across sizes as well as within the active profile.
        if self.output_pool.len() >= MAX_OUTPUT_SLOTS {
            return Err(GpuError::Capacity);
        }
        let device = &self.owner.device;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let texture = Arc::new(device.create_texture(&wgpu::TextureDescriptor {
            label: Some("resident leased super-resolution RGBA8"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            // COPY_SRC exists solely for opt-in verification/capture consumers.
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        }));
        let view = Arc::new(texture.create_view(&wgpu::TextureViewDescriptor::default()));
        let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("resident reconstruction output bindings"),
            layout: &self.reconstruct.get_bind_group_layout(0),
            entries: &[
                buffer_entry(0, &self.final_convolution), buffer_entry(1, &self.planes),
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&view) },
            ],
        });
        finish_validation(scope)?;
        let slot = Arc::new(OutputSlot { texture, view, binding, width, height });
        self.output_pool.push(Arc::clone(&slot));
        Ok(slot)
    }
}

fn validate_model(model: &GpuModel) -> Result<(), GpuError> {
    let plan = &model.plan;
    if (plan.input_width, plan.input_height, plan.output_width, plan.output_height, plan.scale) != (224, 224, 672, 672, 3)
        || model.layers.len() != 4
    {
        return Err(GpuError::UnsupportedModel("requires validated 224x224 luminance, four convolutions, and pixelshuffle3".into()));
    }
    for (index, (layer, expected)) in model.layers.iter().zip([
        (1, 64, 5, true), (64, 64, 3, true), (64, 32, 3, true), (32, 9, 3, false),
    ]).enumerate() {
        validate_layer(layer, expected).map_err(|reason| GpuError::UnsupportedModel(format!("convolution {index}: {reason}")))?;
    }
    Ok(())
}

fn validate_layer(layer: &ConvLayer, expected: (u32, u32, u32, bool)) -> Result<(), &'static str> {
    let (input, output, kernel, relu) = expected;
    if layer.in_channels != input || layer.out_channels != output || layer.kernel != [kernel; 2]
        || layer.stride != [1; 2] || layer.pads != [kernel / 2; 4] || layer.relu != relu
    { return Err("unsupported convolution geometry or activation"); }
    let count = u64::from(input).checked_mul(u64::from(output))
        .and_then(|n| n.checked_mul(u64::from(kernel)))
        .and_then(|n| n.checked_mul(u64::from(kernel)))
        .ok_or("weight count overflow")?;
    if layer.weights.len() as u64 != count || layer.bias.len() as u64 != u64::from(output) {
        return Err("weight/bias count does not match OIHW geometry");
    }
    if !layer.weights.iter().chain(&layer.bias).all(|value| value.is_finite()) {
        return Err("nonfinite weights or bias");
    }
    Ok(())
}

fn validate_limits(device: &wgpu::Device) -> Result<(), GpuError> {
    let limits = device.limits();
    if limits.max_storage_buffer_binding_size as u64 >= SCRATCH_BYTES
        && limits.max_buffer_size >= SCRATCH_BYTES
        && limits.max_storage_buffers_per_shader_stage >= 4
        && limits.max_compute_invocations_per_workgroup >= 64
        && limits.max_compute_workgroup_size_x >= 8
        && limits.max_compute_workgroup_size_y >= 8
        && limits.max_compute_workgroups_per_dimension >= SIDE
        && limits.max_compute_workgroup_storage_size >= 8192
        && limits.max_texture_dimension_2d >= 672
    { Ok(()) } else {
        Err(GpuError::DeviceLimit("requires four storage bindings, 64-thread workgroups, 8KB workgroup storage, and 12.85MB storage bindings".into()))
    }
}

fn pipeline(device: &wgpu::Device, label: &str, source: &str) -> wgpu::ComputePipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label), source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(label), layout: None, module: &module, entry_point: Some("main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(), cache: None,
    })
}

fn scratch(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label), size,
        usage: if cfg!(feature = "gpu-diagnostics") { wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC } else { wgpu::BufferUsages::STORAGE },
        mapped_at_creation: false,
    })
}

fn upload_f32(device: &wgpu::Device, label: &str, values: &[f32]) -> Result<wgpu::Buffer, GpuError> {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label), size: values.len() as u64 * 4,
        usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: true,
    });
    {
        let mut bytes = buffer.slice(..).get_mapped_range_mut().map_err(|error| GpuError::Validation(error.to_string()))?;
        for (index, value) in values.iter().enumerate() { bytes.slice(index * 4..index * 4 + 4).copy_from_slice(&value.to_ne_bytes()); }
    }
    buffer.unmap();
    Ok(buffer)
}

fn buffer_entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource: buffer.as_entire_binding() }
}

struct ValidationWake(std::thread::Thread);
impl Wake for ValidationWake {
    fn wake(self: Arc<Self>) { self.0.unpark(); }
    fn wake_by_ref(self: &Arc<Self>) { self.0.unpark(); }
}

// Native wgpu validation futures resolve from CPU-side validation, not GPU image
// completion. No device wait, texture readback, mapped inference buffer, or unsafe
// executor is needed. Bindings and pipelines are checked only when first created.
fn finish_validation(scope: wgpu::ErrorScopeGuard) -> Result<(), GpuError> {
    let waker = Waker::from(Arc::new(ValidationWake(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(scope.pop());
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(None) => return Ok(()),
            Poll::Ready(Some(error)) => return Err(GpuError::Validation(error.to_string())),
            Poll::Pending => std::thread::park(),
        }
    }
}

// Real hardware acceptance is explicitly disabled outside native macOS Metal.
#[cfg(all(test, target_os = "macos"))]
#[path = "gpu_tests.rs"]
mod gpu_tests;
