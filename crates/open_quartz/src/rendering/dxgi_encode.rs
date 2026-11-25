use super::{
    encode_color::{create_pipelines, plane_formats, InputInterpretation},
    encode_frame::{Completion, ConversionAdmission, ConversionReady, ConvertedFrame},
    GpuBackend, StableFrame, TextureHandle,
};
use crate::{
    outputs::{FrameAlphaMode, FrameColorSpace},
    resources::{FrameBudget, FrameCredit},
};
use oq_media::{
    d3d12_encoder::{D3d12Encoder, FramePool, InputSurface, Profile},
    RationalTimeBase,
};
use std::{
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
    time::Instant,
};
use windows_graphics::{
    core::Interface,
    Win32::Graphics::{
        Direct3D12::{ID3D12Device, ID3D12Resource},
        Dxgi::Common::{DXGI_FORMAT_NV12, DXGI_FORMAT_P010},
    },
};

/// Reusable render targets for the two planes of one fixed delivery format.
pub struct D3d12ColorConverter {
    backend: Arc<GpuBackend>,
    interpretation: InputInterpretation,
    width: u32,
    height: u32,
    planes: [wgpu::Texture; 2],
    views: [wgpu::TextureView; 2],
    layout: wgpu::BindGroupLayout,
    pipelines: [wgpu::RenderPipeline; 2],
    staging: wgpu::Buffer,
    pitch: u32,
}

struct ExportSlot {
    surface: InputSurface,
    _budget: FrameCredit,
    busy: AtomicBool,
}

struct ExportLease {
    slot: Arc<ExportSlot>,
    budget: Arc<FrameBudget>,
}

impl Drop for ExportLease {
    fn drop(&mut self) {
        self.slot.busy.store(false, Ordering::Release);
        self.budget.notify_waiters();
    }
}

/// Fixed GPU conversion working set shared with the hardware encoder.
pub struct Nv12Converter {
    gpu: D3d12ColorConverter,
    slots: Vec<Arc<ExportSlot>>,
    budget: Arc<FrameBudget>,
    waker: Option<Arc<dyn Fn() + Send + Sync>>,
    trace_enabled: bool,
}

impl Nv12Converter {
    pub(crate) fn for_pool(
        backend: Arc<GpuBackend>,
        width: u32,
        height: u32,
        interpretation: InputInterpretation,
        pool: FramePool,
        count: usize,
        max_bytes: u64,
    ) -> Result<Self, String> {
        let budget = backend.frame_budget();
        let row = (u64::from(width)
            * if interpretation.encoder_profile().is_hdr() {
                2
            } else {
                1
            })
        .div_ceil(256)
            * 256;
        let surface_bytes =
            (row * (u64::from(height) + u64::from(height).div_ceil(2))).div_ceil(65_536) * 65_536;
        let bytes = surface_bytes
            .checked_mul(count as u64)
            .ok_or("D3D12 working set overflow")?;
        if count == 0 || bytes == 0 || bytes > max_bytes {
            return Err("D3D12 encoder working set exceeds the export budget".into());
        }
        budget.validate_working_set(count, bytes)?;
        let mut slots = Vec::with_capacity(count);
        for _ in 0..count {
            let surface = pool.allocate().map_err(|error| error.to_string())?;
            let credit = budget
                .try_acquire(
                    (0, surface.texture_ptr().as_ptr() as usize),
                    1,
                    surface_bytes,
                )?
                .ok_or("Frame budget cannot reserve the D3D12 encoder working set")?;
            slots.push(Arc::new(ExportSlot {
                surface,
                _budget: credit,
                busy: AtomicBool::new(false),
            }));
        }
        Ok(Self {
            gpu: D3d12ColorConverter::new(backend, width, height, interpretation)?,
            slots,
            budget,
            waker: None,
            trace_enabled: false,
        })
    }

    pub fn set_waker(&mut self, waker: Arc<dyn Fn() + Send + Sync>) {
        self.waker = Some(waker);
    }

    pub fn set_trace_enabled(&mut self, enabled: bool) {
        self.trace_enabled = enabled;
    }

