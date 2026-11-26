use open_quartz::outputs::{FrameColorSpace, FrameImageDescription, FrameSnapshotIdentity};
use open_quartz::rendering::{GpuBackend, GpuOutputHandle, StableFrame};
use slint::winit_030::{
    invoke_from_active_event_loop, winit, CustomApplicationHandler, EventResult,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use wgpu::{SurfaceColorSpace, TextureFormat};
use winit::{
    event::WindowEvent,
    event_loop::ActiveEventLoop,
    window::{Window, WindowId},
};

const SDR_FALLBACK: &str = "HDR monitor · SDR fallback";
const NATIVE_HDR: &str = "HDR monitor · native HDR";
const WAITING: &str = "HDR monitor waiting for managed sequence frame";
const CLOSED: &str = "HDR monitor closed";
const OPENING: &str = "Opening HDR monitor";

#[derive(Clone)]
pub struct Monitor(Arc<Mutex<State>>);

struct State {
    gpu: Option<Gpu>,
    monitor: Option<NativeWindow>,
    sdr: Option<Blit>,
    status: &'static str,
}

#[derive(Clone)]
struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

struct NativeWindow {
    surface: wgpu::Surface<'static>,
    window: Arc<Window>,
    config: wgpu::SurfaceConfiguration,
    blit: Blit,
    frame: Option<StableFrame>,
    probe_at: Instant,
    parameters: [f32; 4],
    needs_configure: bool,
}

#[derive(Clone)]
pub struct DisplayFrame {
    output: GpuOutputHandle,
    pub image: FrameImageDescription,
    pub identity: FrameSnapshotIdentity,
}

impl DisplayFrame {
    pub fn output(&self) -> &GpuOutputHandle {
        &self.output
    }
}

impl Monitor {
    pub fn new(configuration: &slint::wgpu_30::WGPUConfiguration) -> Self {
        use slint::wgpu_30::WGPUConfiguration;
        let gpu = match configuration {
            WGPUConfiguration::Manual {
                instance,
                adapter,
                device,
                queue,
            } => Some(Gpu {
                instance: instance.clone(),
                adapter: adapter.clone(),
                device: device.clone(),
                queue: queue.clone(),
            }),
            _ => None,
        };
        Self(Arc::new(Mutex::new(State {
            gpu,
            monitor: None,
            sdr: None,
            status: CLOSED,
        })))
    }

    #[cfg(test)]
    pub fn disabled() -> Self {
        Self(Arc::new(Mutex::new(State {
            gpu: None,
            monitor: None,
            sdr: None,
            status: CLOSED,
        })))
    }

    pub fn status(&self) -> (&'static str, bool) {
        let state = self.0.lock().unwrap();
        (state.status, state.monitor.is_some())
    }

    pub fn toggle(&self) {
        let mut state = self.0.lock().unwrap();
        if state.monitor.take().is_some() || state.status == OPENING {
            state.status = CLOSED;
            return;
        }
        state.status = OPENING;
        let monitor = self.clone();
        if invoke_from_active_event_loop(move |event_loop| {
            let mut state = monitor.0.lock().unwrap();
            if state.status != OPENING {
                return;
            }
            match state
                .gpu
                .as_ref()
                .and_then(|gpu| NativeWindow::open(gpu, event_loop))
            {
                Some(window) => {
                    state.monitor = Some(window);
                    state.status = WAITING;
                }
                None => state.status = "HDR monitor unavailable",
            }
        })
        .is_err()
        {
            state.status = "HDR monitor unavailable";
        }
    }

    pub fn close(&self) {
        let mut state = self.0.lock().unwrap();
        state.monitor = None;
        state.status = CLOSED;
    }

    pub fn refresh_display(&self) {
        let mut state = self.0.lock().unwrap();
        let State { gpu, monitor, .. } = &mut *state;
        if let (Some(gpu), Some(monitor)) = (gpu, monitor) {
            if Instant::now() >= monitor.probe_at {
                monitor.probe(gpu);
                monitor.window.request_redraw();
            }
        }
    }

    pub fn present(
        &self,
        backend: &GpuBackend,
        frame: StableFrame,
    ) -> Result<DisplayFrame, String> {
        let mut state = self.0.lock().unwrap();
        let State {
            gpu, monitor, sdr, ..
        } = &mut *state;
        let input = frame.output().texture.as_ref();
        let scene_linear = frame.image.color_space == FrameColorSpace::AcesCgLinear;
        if let Some(monitor) = monitor {
            monitor.frame = scene_linear.then(|| frame.clone());
            monitor.window.request_redraw();
        }
        if !scene_linear {
            return Ok(DisplayFrame {
                image: frame.image.clone(),
                identity: frame.identity.clone(),
                output: frame.into_output(),
            });
        }
        let gpu = gpu
            .as_ref()
            .ok_or("Studio HDR display transform needs the shared GPU")?;
        let blit = sdr.get_or_insert_with(|| Blit::new(gpu, TextureFormat::Rgba8Unorm));
        let (width, height) = (input.width(), input.height());
        static NEXT_SURFACE: AtomicUsize = AtomicUsize::new(0);
        let bytes = u64::from(width) * u64::from(height) * 4;
        let credit = backend
            .frame_budget()
            .try_acquire((4, NEXT_SURFACE.fetch_add(1, Ordering::Relaxed)), 1, bytes)?
            .ok_or("Studio SDR display surface exceeds the GPU frame budget")?;
        let format = open_quartz::rendering::TextureFormat::Rgba8Unorm;
        let output = backend.create_texture(width, height, format);
        blit.draw(gpu, Some(input), &output.texture, [1.0, 1.0, 0.0, 0.0]);
        let mut image = frame.image.clone();
        let mut identity = frame.identity.clone();
        image.color_space = FrameColorSpace::Srgb;
        identity.color_space = FrameColorSpace::Srgb;
        let lease = Arc::new(credit);
        let submitted = (frame, lease.clone());
        gpu.queue.on_submitted_work_done(move || drop(submitted));
        Ok(DisplayFrame {
            image,
            identity,
            output: GpuOutputHandle {
                texture: output.texture,
                view: output.view,
                sampler: output.sampler,
                width,
                height,
                format,
                lease: Some(lease),
            },
        })
    }
}

impl CustomApplicationHandler for Monitor {
    fn window_event(
        &mut self,
        _: &ActiveEventLoop,
        id: WindowId,
        _: Option<&Window>,
        _: Option<&slint::Window>,
        event: &WindowEvent,
    ) -> EventResult {
        let mut state = self.0.lock().unwrap();
        let State {
            gpu,
            monitor,
            status,
            ..
        } = &mut *state;
        let Some(window) = monitor.as_mut().filter(|window| window.window.id() == id) else {
            return EventResult::Propagate;
        };
        match event {
            WindowEvent::CloseRequested | WindowEvent::Destroyed => {
                *monitor = None;
                *status = CLOSED;
            }
            WindowEvent::Resized(_)
            | WindowEvent::ScaleFactorChanged { .. }
            | WindowEvent::Moved(_) => {
                window.probe_at = Instant::now();
                window.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                let gpu = gpu.as_ref().expect("native monitor has a GPU");
                match window.redraw(gpu) {
                    Ok(hdr) => {
                        *status = if window.frame.is_none() {
                            WAITING
                        } else if hdr {
                            NATIVE_HDR
                        } else {
                            SDR_FALLBACK
                        }
                    }
                    Err(()) => {
                        *monitor = None;
                        *status = "HDR monitor surface unavailable";
                    }
                }
            }
            _ => {}
        }
        EventResult::PreventDefault
    }
}

impl NativeWindow {
    fn open(gpu: &Gpu, event_loop: &ActiveEventLoop) -> Option<Self> {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("OpenQuartz Studio · HDR monitor")
                        .with_inner_size(winit::dpi::LogicalSize::new(960, 540)),
                )
                .ok()?,
        );
        let surface = gpu.instance.create_surface(window.clone()).ok()?;
        let caps = surface.get_capabilities(&gpu.adapter);
        let size = window.inner_size();
        let mut config =
            surface.get_default_config(&gpu.adapter, size.width.max(1), size.height.max(1))?;
        if caps.format_capabilities.iter().any(|entry| {
            entry.format == TextureFormat::Rgba16Float
                && entry
                    .color_spaces
                    .contains(wgpu::SurfaceColorSpaces::EXTENDED_SRGB_LINEAR)
        }) {
            config.format = TextureFormat::Rgba16Float;
            config.color_space = SurfaceColorSpace::ExtendedSrgbLinear;
        }
        let blit = Blit::new(gpu, config.format);
        window.request_redraw();
        Some(Self {
            surface,
            window,
            config,
            blit,
            frame: None,
            probe_at: Instant::now(),
            parameters: [1.0, 1.0, 0.0, 0.0],
            needs_configure: true,
        })
    }

    fn redraw(&mut self, gpu: &Gpu) -> Result<bool, ()> {
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(self.parameters[2] != 0.0);
        }
        if self.needs_configure
            || (self.config.width, self.config.height) != (size.width, size.height)
        {
            self.config.width = size.width;
            self.config.height = size.height;
            self.surface.configure(&gpu.device, &self.config);
            self.needs_configure = false;
            self.probe(gpu);
        }
        let hdr = self.parameters[2] != 0.0;
        let texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) => texture,
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
                self.needs_configure = true;
                texture
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.needs_configure = true;
                return Ok(hdr);
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.surface = gpu
                    .instance
                    .create_surface(self.window.clone())
                    .map_err(|_| ())?;
                self.needs_configure = true;
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(hdr)
            }
            wgpu::CurrentSurfaceTexture::Validation => return Err(()),
        };
        self.blit.draw(
            gpu,
            self.frame
                .as_ref()
                .map(|frame| frame.output().texture.as_ref()),
            &texture.texture,
            self.parameters,
        );
        self.window.pre_present_notify();
        gpu.queue.present(texture);
        Ok(hdr)
    }

    fn probe(&mut self, gpu: &Gpu) {
        let display = self.surface.display_hdr_info(&gpu.adapter);
        let white = if cfg!(windows) {
            display
                .luminance
                .and_then(|nits| nits.sdr_white_nits)
                .unwrap_or(80.0)
                / 80.0
        } else {
            1.0
        };
        let headroom = display.tone_map_headroom();
        #[cfg(windows)]
        let headroom = windows_display_headroom(&self.window, white).or(headroom);
        self.parameters = display_parameters(
            self.config.color_space == SurfaceColorSpace::ExtendedSrgbLinear,
            headroom,
            white,
            self.config.format.is_srgb(),
        );
        self.probe_at = Instant::now() + Duration::from_millis(500);
    }
}

