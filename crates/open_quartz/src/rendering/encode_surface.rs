//! Same-queue SDR sRGB RGBA8 to hardware-encoder NV12 conversion.
//!
//! No CPU plane access is used. Each physical 420v allocation is charged once in
//! native-surface domain 0. A consumer lease prevents reuse until both the GPU
//! completion callback and the encoder's actual source-consumption owner release it.
use std::{
    ffi::c_void,
    ptr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
    time::Instant,
};

use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_metal::{MTLTexture, MTLTextureType, MTLTextureUsage};
use oq_media::EncoderSurface;

use super::{GpuBackend, StableFrame, TextureFormat};
use crate::{
    outputs::{FrameAlphaMode, FrameColorSpace},
    resources::FrameBudget,
};

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    static kCVMetalTextureUsage: *const c_void;
    fn CVMetalTextureCacheCreate(
        allocator: *const c_void,
        attributes: *const c_void,
        device: *const c_void,
        texture_attributes: *const c_void,
        cache: *mut *mut c_void,
    ) -> i32;
    fn CVMetalTextureCacheCreateTextureFromImage(
        allocator: *const c_void,
        cache: *mut c_void,
        image: *mut c_void,
        attributes: *const c_void,
        format: usize,
        width: usize,
        height: usize,
        plane: usize,
        texture: *mut *mut c_void,
    ) -> i32;
    fn CVMetalTextureGetTexture(texture: *mut c_void) -> *mut c_void;
    fn CVPixelBufferGetPixelFormatType(buffer: *mut c_void) -> u32;
    fn CVPixelBufferGetPlaneCount(buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetWidthOfPlane(buffer: *mut c_void, plane: usize) -> usize;
    fn CVPixelBufferGetHeightOfPlane(buffer: *mut c_void, plane: usize) -> usize;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;
    fn CFDictionaryCreate(
        allocator: *const c_void,
        keys: *const *const c_void,
        values: *const *const c_void,
        count: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> *mut c_void;
    fn CFNumberCreate(allocator: *const c_void, kind: isize, value: *const c_void) -> *mut c_void;
    fn CFRelease(value: *const c_void);
}

struct CfRef(*mut c_void);
// Immutable CVMetalTexture wrappers are shareable. The cache itself is accessed
// exclusively through &mut Nv12Converter; it never enters a queue callback.
unsafe impl Send for CfRef {}
unsafe impl Sync for CfRef {}
impl Drop for CfRef {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) }
    }
}

/// Explicit host declaration, not a guess made from untagged shader output.
/// RGB is sRGB encoded with BT.709 primaries; output is limited-range BT.709.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputInterpretation {
    /// Straight alpha; decode sRGB, composite over black in linear light.
    SdrSrgbRgba8Black,
    /// Encoded RGB is premultiplied; unpremultiply before decoding sRGB.
    SdrSrgbRgba8PremultipliedBlack,
    SdrSrgbRgba8OpaqueBlack,
}
impl InputInterpretation {
    fn alpha(self) -> FrameAlphaMode {
        match self {
            Self::SdrSrgbRgba8Black => FrameAlphaMode::Straight,
            Self::SdrSrgbRgba8PremultipliedBlack => FrameAlphaMode::Premultiplied,
            Self::SdrSrgbRgba8OpaqueBlack => FrameAlphaMode::Opaque,
        }
    }
    fn shader_alpha(self) -> u32 {
        match self {
            Self::SdrSrgbRgba8Black => 0,
            Self::SdrSrgbRgba8PremultipliedBlack => 1,
            Self::SdrSrgbRgba8OpaqueBlack => 2,
        }
    }
}

#[derive(Clone)]
pub struct ConversionReady(Arc<Completion>);
struct Completion {
    ready: AtomicBool,
    completed_at: OnceLock<Instant>,
}
impl ConversionReady {
    /// Observes actual queue completion; does not submit or wait for GPU work.
    pub fn is_ready(&self) -> bool {
        self.0.ready.load(Ordering::Acquire)
    }
    /// Host callback observation, present only when tracing was enabled.
    pub fn completed_at(&self) -> Option<Instant> {
        self.0.completed_at.get().copied()
    }
}
pub struct ConvertedFrame {
    pub surface: EncoderSurface,
    pub ready: ConversionReady,
}
pub enum ConversionAdmission {
    Accepted(ConvertedFrame),
    WouldBlock(StableFrame),
}

