//! CoreVideo/Metal plane import and GPU color conversion. Call only from the
//! render thread: a bounded wait protects decoder surface reuse.
use std::{collections::HashMap, ffi::c_void, ptr, sync::Mutex, time::Duration};

use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_metal::{MTLTexture, MTLTextureType};
use oq_media::{ColorMatrix, ColorPrimaries, PixelFormat, TransferFunction, VideoToolboxFrame};

use super::{GpuBackend, TextureFormat, TextureHandle};

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
    pipelines: HashMap<u8, (wgpu::BindGroupLayout, wgpu::RenderPipeline)>,
    failed: bool,
}

pub struct VideoToolboxImporter { state: Mutex<State> }

impl VideoToolboxImporter {
    fn new(backend: &GpuBackend) -> Result<Self, String> {
        let device = unsafe { backend.device.as_hal::<wgpu::hal::api::Metal>() }
            .ok_or("VideoToolbox requires the wgpu Metal backend")?;
        let raw_device = Retained::as_ptr(device.raw_device()) as *const c_void;
        let mut cache = ptr::null_mut();
        let status = unsafe { CVMetalTextureCacheCreate(ptr::null(), ptr::null(), raw_device, ptr::null(), &mut cache) };
        if status != 0 || cache.is_null() { return Err(format!("CVMetalTextureCacheCreate failed: {status}")); }
        Ok(Self { state: Mutex::new(State { cache: CfRef(cache), pipelines: HashMap::new(), failed: false }) })
    }
}