#[cfg(windows)]
fn windows_display_headroom(window: &Window, white: f32) -> Option<f32> {
    use windows::{
        core::Interface,
        Win32::{
            Foundation::HWND,
            Graphics::{Dxgi, Gdi},
        },
    };
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::Win32(raw) = handle.as_raw() else {
        return None;
    };
    let hwnd = HWND(raw.hwnd.get() as *mut _);
    // A DXGI factory can retain SDR properties across an HDR mode change even
    // when the set of connected outputs has not changed. Re-enumerate per probe.
    // SAFETY: the winit window owns hwnd, COM interfaces remain live inside the
    // loops, and the windows bindings allocate each method's output.
    unsafe {
        let monitor = Gdi::MonitorFromWindow(hwnd, Gdi::MONITOR_DEFAULTTONEAREST);
        if monitor.is_invalid() {
            return None;
        }
        let factory: Dxgi::IDXGIFactory1 = Dxgi::CreateDXGIFactory1().ok()?;
        for adapter in (0..).map_while(|index| factory.EnumAdapters1(index).ok()) {
            for output in (0..).map_while(|index| adapter.EnumOutputs(index).ok()) {
                let desc = output
                    .cast::<Dxgi::IDXGIOutput6>()
                    .ok()
                    .and_then(|output| output.GetDesc1().ok());
                let Some(desc) = desc.filter(|desc| desc.Monitor == monitor) else {
                    continue;
                };
                return Some(
                    if desc.ColorSpace == Dxgi::Common::DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020 {
                        desc.MaxLuminance / (white * 80.0)
                    } else {
                        1.0
                    },
                );
            }
        }
    }
    None
}

