//! CoreVideo/Metal plane import and same-queue GPU color conversion.
//! Native owners and shared import credits survive until queue completion.
use std::{collections::HashMap, ffi::c_void, ptr, sync::{Arc, Mutex}};
use crate::operators::CameraLog;
use super::video_color::Conversion;

use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_metal::{MTLTexture, MTLTextureType};
use oq_media::{PixelFormat, VideoToolboxFrame};

use super::{GpuBackend, TextureFormat, TextureHandle};
use crate::resources::{FrameBudget, FrameCredit};

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    fn CVMetalTextureCacheCreate(allocator: *const c_void, attributes: *const c_void, device: *const c_void, texture_attributes: *const c_void, cache: *mut *mut c_void) -> i32;
    fn CVMetalTextureCacheCreateTextureFromImage(allocator: *const c_void, cache: *mut c_void, image: *mut c_void, attributes: *const c_void, format: usize, width: usize, height: usize, plane: usize, texture: *mut *mut c_void) -> i32;
    fn CVMetalTextureGetTexture(texture: *mut c_void) -> *mut c_void;
    fn CVPixelBufferGetWidth(buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetHeight(buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetPlaneCount(buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetWidthOfPlane(buffer: *mut c_void, plane: usize) -> usize;
    fn CVPixelBufferGetHeightOfPlane(buffer: *mut c_void, plane: usize) -> usize;
    fn CVPixelBufferGetPixelFormatType(buffer: *mut c_void) -> u32;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: *const c_void);
}

// CoreFoundation objects here are immutable texture wrappers or a
// cache accessed under the importer mutex. Their ownership can cross threads.
struct CfRef(*mut c_void);
unsafe impl Send for CfRef {}
impl Drop for CfRef {
    fn drop(&mut self) { unsafe { CFRelease(self.0) } }
}

struct State {
    cache: CfRef,
    pipelines: HashMap<u16, (wgpu::BindGroupLayout, wgpu::RenderPipeline)>,
}

pub struct VideoToolboxImporter {
    state: Mutex<State>,
    pending: Arc<Mutex<PendingImports>>,
}

struct PendingImports {
    surfaces: usize,
    bytes: usize,
    max_surfaces: usize,
    max_bytes: usize,
    frame_budget: Option<Arc<FrameBudget>>,
    waker: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl PendingImports {
    fn available(&self, bytes: usize) -> bool {
        self.surfaces < self.max_surfaces && bytes <= self.max_bytes.saturating_sub(self.bytes)
    }
}

// Also rolls back credit when any pre-submission operation fails. Once submitted,
// only the queue's completion callback owns this guard, never the source/session.
struct ImportCredit {
    pending: Arc<Mutex<PendingImports>>,
    bytes: usize,
    _physical_credit: Option<FrameCredit>,
}

impl Drop for ImportCredit {
    fn drop(&mut self) {
        let (waker, budget) = {
            let mut pending = self.pending.lock().unwrap_or_else(|error| error.into_inner());
            pending.surfaces -= 1;
            pending.bytes -= self.bytes;
            (pending.waker.clone(), pending.frame_budget.clone())
        };
        if let Some(waker) = waker { waker(); }
        // Local conversion credit is returned even if a source still retains
        // the same physical allocation. Notify every sharing session in that
        // case too; the legacy single importer waker alone is insufficient.
        if let Some(budget) = budget { budget.notify_waiters(); }
    }
}

impl VideoToolboxImporter {
    fn new(backend: &GpuBackend) -> Result<Self, String> {
        let device = unsafe { backend.device.as_hal::<wgpu::hal::api::Metal>() }
            .ok_or("VideoToolbox requires the wgpu Metal backend")?;
        let raw_device = Retained::as_ptr(device.raw_device()) as *const c_void;
        let mut cache = ptr::null_mut();
        let status = unsafe { CVMetalTextureCacheCreate(ptr::null(), ptr::null(), raw_device, ptr::null(), &mut cache) };
        if status != 0 || cache.is_null() { return Err(format!("CVMetalTextureCacheCreate failed: {status}")); }
        Ok(Self {
            state: Mutex::new(State { cache: CfRef(cache), pipelines: HashMap::new() }),
            pending: Arc::new(Mutex::new(PendingImports {
                surfaces: 0, bytes: 0, max_surfaces: 6, max_bytes: 256 * 1024 * 1024,
                frame_budget: None, waker: None,
            })),
        })
    }
}

impl GpuBackend {
    fn videotoolbox_importer(&self) -> Result<&VideoToolboxImporter, String> {
        self.videotoolbox_importer.get_or_init(|| VideoToolboxImporter::new(self)).as_ref().map_err(Clone::clone)
    }

    pub(crate) fn try_create_video_texture(&self, width: u32, height: u32, format: TextureFormat) -> Result<Option<TextureHandle>, String> {
        let limit = self.device.limits().max_texture_dimension_2d;
        if width == 0 || height == 0 || width > limit || height > limit { return Err("invalid video dimensions".into()); }
        let bytes = u64::from(width).checked_mul(u64::from(height)).and_then(|pixels| pixels.checked_mul(format.bytes_per_pixel() as u64))
            .ok_or("video destination allocation size overflow")?;
        let Some(credit) = self.acquire_texture_credit(&self.frame_budget(), bytes)? else { return Ok(None); };
        let credit = Arc::new(credit);
        let mut texture = self.create_texture(width, height, format);
        texture.texture = self.leased_metal_texture(texture.texture, credit.clone())?;
        texture.view = Arc::new(texture.texture.create_view(&Default::default()));
        texture.lease = Some(credit);
        Ok(Some(texture))
    }

    /// The shared backend cannot silently switch physical coordinators between
    /// sessions while native/source allocations still belong to the old one.
    pub(crate) fn set_videotoolbox_budget(&self, budget: Arc<FrameBudget>) -> Result<(), String> {
        let mut pending = self.videotoolbox_importer()?.pending.lock().map_err(|_| "VideoToolbox credit lock poisoned")?;
        if let Some(existing) = &pending.frame_budget {
            if Arc::ptr_eq(existing, &budget) { return Ok(()); }
            return Err("VideoToolbox backend already has another shared frame budget".into());
        }
        if pending.surfaces != 0 { return Err("cannot attach frame budget to in-flight unbudgeted imports".into()); }
        self.set_frame_budget(Arc::clone(&budget))?;
        pending.frame_budget = Some(budget);
        Ok(())
    }

    pub(crate) fn set_videotoolbox_waker(&self, waker: Arc<dyn Fn() + Send + Sync>) -> Result<(), String> {
        self.videotoolbox_importer()?.pending.lock().map_err(|_| "VideoToolbox credit lock poisoned")?.waker = Some(waker);
        Ok(())
    }

    /// Applies device-wide limits, including imports retained by old sessions.
    /// Lowering a limit does not revoke existing in-flight native owners.
    pub(crate) fn configure_videotoolbox_imports(&self, max_surfaces: usize, max_bytes: usize) -> Result<(), String> {
        if max_surfaces == 0 || max_bytes == 0 { return Err("VideoToolbox import limits must be nonzero".into()); }
        let mut pending = self.videotoolbox_importer()?.pending.lock().map_err(|_| "VideoToolbox credit lock poisoned")?;
        pending.max_surfaces = max_surfaces;
        pending.max_bytes = max_bytes;
        Ok(())
    }

    /// Returns false only for temporary credit exhaustion; an individually
    /// oversized native allocation is a fatal admission error, not a retry loop.
    /// This preflight neither submits work nor retains decoder surfaces.
    #[cfg(test)]
    pub(crate) fn can_upload_videotoolbox(&self, frame: &VideoToolboxFrame) -> Result<bool, String> {
        let bytes = videotoolbox_bytes(frame)?;
        let pending = self.videotoolbox_importer()?.pending.lock().map_err(|_| "VideoToolbox credit lock poisoned")?;
        if bytes > pending.max_bytes { return Err("VideoToolbox surface exceeds import byte budget".into()); }
        if !pending.available(bytes) { return Ok(false); }
        match &pending.frame_budget {
            Some(budget) => budget.can_acquire((0, frame.pixel_buffer() as usize), 1, bytes as u64),
            None => Ok(true),
        }
    }

    /// Conservative physical occupancy: each pending conversion reserves one
    /// native allocation, even if several conversions refer to the same surface.
    pub(crate) fn videotoolbox_pending(&self) -> (usize, usize) {
        let Some(Ok(importer)) = self.videotoolbox_importer.get() else { return (0, 0); };
        let pending = importer.pending.lock().unwrap_or_else(|error| error.into_inner());
        (pending.surfaces, pending.bytes)
    }

    /// Imports decoder planes with no CPU pixel access or host completion wait.
    /// Consumers must submit to this same queue, which orders conversion before
    /// graph reads and stable-output copies without a host synchronization point.
    pub fn upload_videotoolbox(&self, frame: &VideoToolboxFrame, output: &TextureHandle) -> Result<(), String> {
        if self.try_upload_videotoolbox(frame, output)? { Ok(()) }
        else { Err("VideoToolbox import credits exhausted".into()) }
    }

    /// Atomically reserves shared credit; false means retry after GPU progress.
    /// Fatal validation/import failures never masquerade as temporary fullness.
    pub(crate) fn try_upload_videotoolbox(&self, frame: &VideoToolboxFrame, output: &TextureHandle) -> Result<bool, String> {
        self.try_upload_camera_frame(frame, output, None)
    }

    pub(crate) fn try_upload_camera_frame(
        &self, frame: &VideoToolboxFrame, output: &TextureHandle, camera: Option<CameraLog>,
    ) -> Result<bool, String> {
        let (width, height) = (frame.width(), frame.height());
        let limit = self.device.limits().max_texture_dimension_2d;
        if width == 0 || height == 0 || width > limit || height > limit || output.width != width || output.height != height || !matches!(output.format, TextureFormat::Rgba8Unorm | TextureFormat::Rgba16Float) {
            return Err("invalid VideoToolbox frame or output dimensions/format".into());
        }
        let buffer = frame.pixel_buffer();
        if buffer.is_null() { return Err("null VideoToolbox pixel buffer".into()); }
        let (pixel_type, plane_count) = unsafe { (CVPixelBufferGetPixelFormatType(buffer), CVPixelBufferGetPlaneCount(buffer)) };
        let (format, metal_formats, formats) = match frame.format() {
            PixelFormat::Nv12 if pixel_type == u32::from_be_bytes(*b"420v") || pixel_type == u32::from_be_bytes(*b"420f") => (0u8, [10, 30], [wgpu::TextureFormat::R8Unorm, wgpu::TextureFormat::Rg8Unorm]),
            PixelFormat::P010 if pixel_type == u32::from_be_bytes(*b"x420") || pixel_type == u32::from_be_bytes(*b"xf20") => {
                if !self.device.features().contains(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM) { return Err("P010 requires TEXTURE_FORMAT_16BIT_NORM on the wgpu device".into()); }
                (1, [20, 60], [wgpu::TextureFormat::R16Unorm, wgpu::TextureFormat::Rg16Unorm])
            }
            PixelFormat::Bgra8 if pixel_type == u32::from_be_bytes(*b"BGRA") => (2, [80, 80], [wgpu::TextureFormat::Bgra8Unorm, wgpu::TextureFormat::Bgra8Unorm]),
            _ => return Err(format!("unsupported CoreVideo pixel format {pixel_type:#x}")),
        };
        if unsafe { CVPixelBufferGetWidth(buffer) != width as usize || CVPixelBufferGetHeight(buffer) != height as usize } || (format != 2 && plane_count != 2) || (format == 2 && plane_count != 0) {
            return Err("invalid CoreVideo pixel buffer dimensions or planes".into());
        }
        if format != 2 {
            let full = pixel_type == u32::from_be_bytes(*b"420f") || pixel_type == u32::from_be_bytes(*b"xf20");
            if full != frame.color().full_range { return Err("CoreVideo pixel format disagrees with color range".into()); }
        }
        let importer = self.videotoolbox_importer()?;
        let bytes = videotoolbox_bytes(frame)?;
        let credit = {
            let mut pending = importer.pending.lock().map_err(|_| "VideoToolbox credit lock poisoned")?;
            if bytes > pending.max_bytes { return Err("VideoToolbox surface exceeds import byte budget".into()); }
            if !pending.available(bytes) { return Ok(false); }
            let physical_credit = if let Some(budget) = &pending.frame_budget {
                match budget.try_acquire((0, buffer as usize), 1, bytes as u64)? {
                    Some(credit) => Some(credit),
                    None => return Ok(false),
                }
            } else { None };
            pending.surfaces += 1;
            pending.bytes += bytes;
            ImportCredit { pending: Arc::clone(&importer.pending), bytes, _physical_credit: physical_credit }
        };
        let mut state = importer.state.lock().map_err(|_| "VideoToolbox importer lock poisoned")?;
        let mut cv_textures: [Option<CfRef>; 2] = [None, None];
        let mut textures: [Option<wgpu::Texture>; 2] = [None, None];
        for plane in 0..if format == 2 { 1 } else { 2 } {
            let (pw, ph) = if format == 2 { (width as usize, height as usize) } else { unsafe { (CVPixelBufferGetWidthOfPlane(buffer, plane), CVPixelBufferGetHeightOfPlane(buffer, plane)) } };
            let expected = if plane == 0 { (width as usize, height as usize) } else { ((width as usize + 1) / 2, (height as usize + 1) / 2) };
            if (pw, ph) != expected { return Err("invalid CoreVideo plane dimensions".into()); }
            let mut cv_texture = ptr::null_mut();
            let status = unsafe { CVMetalTextureCacheCreateTextureFromImage(ptr::null(), state.cache.0, buffer, ptr::null(), metal_formats[plane], pw, ph, plane, &mut cv_texture) };
            if status != 0 || cv_texture.is_null() { return Err(format!("CVMetalTextureCacheCreateTextureFromImage plane {plane} failed: {status}")); }
            let cv_texture = CfRef(cv_texture);
            let raw = unsafe { CVMetalTextureGetTexture(cv_texture.0) };
            let raw = unsafe { Retained::<ProtocolObject<dyn MTLTexture>>::retain(raw.cast()) }.ok_or("CVMetalTexture has no Metal texture")?;
            let size = wgpu::Extent3d { width: pw as u32, height: ph as u32, depth_or_array_layers: 1 };
            let hal_texture = unsafe { wgpu::hal::metal::Device::texture_from_raw(raw, formats[plane], MTLTextureType::Type2D, 1, 1, size.into(), None) };
            let texture = unsafe { self.device.create_texture_from_hal::<wgpu::hal::api::Metal>(hal_texture, &wgpu::TextureDescriptor { label: Some("videotoolbox-plane"), size, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: formats[plane], usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] }, wgpu::TextureUses::RESOURCE) };
            textures[plane] = Some(texture);
            cv_textures[plane] = Some(cv_texture);
        }
        let conversion = Conversion::new(format, frame.color(), output.format == TextureFormat::Rgba16Float, camera);
        let (layout, pipeline) = state.pipelines.entry(conversion.key()).or_insert_with(|| conversion.create(&self.device));
        let view0 = textures[0].as_ref().unwrap().create_view(&Default::default());
        let view1 = textures[if format == 2 { 0 } else { 1 }].as_ref().unwrap().create_view(&Default::default());
        let bindings = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("videotoolbox-planes"), layout, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view0) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view1) },
        ] });
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("videotoolbox-convert") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("videotoolbox-convert"), color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &output.view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })], depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bindings, &[]);
            pass.draw(0..3, 0..1);
        }
        // The AVFrame owns the decoder-pool/CVPixelBuffer reference; CV wrappers
        // own the imported Metal planes. Retain both, as well as wgpu handles,
        // until actual completion. Source cancellation or reset cannot release
        // them. On device loss, wgpu retires work before destroying callbacks.
        let retained_frame = frame.try_clone()?;
        drop(state);
        self.queue.submit([encoder.finish()]);
        self.queue.on_submitted_work_done(move || {
            drop((retained_frame, cv_textures, textures));
            drop(credit);
        });
        Ok(true)
    }
}