struct Planes {
    _textures: [wgpu::Texture; 2],
    views: [wgpu::TextureView; 2],
    _wrappers: [CfRef; 2],
}
struct Slot {
    surface: EncoderSurface,
    planes: Planes,
    busy: AtomicBool,
}
struct ConsumerLease {
    slot: Arc<Slot>,
    budget: Arc<FrameBudget>,
    waker: Option<Arc<dyn Fn() + Send + Sync>>,
}
impl Drop for ConsumerLease {
    fn drop(&mut self) {
        self.slot.busy.store(false, Ordering::Release);
        if let Some(waker) = &self.waker {
            waker();
        }
        self.budget.notify_waiters();
    }
}

pub struct Nv12Converter {
    backend: Arc<GpuBackend>,
    budget: Arc<FrameBudget>,
    cache: CfRef,
    writable_attributes: CfRef,
    slots: Vec<Arc<Slot>>,
    max_surfaces: usize,
    max_bytes: u64,
    interpretation: Option<InputInterpretation>,
    pipeline: Option<(
        InputInterpretation,
        wgpu::BindGroupLayout,
        [wgpu::RenderPipeline; 2],
    )>,
    waker: Option<Arc<dyn Fn() + Send + Sync>>,
    trace_enabled: bool,
}

impl Nv12Converter {
    pub fn new(
        backend: Arc<GpuBackend>,
        max_surfaces: usize,
        max_bytes: u64,
    ) -> Result<Self, String> {
        if max_surfaces == 0 || max_bytes == 0 {
            return Err("NV12 pool limits must be nonzero".into());
        }
        let usage: i64 = 5; // MTLTextureUsageShaderRead | MTLTextureUsageRenderTarget
        let usage = unsafe { CFNumberCreate(ptr::null(), 4, (&usage as *const i64).cast()) };
        if usage.is_null() {
            return Err("cannot allocate NV12 Metal usage attribute".into());
        }
        let usage = CfRef(usage);
        let attributes = unsafe {
            CFDictionaryCreate(
                ptr::null(),
                &kCVMetalTextureUsage,
                &(usage.0 as *const c_void),
                1,
                &kCFTypeDictionaryKeyCallBacks,
                &kCFTypeDictionaryValueCallBacks,
            )
        };
        if attributes.is_null() {
            return Err("cannot allocate NV12 Metal texture attributes".into());
        }
        let attributes = CfRef(attributes);
        let cache = {
            let device = unsafe { backend.device.as_hal::<wgpu::hal::api::Metal>() }
                .ok_or("NV12 conversion requires the hardware Metal backend")?;
            let mut cache = ptr::null_mut();
            let status = unsafe {
                CVMetalTextureCacheCreate(
                    ptr::null(),
                    ptr::null(),
                    Retained::as_ptr(device.raw_device()).cast(),
                    attributes.0,
                    &mut cache,
                )
            };
            if status != 0 || cache.is_null() {
                return Err(format!("NV12 CVMetalTextureCacheCreate failed: {status}"));
            }
            CfRef(cache)
        };
        Ok(Self {
            budget: backend.frame_budget(),
            backend,
            cache,
            writable_attributes: attributes,
            slots: Vec::new(),
            max_surfaces,
            max_bytes,
            interpretation: None,
            pipeline: None,
            waker: None,
            trace_enabled: false,
        })
    }

    pub fn set_input_interpretation(&mut self, interpretation: InputInterpretation) {
        self.interpretation = Some(interpretation);
    }
    /// Wakers must only send notifications: never re-enter GPU/codec APIs.
    pub fn set_waker(&mut self, waker: Arc<dyn Fn() + Send + Sync>) {
        self.waker = Some(waker);
    }
    pub fn set_trace_enabled(&mut self, enabled: bool) {
        self.trace_enabled = enabled;
    }