fn display_parameters(extended: bool, headroom: Option<f32>, white: f32, srgb: bool) -> [f32; 4] {
    let headroom = headroom.filter(|value| extended && value.is_finite() && *value > 1.01);
    [
        headroom.unwrap_or(1.0),
        headroom.map_or(1.0, |_| white),
        if headroom.is_some() { 1.0 } else { 0.0 },
        if extended || srgb { 1.0 } else { 0.0 },
    ]
}

struct Blit {
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    parameters: wgpu::Buffer,
}

impl Blit {
    fn new(gpu: &Gpu, format: TextureFormat) -> Self {
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Studio monitor display transform"),
                source: wgpu::ShaderSource::Wgsl(
                    format!(
                        "{}{}",
                        open_quartz::operators::WGSL_COLOR_TRANSFORMS,
                        include_str!("hdr_monitor.wgsl")
                    )
                    .into(),
                ),
            });
        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Studio monitor blit"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vertex"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fragment"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            min_filter: wgpu::FilterMode::Linear,
            mag_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let parameters = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Studio display headroom"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            sampler,
            parameters,
        }
    }

    fn draw(
        &self,
        gpu: &Gpu,
        input: Option<&wgpu::Texture>,
        target: &wgpu::Texture,
        parameters: [f32; 4],
    ) {
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let view = target.create_view(&Default::default());
        let bindings = input.map(|input| {
            let bytes: Vec<u8> = parameters.into_iter().flat_map(f32::to_ne_bytes).collect();
            gpu.queue.write_buffer(&self.parameters, 0, &bytes);
            let input_view = input.create_view(&Default::default());
            gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Studio program frame"),
                layout: &self.pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&input_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.parameters.as_entire_binding(),
                    },
                ],
            })
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Studio display monitor"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let (Some(input), Some(bindings)) = (input, &bindings) {
                let w = target.width() as f32;
                let h = target.height() as f32;
                let scale = (w / input.width() as f32).min(h / input.height() as f32);
                let (width, height) = (input.width() as f32 * scale, input.height() as f32 * scale);
                pass.set_viewport(
                    (w - width) * 0.5,
                    (h - height) * 0.5,
                    width,
                    height,
                    0.0,
                    1.0,
                );
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, bindings, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        gpu.queue.submit([encoder.finish()]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_change_recalculates_native_headroom_or_sdr_fallback() {
        assert_eq!(
            display_parameters(true, Some(3.0), 2.5, false),
            [3.0, 2.5, 1.0, 1.0]
        );
        assert_eq!(
            display_parameters(true, Some(1.0), 2.5, false),
            [1.0, 1.0, 0.0, 1.0]
        );
        assert_eq!(
            display_parameters(true, None, 2.5, false),
            [1.0, 1.0, 0.0, 1.0]
        );
        assert_eq!(
            display_parameters(false, Some(3.0), 2.5, false),
            [1.0, 1.0, 0.0, 0.0]
        );
    }

    #[test]
    #[ignore = "requires the native Studio GPU"]
    fn float_surface_preserves_extended_highlights_and_falls_back_to_sdr() {
        let (mut bridge, config, _) = crate::preview::PreviewBridge::new().unwrap();
        let monitor = Monitor::new(&config);
        let gpu = monitor.0.lock().unwrap().gpu.clone().unwrap();
        let format = open_quartz::rendering::TextureFormat::Rgba16Float;
        let input = bridge.backend.create_texture(1, 1, format);
        let target = bridge.backend.create_texture(1, 1, format);
        // Neutral AP1 at 4.0 with opaque alpha, encoded as IEEE binary16.
        gpu.queue.write_texture(
            input.texture.as_image_copy(),
            &[0, 0x44, 0, 0x44, 0, 0x44, 0, 0x3c],
            wgpu::TexelCopyBufferLayout::default(),
            input.texture.size(),
        );
        let blit = Blit::new(&gpu, TextureFormat::Rgba16Float);
        let read_level = |headroom| {
            blit.draw(
                &gpu,
                Some(&input.texture),
                &target.texture,
                display_parameters(true, Some(headroom), 1.0, false),
            );
            let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 8,
                mapped_at_creation: false,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            });
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(
                target.texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout::default(),
                },
                target.texture.size(),
            );
            let submitted = gpu.queue.submit([encoder.finish()]);
            let (done, result) = std::sync::mpsc::sync_channel(1);
            let slice = buffer.slice(..);
            slice.map_async(wgpu::MapMode::Read, move |value| {
                let _ = done.send(value);
            });
            gpu.device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submitted),
                    timeout: Some(Duration::from_secs(5)),
                })
                .unwrap();
            result
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap();
            let data = slice.get_mapped_range().unwrap();
            u16::from_le_bytes([data[0], data[1]])
        };
        let hdr = read_level(3.0);
        let sdr = read_level(1.0);
        assert!(
            (0x4000..0x4200).contains(&hdr),
            "expected 2–3 in the native HDR buffer, got {hdr:04x}"
        );
        assert!(
            (0x3a50..0x3a80).contains(&sdr),
            "expected ~0.8 in the linear SDR fallback, got {sdr:04x}"
        );
        bridge.shutdown();
    }
}