    pub fn try_convert(&mut self, input: StableFrame) -> Result<ConversionAdmission, String> {
        if !input.belongs_to(&self.gpu.backend) {
            return Err("Encoder input belongs to another GPU device".into());
        }
        if (input.image.color_space != FrameColorSpace::Unknown
            && input.image.color_space != self.gpu.interpretation.color_space())
            || (input.image.alpha_mode != FrameAlphaMode::Unknown
                && input.image.alpha_mode != self.gpu.interpretation.alpha())
        {
            return Err("Encoder input metadata disagrees with its declared interpretation".into());
        }
        let Some(slot) = self
            .slots
            .iter()
            .find(|slot| !slot.busy.load(Ordering::Acquire))
            .cloned()
        else {
            return Ok(ConversionAdmission::WouldBlock(input));
        };
        let mut surface = slot
            .surface
            .try_retain()
            .map_err(|error| error.to_string())?;
        slot.busy.store(true, Ordering::Release);
        surface
            .attach_lease(ExportLease {
                slot,
                budget: self.budget.clone(),
            })
            .map_err(|error| error.to_string())?;
        let gpu_lease = surface.try_retain().map_err(|error| error.to_string())?;
        let handle = input.output();
        let texture = TextureHandle {
            texture: handle.texture.clone(),
            view: handle.view.clone(),
            sampler: handle.sampler.clone(),
            lease: handle.lease.clone(),
            width: handle.width,
            height: handle.height,
            format: handle.format,
        };
        self.gpu.convert(&texture, &mut surface)?;
        let ready = ConversionReady(Arc::new(Completion {
            ready: AtomicBool::new(false),
            completed_at: OnceLock::new(),
        }));
        let completed = ready.clone();
        let trace = self.trace_enabled;
        let waker = self.waker.clone();
        self.gpu.backend.queue.on_submitted_work_done(move || {
            drop((input, gpu_lease));
            if trace {
                let _ = completed.0.completed_at.set(Instant::now());
            }
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

impl D3d12ColorConverter {
    pub fn new(
        backend: Arc<GpuBackend>,
        width: u32,
        height: u32,
        interpretation: InputInterpretation,
    ) -> Result<Self, String> {
        let max = backend.device.limits().max_texture_dimension_2d;
        if width == 0
            || height == 0
            || width > max
            || height > max
            || width % 2 != 0
            || height % 2 != 0
        {
            return Err(
                "Encoder plane dimensions must be positive, even and within device limits".into(),
            );
        }
        if interpretation.encoder_profile().is_hdr()
            && !backend.device.features().contains(
                wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                    | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
            )
        {
            return Err("Render device cannot render to 16-bit encoder planes".into());
        }
        let (_, layout, pipelines) = create_pipelines(&backend.device, interpretation);
        let planes = std::array::from_fn(|plane| {
            let divisor = if plane == 0 { 1 } else { 2 };
            backend.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("D3D12 RGB conversion plane"),
                size: wgpu::Extent3d {
                    width: width / divisor,
                    height: height / divisor,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: plane_formats(interpretation.encoder_profile())[plane],
                usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
        });
        let views = planes
            .each_ref()
            .map(|plane| plane.create_view(&Default::default()));
        // The two planes have equal row byte counts. The queue serializes
        // their copies and reuse of the same GPU-only staging buffer.
        let bytes_per_sample = if interpretation.encoder_profile().is_hdr() {
            2
        } else {
            1
        };
        let alignment = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let pitch = (width * bytes_per_sample).div_ceil(alignment) * alignment;
        let staging_len = u64::from(pitch) * u64::from(height);
        if staging_len > backend.device.limits().max_buffer_size {
            return Err("Encoder plane exceeds the device buffer limit".into());
        }
        let staging = backend.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU encoder plane copy"),
            size: staging_len,
            usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            backend,
            interpretation,
            width,
            height,
            planes,
            views,
            layout,
            pipelines,
            staging,
            pitch,
        })
    }

    /// Submit conversion of a GPU texture. Retain the input and output leases
    /// until the submission completes, then pass the output to the encoder.
    pub fn convert(
        &mut self,
        input: &TextureHandle,
        surface: &mut InputSurface,
    ) -> Result<wgpu::SubmissionIndex, String> {
        if input.width != self.width
            || input.height != self.height
            || input.format != self.interpretation.texture_format()
            || !input
                .texture
                .usage()
                .contains(wgpu::TextureUsages::TEXTURE_BINDING)
        {
            return Err(
                "RGB texture does not match the encoder interpretation or dimensions".into(),
            );
        }
        let bindings = self
            .backend
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("D3D12 encoder RGB"),
                layout: &self.layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&input.view),
                }],
            });
        let mut commands = self
            .backend
            .device
            .create_command_encoder(&Default::default());
        for (pipeline, view) in self.pipelines.iter().zip(&self.views) {
            let mut pass = commands.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bindings, &[]);
            pass.draw(0..3, 0..1);
        }
        self.backend.queue.submit([commands.finish()]);
        self.backend.copy_d3d12_encoder_planes(
            self.planes.each_ref(),
            surface,
            &self.staging,
            self.pitch,
        )
    }
}
impl GpuBackend {
    pub fn open_d3d12_encoder(
        &self,
        width: u32,
        height: u32,
        time_base: RationalTimeBase,
        bitrate: usize,
        profile: Profile,
    ) -> Result<D3d12Encoder, String> {
        let device = unsafe { self.device.as_hal::<wgpu::hal::api::Dx12>() }
            .ok_or("Video encoding requires the DX12 render device")?;
        let raw = NonNull::new(device.raw_device().as_raw()).ok_or("Missing DX12 render device")?;
        unsafe { D3d12Encoder::open_on_device(width, height, time_base, bitrate, profile, raw) }
            .map_err(|error| error.to_string())
    }