    fn validate_dimensions(&self, width: u32, height: u32) -> Result<(), String> {
        let limit = self.backend.device.limits().max_texture_dimension_2d;
        if width == 0
            || height == 0
            || width % 2 != 0
            || height % 2 != 0
            || width > limit
            || height > limit
        {
            return Err(
                "NV12 conversion requires positive even dimensions within device limits".into(),
            );
        }
        Ok(())
    }

    /// Reserve the full encoder working set before admitting a decoded source.
    /// False retains any partial reservation and means shared physical pressure;
    /// callers retry after a budget wake without submitting decode/render work.
    pub fn prepare_surfaces(
        &mut self,
        width: u32,
        height: u32,
        count: usize,
    ) -> Result<bool, String> {
        self.validate_dimensions(width, height)?;
        if count == 0 || count > self.max_surfaces {
            return Err("NV12 preparation count exceeds pool capacity".into());
        }
        self.slots.retain(|slot| {
            slot.surface.width() == width && slot.surface.height() == height
                || slot.busy.load(Ordering::Acquire)
        });
        let matching =
            |slot: &&Arc<Slot>| slot.surface.width() == width && slot.surface.height() == height;
        while self.slots.iter().filter(matching).count() < count {
            if self.slots.len() == self.max_surfaces {
                return Ok(false);
            }
            let surface = EncoderSurface::new(width, height).map_err(|error| error.to_string())?;
            let bytes = surface.allocation_size();
            if bytes == 0 {
                return Err("NV12 surface has no measurable physical allocation".into());
            }
            let working_bytes = bytes
                .checked_mul(count as u64)
                .ok_or("NV12 pool allocation size overflow")?;
            if working_bytes > self.max_bytes {
                return Err("NV12 working set exceeds converter byte budget".into());
            }
            self.budget.validate_working_set(count, working_bytes)?;
            let allocated: u64 = self
                .slots
                .iter()
                .map(|slot| slot.surface.allocation_size())
                .sum();
            if bytes > self.max_bytes.saturating_sub(allocated) {
                return Ok(false);
            }
            let Some(credit) =
                self.budget
                    .try_acquire((0, surface.pixel_buffer() as usize), 1, bytes)?
            else {
                return Ok(false);
            };
            let surface = surface.with_lease(Arc::new(credit));
            let planes = self.import_planes(&surface)?;
            self.slots.push(Arc::new(Slot {
                surface,
                planes,
                busy: AtomicBool::new(false),
            }));
        }
        Ok(true)
    }