fn videotoolbox_bytes(frame: &VideoToolboxFrame) -> Result<usize, String> {
    if frame.pixel_buffer().is_null() { return Err("null VideoToolbox pixel buffer".into()); }
    let bytes = usize::try_from(frame.allocation_size()).map_err(|_| "VideoToolbox allocation size overflows address space")?;
    if bytes == 0 { return Err("VideoToolbox surface has no measurable allocation".into()); }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn metal_backend() -> GpuBackend {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        }).await.expect("Metal GPU required for VideoToolbox test");
        let features = adapter.features() & wgpu::Features::TEXTURE_FORMAT_16BIT_NORM;
        let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: features,
            ..Default::default()
        }).await.unwrap();
        GpuBackend::from_device(device, queue)
    }

    #[test]
    fn decoded_surfaces_convert_and_remain_valid_after_frame_release() {
        pollster::block_on(async {
            let backend = metal_backend().await;
            let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures");
            let mut white = oq_media::Session::open(fixtures.join("white_64x64.mp4")).unwrap();
            let frame = white.next_frame().unwrap().expect("white fixture frame");
            let output = backend.create_texture(frame.frame.width(), frame.frame.height(), TextureFormat::Rgba8Unorm);
            backend.upload_videotoolbox(&frame.frame, &output).unwrap();
            drop(frame);
            drop(white);
            let mut gray = oq_media::Session::open(fixtures.join("gray_64x64.mp4")).unwrap();
            let frame = gray.next_frame().unwrap().expect("gray fixture frame");
            let gray_output = backend.create_texture(frame.frame.width(), frame.frame.height(), TextureFormat::Rgba8Unorm);
            backend.upload_videotoolbox(&frame.frame, &gray_output).unwrap();
            drop(frame);
            drop(gray);
            let white_pixels = backend.read_texture_rgba(&output.texture, output.width, output.height).await.unwrap();
            let gray_pixels = backend.read_texture_rgba(&gray_output.texture, gray_output.width, gray_output.height).await.unwrap();
            for (w, g) in white_pixels.chunks_exact(4).zip(gray_pixels.chunks_exact(4)) {
                assert!(w[0] > 220 && w[1] > 220 && w[2] > 220 && w[3] == 255, "white pixel: {w:?}");
                assert!(g[0] > 40 && g[0] < 220 && g[0].abs_diff(g[1]) <= 4 && g[0].abs_diff(g[2]) <= 4 && g[3] == 255, "gray pixel: {g:?}");
            }
        });
    }

    #[test]
    fn alternating_sources_submit_without_import_wait_and_survive_decoder_drop() {
        pollster::block_on(async {
            let backend = metal_backend().await;
            backend.configure_videotoolbox_imports(6, 256 * 1024 * 1024).unwrap();
            let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
            let mut outputs = Vec::new();
            for index in 0..6 {
                let white = index % 2 == 0;
                let mut source = oq_media::Session::open(fixtures.join(if white { "white_64x64.mp4" } else { "gray_64x64.mp4" })).unwrap();
                let frame = source.next_frame().unwrap().expect("decoded fixture frame");
                let output = backend.create_texture(frame.frame.width(), frame.frame.height(), TextureFormat::Rgba8Unorm);
                assert!(backend.try_upload_videotoolbox(&frame.frame, &output).unwrap());
                drop(frame);
                drop(source);
                outputs.push((white, output));
                let (surfaces, bytes) = backend.videotoolbox_pending();
                assert!((1..=6).contains(&surfaces), "latest import must retain its native owners without polling");
                assert!(bytes > 0 && bytes <= 256 * 1024 * 1024);
            }
            // No import waited/polled. All original decoders and leases are gone
            // before the first readback submission drains their completion work.
            for (white, output) in outputs {
                let pixels = backend.read_texture_rgba(&output.texture, output.width, output.height).await.unwrap();
                for pixel in pixels.chunks_exact(4) {
                    if white {
                        assert!(pixel[0] > 220 && pixel[1] > 220 && pixel[2] > 220 && pixel[3] == 255, "white pixel: {pixel:?}");
                    } else {
                        assert!(pixel[0] > 40 && pixel[0] < 220 && pixel[0].abs_diff(pixel[1]) <= 4 && pixel[0].abs_diff(pixel[2]) <= 4 && pixel[3] == 255, "gray pixel: {pixel:?}");
                    }
                }
            }
            assert_eq!(backend.videotoolbox_pending(), (0, 0));
        });
    }

    #[test]
    fn shared_import_surface_and_byte_credits_release_only_after_gpu_completion() {
        pollster::block_on(async {
            let backend = metal_backend().await;
            let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/white_64x64.mp4");
            let mut source = oq_media::Session::open(fixture).unwrap();
            let frame = source.next_frame().unwrap().expect("decoded fixture frame");
            let output = backend.create_texture(frame.frame.width(), frame.frame.height(), TextureFormat::Rgba8Unorm);
            let bytes = videotoolbox_bytes(&frame.frame).unwrap();
            assert!(backend.configure_videotoolbox_imports(0, bytes).is_err());
            backend.configure_videotoolbox_imports(2, bytes).unwrap();
            assert!(backend.can_upload_videotoolbox(&frame.frame).unwrap());
            assert!(backend.try_upload_videotoolbox(&frame.frame, &output).unwrap());
            assert_eq!(backend.videotoolbox_pending(), (1, bytes));
            // Byte saturation must reject without submitting work or overwriting
            // the already submitted output, even though a surface slot remains.
            assert!(!backend.can_upload_videotoolbox(&frame.frame).unwrap());
            assert!(!backend.try_upload_videotoolbox(&frame.frame, &output).unwrap());
            assert_eq!(backend.videotoolbox_pending(), (1, bytes));
            // A new session's changed limits cannot reclaim old native owners.
            backend.configure_videotoolbox_imports(1, bytes * 2).unwrap();
            assert!(!backend.can_upload_videotoolbox(&frame.frame).unwrap());
            assert!(!backend.try_upload_videotoolbox(&frame.frame, &output).unwrap());
            backend.configure_videotoolbox_imports(2, bytes - 1).unwrap();
            assert!(backend.can_upload_videotoolbox(&frame.frame).is_err());
            assert!(backend.try_upload_videotoolbox(&frame.frame, &output).is_err());
            assert_eq!(backend.videotoolbox_pending(), (1, bytes));
            let pixels = backend.read_texture_rgba(&output.texture, output.width, output.height).await.unwrap();
            assert!(pixels.chunks_exact(4).all(|pixel| pixel[0] > 220 && pixel[1] > 220 && pixel[2] > 220 && pixel[3] == 255));
            assert_eq!(backend.videotoolbox_pending(), (0, 0));
            backend.configure_videotoolbox_imports(1, bytes).unwrap();
            assert!(backend.can_upload_videotoolbox(&frame.frame).unwrap());
            assert!(backend.try_upload_videotoolbox(&frame.frame, &output).unwrap());
            drop(frame);
            drop(source);
            let pixels = backend.read_texture_rgba(&output.texture, output.width, output.height).await.unwrap();
            assert!(pixels.chunks_exact(4).all(|pixel| pixel[0] > 220 && pixel[1] > 220 && pixel[2] > 220 && pixel[3] == 255));
            assert_eq!(backend.videotoolbox_pending(), (0, 0));
        });
    }

    #[test]
    fn source_and_importer_share_one_physical_charge_until_completion() {
        pollster::block_on(async {
            let backend = metal_backend().await;
            let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/white_64x64.mp4");
            let mut source = oq_media::Session::open(fixture).unwrap();
            let frame = source.next_frame().unwrap().expect("decoded fixture frame");
            let bytes = frame.frame.allocation_size();
            let budget = FrameBudget::new(1, bytes).unwrap();
            backend.set_videotoolbox_budget(budget.clone()).unwrap();
            let source_credit = budget.try_acquire((0, frame.frame.pixel_buffer() as usize), 1, bytes).unwrap().unwrap();
            let output = backend.create_texture(frame.frame.width(), frame.frame.height(), TextureFormat::Rgba8Unorm);
            assert!(backend.try_upload_videotoolbox(&frame.frame, &output).unwrap());
            assert_eq!(budget.usage(), (1, bytes));
            drop((source_credit, frame, source));
            assert_eq!(budget.usage(), (1, bytes));
            assert!(budget.try_acquire((2, 1), 1, bytes).unwrap().is_none());
            let pixels = backend.read_texture_rgba(&output.texture, output.width, output.height).await.unwrap();
            assert!(pixels.chunks_exact(4).all(|pixel| pixel[0] > 220 && pixel[1] > 220 && pixel[2] > 220 && pixel[3] == 255));
            assert_eq!(budget.usage(), (0, 0));
            let retry = budget.try_acquire((2, 1), 1, bytes).unwrap().unwrap();
            drop(retry);
        });
    }

    #[test]
    fn prepared_video_destination_credit_survives_executor_and_native_binding_owners() {
        pollster::block_on(async {
            let backend = Arc::new(metal_backend().await);
            let budget = FrameBudget::new(1, 16).unwrap();
            backend.set_frame_budget(budget.clone()).unwrap();
            let mut executor = crate::rendering::GpuExecutor::new(backend.clone());
            assert!(executor.prepare_video_texture("video", 2, 2).unwrap());
            assert_eq!(budget.usage(), (1, 16));
            assert!(executor.prepare_video_texture("video", 2, 2).unwrap());
            assert!(!executor.prepare_video_texture("other", 2, 2).unwrap());
            assert!(executor.prepare_video_texture("video", 4, 4).is_err());
            assert_eq!(executor.output_handle("video").unwrap().width, 2);
            executor.upload_rgba("video", &[123, 45, 67, 255].repeat(4), 2, 2).unwrap();
            let output = executor.output_handle("video").unwrap();
            let pixels = backend.read_texture_rgba(&output.texture, 2, 2).await.unwrap();
            assert_eq!(pixels, [123, 45, 67, 255].repeat(4));
            let view = output.texture.create_view(&Default::default());
            let layout = backend.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("video-credit-owner-test"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                }],
            });
            let binding = backend.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("video-credit-owner-test"), layout: &layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }],
            });
            drop((view, output, executor));
            backend.device.poll(wgpu::PollType::Poll).unwrap();
            assert_eq!(budget.usage(), (1, 16), "native binding, not executor lifetime, owns the physical charge");
            drop(binding);
            for _ in 0..3 {
                backend.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(std::time::Duration::from_secs(5)) }).unwrap();
            }
            assert_eq!(budget.usage(), (0, 0));
        });
    }
}