impl GpuBackend {
    /// Imports decoder planes without CPU pixel access. This may wait up to five
    /// seconds and therefore must never be called on the UI thread.
    pub fn upload_videotoolbox(&self, frame: &VideoToolboxFrame, output: &TextureHandle) -> Result<(), String> {
        let (width, height) = (frame.width(), frame.height());
        let limit = self.device.limits().max_texture_dimension_2d;
        if width == 0 || height == 0 || width > limit || height > limit || output.width != width || output.height != height || output.format != TextureFormat::Rgba8Unorm {
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
        let importer = self.videotoolbox_importer.get_or_init(|| VideoToolboxImporter::new(self)).as_ref().map_err(Clone::clone)?;
        let mut state = importer.state.lock().map_err(|_| "VideoToolbox importer lock poisoned")?;
        if state.failed { return Err("VideoToolbox GPU completion previously failed".into()); }
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
        let matrix = match frame.color().matrix { ColorMatrix::Bt601 => 0, ColorMatrix::Bt709 => 1, ColorMatrix::Bt2020 => 2 };
        let source_srgb = frame.color().transfer == TransferFunction::Srgb;
        let pal_primaries = frame.color().primaries == ColorPrimaries::Bt470Bg;
        let key = ((format * 6 + matrix * 2 + u8::from(frame.color().full_range)) * 2 + u8::from(source_srgb)) * 2 + u8::from(pal_primaries);
        let (layout, pipeline) = state.pipelines.entry(key).or_insert_with(|| create_pipeline(&self.device, format, matrix, frame.color().full_range, source_srgb, pal_primaries));
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
        // Keep the AVFrame (not just its pixel buffer) out of the decoder pool
        // until completion, including when polling times out.
        let retained_frame = frame.try_clone()?;
        let submission = self.queue.submit([encoder.finish()]);
        self.queue.on_submitted_work_done(move || { drop((retained_frame, cv_textures)); });
        if let Err(error) = self.device.poll(wgpu::PollType::Wait { submission_index: Some(submission), timeout: Some(Duration::from_secs(5)) }) {
            state.failed = true;
            return Err(format!("VideoToolbox conversion GPU wait failed: {error:?}"));
        }
        Ok(())
    }
}

fn create_pipeline(device: &wgpu::Device, format: u8, matrix: u8, full: bool, source_srgb: bool, pal_primaries: bool) -> (wgpu::BindGroupLayout, wgpu::RenderPipeline) {
    let (kr, kb) = match matrix { 0 => (0.299f64, 0.114f64), 1 => (0.2126, 0.0722), _ => (0.2627, 0.0593) };
    let kg = 1.0 - kr - kb;
    let (scale, y_offset, y_range, c_mid, c_range) = match (format, full) {
        (1, false) => (65535.0 / 64.0, 64.0, 876.0, 512.0, 896.0),
        (1, true) => (65535.0 / 64.0, 0.0, 1023.0, 512.0, 1023.0),
        (_, false) => (255.0, 16.0, 219.0, 128.0, 224.0),
        _ => (255.0, 0.0, 255.0, 128.0, 255.0),
    };
    let conversion = if format == 2 { "return vec4f(output_color(textureLoad(y_tex,p,0).rgb),1.0);".to_owned() } else { format!("let y = (textureLoad(y_tex,p,0).r * {scale} - {y_offset}) / {y_range}; let chroma = (textureLoad(uv_tex,c,0).rg * {scale} - vec2f({c_mid})) / {c_range}; let rgb = clamp(vec3f(y + {rv} * chroma.y, y - {gu} * chroma.x - {gv} * chroma.y, y + {bu} * chroma.x), vec3f(0.0), vec3f(1.0)); return vec4f(output_color(rgb),1.0);", rv=2.0*(1.0-kr), gu=2.0*kb*(1.0-kb)/kg, gv=2.0*kr*(1.0-kr)/kg, bu=2.0*(1.0-kb)) };
    let decode_transfer = if source_srgb {
        "let linear = select(pow(max((v + vec3f(0.055)) / 1.055, vec3f(0.0)), vec3f(2.4)), v / 12.92, v <= vec3f(0.04045));"
    } else {
        "let linear = select(pow(max((v + vec3f(0.099)) / 1.099, vec3f(0.0)), vec3f(1.0 / 0.45)), v / 4.5, v < vec3f(0.081));"
    };
    // BT.470BG (PAL) and BT.709 share D65. Convert linear RGB using their
    // chromaticities: PAL R(.64,.33) G(.29,.60) B(.15,.06), 709 G(.30,.60).
    let gamut = if pal_primaries {
        "let rgb709 = vec3f(1.044043209 * linear.r - 0.044043209 * linear.g, linear.g, 0.011793378 * linear.g + 0.988206622 * linear.b);"
    } else { "let rgb709 = linear;" };
    let transfer = if source_srgb && !pal_primaries { "return v;".to_owned() } else { format!(r#"
        {decode_transfer}
        {gamut}
        let encoded = select(1.055 * pow(max(rgb709, vec3f(0.0)), vec3f(1.0 / 2.4)) - vec3f(0.055), 12.92 * rgb709, rgb709 <= vec3f(0.0031308));
        return clamp(encoded, vec3f(0.0), vec3f(1.0));
    "#) };
    let source = format!(r#"
struct O {{ @builtin(position) p: vec4f, @location(0) uv: vec2f }}
@vertex fn vs(@builtin(vertex_index) i: u32) -> O {{ var o: O; let x = f32(i / 2u) * 4.0 - 1.0; let y = f32(i % 2u) * 4.0 - 1.0; o.p = vec4f(x,y,0,1); o.uv = vec2f((x+1.0)*0.5,(1.0-y)*0.5); return o; }}
@group(0) @binding(0) var y_tex: texture_2d<f32>;
@group(0) @binding(1) var uv_tex: texture_2d<f32>;
fn output_color(v: vec3f) -> vec3f {{ {transfer} }}
@fragment fn fs(@location(0) uv: vec2f) -> @location(0) vec4f {{ let ys = textureDimensions(y_tex); let cs = textureDimensions(uv_tex); let p = vec2i(clamp(uv * vec2f(ys), vec2f(0.0), vec2f(ys - 1u))); let c = vec2i(clamp(uv * vec2f(cs), vec2f(0.0), vec2f(cs - 1u))); {conversion} }}
"#);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("videotoolbox-conversion"), source: wgpu::ShaderSource::Wgsl(source.into()) });
    let entries = [0, 1].map(|binding| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("videotoolbox-layout"), entries: &entries });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("videotoolbox-pipeline-layout"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor { label: Some("videotoolbox-pipeline"), layout: Some(&pipeline_layout), vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() }, fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs"), targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba8Unorm, blend: None, write_mask: wgpu::ColorWrites::ALL })], compilation_options: Default::default() }), primitive: Default::default(), depth_stencil: None, multisample: Default::default(), multiview_mask: None, cache: None });
    (layout, pipeline)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoded_surfaces_convert_and_remain_valid_after_frame_release() {
        pollster::block_on(async {
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
            let backend = GpuBackend::from_device(device, queue);
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
}