    fn import_planes(&self, surface: &EncoderSurface) -> Result<Planes, String> {
        let buffer = surface.pixel_buffer();
        if unsafe {
            CVPixelBufferGetPixelFormatType(buffer) != u32::from_be_bytes(*b"420v")
                || CVPixelBufferGetPlaneCount(buffer) != 2
        } {
            return Err("encoder surface is not two-plane video-range 420v".into());
        }
        let mut textures: [Option<wgpu::Texture>; 2] = [None, None];
        let mut wrappers: [Option<CfRef>; 2] = [None, None];
        for plane in 0..2 {
            let divisor = if plane == 0 { 1 } else { 2 };
            let (width, height) = (surface.width() / divisor, surface.height() / divisor);
            if unsafe {
                CVPixelBufferGetWidthOfPlane(buffer, plane) != width as usize
                    || CVPixelBufferGetHeightOfPlane(buffer, plane) != height as usize
            } {
                return Err(format!("invalid encoder NV12 plane {plane} dimensions"));
            }
            let mut wrapper = ptr::null_mut();
            let metal_format = if plane == 0 { 10 } else { 30 }; // R8Unorm, RG8Unorm
            let status = unsafe {
                CVMetalTextureCacheCreateTextureFromImage(
                    ptr::null(),
                    self.cache.0,
                    buffer,
                    self.writable_attributes.0,
                    metal_format,
                    width as usize,
                    height as usize,
                    plane,
                    &mut wrapper,
                )
            };
            if status != 0 || wrapper.is_null() {
                return Err(format!(
                    "writable NV12 CVMetalTexture plane {plane} import failed: {status}"
                ));
            }
            let wrapper = CfRef(wrapper);
            let raw = unsafe {
                Retained::<ProtocolObject<dyn MTLTexture>>::retain(
                    CVMetalTextureGetTexture(wrapper.0).cast(),
                )
            }
            .ok_or("NV12 CVMetalTexture has no Metal texture")?;
            if !raw.usage().contains(MTLTextureUsage::RenderTarget)
                || raw.width() != width as usize
                || raw.height() != height as usize
            {
                return Err(format!(
                    "CoreVideo NV12 plane {plane} is not a correctly sized Metal render target"
                ));
            }
            let format = if plane == 0 {
                wgpu::TextureFormat::R8Unorm
            } else {
                wgpu::TextureFormat::Rg8Unorm
            };
            let size = wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            };
            let native = unsafe {
                wgpu::hal::metal::Device::texture_from_raw(
                    raw,
                    format,
                    MTLTextureType::Type2D,
                    1,
                    1,
                    size.into(),
                    None,
                )
            };
            let texture = unsafe {
                self.backend
                    .device
                    .create_texture_from_hal::<wgpu::hal::api::Metal>(
                        native,
                        &wgpu::TextureDescriptor {
                            label: Some("encoder-nv12-writable-plane"),
                            size,
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format,
                            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                            view_formats: &[],
                        },
                        wgpu::TextureUses::UNINITIALIZED,
                    )
            };
            textures[plane] = Some(texture);
            wrappers[plane] = Some(wrapper);
        }
        let textures = textures.map(|texture| texture.expect("both NV12 planes imported"));
        let views = textures
            .each_ref()
            .map(|texture| texture.create_view(&Default::default()));
        Ok(Planes {
            _textures: textures,
            views,
            _wrappers: wrappers.map(|wrapper| wrapper.expect("both NV12 wrappers imported")),
        })
    }

    /// WouldBlock returns the original frame and encodes/submits no GPU work.
    pub fn try_convert(&mut self, input: StableFrame) -> Result<ConversionAdmission, String> {
        let interpretation = self
            .interpretation
            .ok_or("NV12 input interpretation must be explicitly declared")?;
        let output = input.output();
        self.validate_dimensions(output.width, output.height)?;
        if !input.belongs_to(&self.backend) {
            return Err("NV12 input belongs to another GPU device".into());
        }
        if output.format != TextureFormat::Rgba8Unorm
            || output.texture.format() != wgpu::TextureFormat::Rgba8Unorm
            || !output
                .texture
                .usage()
                .contains(wgpu::TextureUsages::TEXTURE_BINDING)
        {
            return Err("NV12 input must be a shader-readable SDR RGBA8Unorm texture".into());
        }
        if input.image.color_space != FrameColorSpace::Unknown
            && input.image.color_space != FrameColorSpace::Srgb
        {
            return Err(
                "NV12 input color metadata disagrees with explicit sRGB interpretation".into(),
            );
        }
        if input.image.alpha_mode != FrameAlphaMode::Unknown
            && input.image.alpha_mode != interpretation.alpha()
        {
            return Err("NV12 input alpha metadata disagrees with explicit interpretation".into());
        }
        if input.image.width != output.width || input.image.height != output.height {
            return Err("NV12 frame dimensions disagree with image metadata".into());
        }
        let slot = self
            .slots
            .iter()
            .find(|slot| {
                slot.surface.width() == output.width
                    && slot.surface.height() == output.height
                    && !slot.busy.load(Ordering::Acquire)
            })
            .cloned();
        let Some(slot) = slot else {
            return Ok(ConversionAdmission::WouldBlock(input));
        };
        if self.pipeline.as_ref().map(|(kind, _, _)| *kind) != Some(interpretation) {
            self.pipeline = Some(create_pipelines(&self.backend.device, interpretation));
        }
        let (_, layout, pipelines) = self.pipeline.as_ref().unwrap();
        let bindings = self
            .backend
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("encoder-nv12-input"),
                layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&output.view),
                }],
            });
        let mut encoder =
            self.backend
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("encoder-nv12-convert"),
                });
        for plane in 0..2 {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("encoder-nv12-plane"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &slot.planes.views[plane],
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&pipelines[plane]);
            pass.set_bind_group(0, &bindings, &[]);
            pass.draw(0..3, 0..1);
        }
        slot.busy.store(true, Ordering::Release);
        let lease = Arc::new(ConsumerLease {
            slot: slot.clone(),
            budget: self.budget.clone(),
            waker: self.waker.clone(),
        });
        let surface = slot.surface.clone().with_lease(lease);
        let retained_surface = surface.clone();
        let ready = ConversionReady(Arc::new(Completion {
            ready: AtomicBool::new(false),
            completed_at: OnceLock::new(),
        }));
        let completed = ready.clone();
        let waker = self.waker.clone();
        let trace_enabled = self.trace_enabled;
        self.backend.queue.submit([encoder.finish()]);
        self.backend.queue.on_submitted_work_done(move || {
            if trace_enabled {
                let _ = completed.0.completed_at.set(Instant::now());
            }
            // The retained surface's consumer lease owns Slot, including every
            // CVMetal wrapper and wgpu plane. Cancellation cannot retire it early.
            drop((input, retained_surface, slot));
            completed.0.ready.store(true, Ordering::Release);
            if let Some(waker) = waker {
                waker();
            }
        });
        Ok(ConversionAdmission::Accepted(ConvertedFrame {
            surface,
            ready,
        }))
    }
}