    /// Copies GPU-rendered luma and chroma into an encoder surface on the same
    /// device. The caller must wait for the returned submission before encoding.
    fn copy_d3d12_encoder_planes(
        &self,
        planes: [&wgpu::Texture; 2],
        surface: &mut InputSurface,
        staging: &wgpu::Buffer,
        pitch: u32,
    ) -> Result<wgpu::SubmissionIndex, String> {
        let pointer = surface.texture_ptr().as_ptr();
        let resource = unsafe { ID3D12Resource::from_raw_borrowed(&pointer) }
            .ok_or("Missing encoder texture")?;
        let mut owner: Option<ID3D12Device> = None;
        unsafe { resource.GetDevice(&mut owner) }.map_err(|error| error.to_string())?;
        let device = unsafe { self.device.as_hal::<wgpu::hal::api::Dx12>() }
            .ok_or("Video encoding requires DX12")?;
        if owner.as_ref() != Some(device.raw_device()) {
            return Err("Encoder texture belongs to a different D3D12 device".into());
        }
        drop(device);
        let info = unsafe { resource.GetDesc() };
        let (format, plane_formats, required) = match info.Format {
            DXGI_FORMAT_NV12 => (
                wgpu::TextureFormat::NV12,
                [wgpu::TextureFormat::R8Unorm, wgpu::TextureFormat::Rg8Unorm],
                wgpu::Features::TEXTURE_FORMAT_NV12,
            ),
            DXGI_FORMAT_P010 => (
                wgpu::TextureFormat::P010,
                [
                    wgpu::TextureFormat::R16Unorm,
                    wgpu::TextureFormat::Rg16Unorm,
                ],
                wgpu::Features::TEXTURE_FORMAT_P010 | wgpu::Features::TEXTURE_FORMAT_16BIT_NORM,
            ),
            _ => return Err("Encoder allocated an unsupported DXGI format".into()),
        };
        if !self.device.features().contains(required) {
            return Err(format!("Render device cannot copy to {format:?}"));
        }
        let size = wgpu::Extent3d {
            width: u32::try_from(info.Width).map_err(|_| "Invalid encoder width")?,
            height: info.Height,
            depth_or_array_layers: 1,
        };
        let chroma = wgpu::Extent3d {
            width: size.width / 2,
            height: size.height / 2,
            depth_or_array_layers: 1,
        };
        for ((texture, format), extent) in planes.iter().zip(plane_formats).zip([size, chroma]) {
            if texture.size() != extent
                || texture.format() != format
                || !texture.usage().contains(wgpu::TextureUsages::COPY_SRC)
            {
                return Err("GPU plane does not match the encoder surface".into());
            }
        }
        let texture = unsafe {
            let raw = wgpu::hal::dx12::Device::texture_from_raw(
                resource.clone(),
                format,
                wgpu::TextureDimension::D2,
                size,
                1,
                1,
            );
            self.device.create_texture_from_hal::<wgpu::hal::api::Dx12>(
                raw,
                &wgpu::TextureDescriptor {
                    label: Some("D3D12 encoder surface"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                },
                wgpu::TextureUses::PRESENT,
            )
        };
        let mut commands = self.device.create_command_encoder(&Default::default());
        for ((plane, extent), aspect) in planes
            .into_iter()
            .zip([size, chroma])
            .zip([wgpu::TextureAspect::Plane0, wgpu::TextureAspect::Plane1])
        {
            let transfer = wgpu::TexelCopyBufferInfo {
                buffer: staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(pitch),
                    rows_per_image: Some(extent.height),
                },
            };
            commands.copy_texture_to_buffer(plane.as_image_copy(), transfer, extent);
            commands.copy_buffer_to_texture(
                transfer,
                wgpu::TexelCopyTextureInfo {
                    aspect,
                    ..texture.as_image_copy()
                },
                extent,
            );
        }
        commands.transition_resources(
            std::iter::empty(),
            std::iter::once(wgpu::TextureTransition {
                texture: &texture,
                selector: None,
                state: wgpu::TextureUses::PRESENT,
            }),
        );
        Ok(self.queue.submit([commands.finish()]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires H.264 and HEVC Main 10 hardware encoding"]
    fn converts_gpu_rgb_and_encodes_sdr_pq_and_hlg() {
        pollster::block_on(async {
            let mut options = wgpu::InstanceDescriptor::new_without_display_handle();
            options.backends = wgpu::Backends::DX12;
            let instance = wgpu::Instance::new(options);
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .unwrap();
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    required_features: wgpu::Features::TEXTURE_FORMAT_NV12
                        | wgpu::Features::TEXTURE_FORMAT_P010
                        | wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                        | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
                    ..Default::default()
                })
                .await
                .unwrap();
            let backend = Arc::new(GpuBackend::from_device(device, queue));
            for interpretation in [
                InputInterpretation::SdrSrgbRgba8OpaqueBlack,
                InputInterpretation::Hdr10AcesCgRgba16Float,
                InputInterpretation::HlgAcesCgRgba16Float,
            ] {
                let profile = interpretation.encoder_profile();
                let mut encoder = backend
                    .open_d3d12_encoder(
                        320,
                        180,
                        RationalTimeBase {
                            numerator: 1,
                            denominator: 30,
                        },
                        1_000_000,
                        profile,
                    )
                    .unwrap();
                let input = backend.create_texture(320, 180, interpretation.texture_format());
                let mut commands = backend.device.create_command_encoder(&Default::default());
                commands.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &input.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                backend.queue.submit([commands.finish()]);
                let mut converter =
                    D3d12ColorConverter::new(backend.clone(), 320, 180, interpretation).unwrap();
                let mut output = encoder.allocate().unwrap();
                let submission = converter.convert(&input, &mut output).unwrap();
                backend
                    .device
                    .poll(wgpu::PollType::Wait {
                        submission_index: Some(submission),
                        timeout: None,
                    })
                    .unwrap();
                let mut packets = encoder.encode(output, 0, 1).unwrap();
                packets.extend(encoder.drain().unwrap());
                assert_eq!(packets.len(), 1);
                assert_eq!(packets[0].pts(), Some(0));
                check_pixels(encoder.parameters(), packets.remove(0), profile);
            }
        });
    }

    fn check_pixels(
        parameters: ffmpeg_next::codec::Parameters,
        mut packet: ffmpeg_next::Packet,
        profile: Profile,
    ) {
        use ffmpeg_next::{codec, format, frame::Video, Rational};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gpu-planes.mp4");
        let rate = Rational(1, 30);
        let mut output = format::output(&path).unwrap();
        let mut stream = output
            .add_stream(codec::encoder::find(codec::Id::None))
            .unwrap();
        stream.set_time_base(rate);
        stream.set_parameters(parameters);
        output.write_header().unwrap();
        packet.set_stream(0);
        packet.rescale_ts(rate, output.stream(0).unwrap().time_base());
        packet.write_interleaved(&mut output).unwrap();
        output.write_trailer().unwrap();
        drop(output);

        // Independent software decode of the file is only used as a test
        // reference. Production GPU planes never enter a host buffer.
        let mut file = format::input(&path).unwrap();
        let mut decoder =
            codec::context::Context::from_parameters(file.stream(0).unwrap().parameters())
                .unwrap()
                .decoder()
                .video()
                .unwrap();
        let (_, packet) = file.packets().next().unwrap();
        decoder.send_packet(&packet).unwrap();
        decoder.send_eof().unwrap();
        let mut frame = Video::empty();
        decoder.receive_frame(&mut frame).unwrap();
        let sample = |plane| {
            let data = frame.data(plane);
            let offset = frame.stride(plane) * 16;
            if profile == Profile::Sdr {
                i32::from(data[offset + 32])
            } else {
                i32::from(u16::from_le_bytes(
                    data[offset + 64..offset + 66].try_into().unwrap(),
                ))
            }
        };
        let expected = match profile {
            Profile::Sdr => [235, 128, 128],
            Profile::Hdr10 => [573, 512, 512],
            Profile::Hlg => [721, 512, 512],
        };
        for (plane, expected) in expected.into_iter().enumerate() {
            let actual = sample(plane);
            assert!(
                (actual - expected).abs() <= 3,
                "{profile:?} plane {plane}: expected {expected}, got {actual}"
            );
        }
    }
}