fn create_pipelines(
    device: &wgpu::Device,
    interpretation: InputInterpretation,
) -> (
    InputInterpretation,
    wgpu::BindGroupLayout,
    [wgpu::RenderPipeline; 2],
) {
    let source = SHADER.replace("ALPHA_MODE", &interpretation.shader_alpha().to_string());
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("encoder-nv12-srgb-to-709"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("encoder-nv12-layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("encoder-nv12-pipeline-layout"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipelines = [
        ("fs_y", wgpu::TextureFormat::R8Unorm),
        ("fs_uv", wgpu::TextureFormat::Rg8Unorm),
    ]
    .map(|(entry, format)| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("encoder-nv12-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(entry),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        })
    });
    (interpretation, layout, pipelines)
}

const SHADER: &str = r#"
@group(0) @binding(0) var source: texture_2d<f32>;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    return vec4f(f32(i / 2u) * 4.0 - 1.0, f32(i % 2u) * 4.0 - 1.0, 0.0, 1.0);
}
fn rgb709(p: vec2i) -> vec3f {
    let sample = textureLoad(source, p, 0);
    var encoded = sample.rgb;
    var alpha = sample.a;
    if (ALPHA_MODE == 1u) {
        encoded = select(vec3f(0.0), clamp(encoded / max(alpha, 0.000001), vec3f(0.0), vec3f(1.0)), alpha > 0.0);
    }
    if (ALPHA_MODE == 2u) { alpha = 1.0; }
    let linear = select(pow((encoded + vec3f(0.055)) / 1.055, vec3f(2.4)), encoded / 12.92, encoded <= vec3f(0.04045)) * alpha;
    return select(1.099 * pow(linear, vec3f(0.45)) - vec3f(0.099), 4.5 * linear, linear < vec3f(0.018));
}
fn ycbcr(rgb: vec3f) -> vec3f {
    let y = dot(rgb, vec3f(0.2126, 0.7152, 0.0722));
    return vec3f(y, (rgb.b - y) / 1.8556, (rgb.r - y) / 1.5748);
}
@fragment fn fs_y(@builtin(position) p: vec4f) -> @location(0) vec4f {
    let y = ycbcr(rgb709(vec2i(p.xy))).x;
    return vec4f(floor(clamp(16.0 + 219.0 * y, 16.0, 235.0) + 0.5) / 255.0, 0.0, 0.0, 1.0);
}
@fragment fn fs_uv(@builtin(position) p: vec4f) -> @location(0) vec4f {
    let q = vec2i(p.xy) * 2;
    let rgb = (rgb709(q) + rgb709(q + vec2i(1,0)) + rgb709(q + vec2i(0,1)) + rgb709(q + vec2i(1,1))) * 0.25;
    let uv = ycbcr(rgb).yz;
    let codes = floor(clamp(vec2f(128.0) + 224.0 * uv, vec2f(16.0), vec2f(240.0)) + vec2f(0.5));
    return vec4f(codes / 255.0, 0.0, 1.0);
}
"#;

#[cfg(test)]
#[path = "encode_surface_tests.rs"]
mod tests;
