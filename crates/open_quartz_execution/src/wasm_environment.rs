#![cfg(target_arch = "wasm32")]

use std::sync::Arc;

use crate::gpu::{
    GpuBackend, GpuExecutor, GpuOutputHandle, TextureFormat, BLIT_FRAG, FULLSCREEN_VERT_WITH_UV,
};
use open_quartz_schema::{SdkError, SdkErrorCode};

#[derive(Clone)]
pub struct BrowserFrame {
    bitmap: web_sys::ImageBitmap,
    timestamp_ns: u64,
}

impl BrowserFrame {
    pub fn new(bitmap: web_sys::ImageBitmap, timestamp_ns: u64) -> Self {
        Self {
            bitmap,
            timestamp_ns,
        }
    }

    pub fn timestamp_ns(&self) -> u64 {
        self.timestamp_ns
    }

    pub fn width(&self) -> u32 {
        self.bitmap.width()
    }

    pub fn height(&self) -> u32 {
        self.bitmap.height()
    }
}

/// Rust-owned WebGPU objects created inside a browser worker.
///
/// TypeScript supplies only the host canvas. Adapter/device/queue/surface
/// ownership stays in Rust so browser execution does not require a second
/// TypeScript GPU engine.
pub struct BrowserGpuEnvironment {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    pub surface: wgpu::Surface<'static>,
    pub surface_format: wgpu::TextureFormat,
    pub backend: Arc<GpuBackend>,
    pub executor: GpuExecutor,
    present_bind_group_layout: wgpu::BindGroupLayout,
    present_pipeline: wgpu::RenderPipeline,
}

impl BrowserGpuEnvironment {
    pub async fn from_offscreen_canvas(canvas: web_sys::OffscreenCanvas) -> Result<Self, SdkError> {
        let width = canvas.width().max(1);
        let height = canvas.height().max(1);
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::BROWSER_WEBGPU,
            ..Default::default()
        });
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::OffscreenCanvas(canvas))
            .map_err(|error| {
                SdkError::new(
                    SdkErrorCode::InvalidResource,
                    "Cannot create a WebGPU OffscreenCanvas surface",
                )
                .with_details(error.to_string())
            })?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await
            .map_err(|error| {
                SdkError::new(
                    SdkErrorCode::InvalidResource,
                    "Cannot find a browser WebGPU adapter",
                )
                .with_details(error.to_string())
            })?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .map_err(|error| {
                SdkError::new(
                    SdkErrorCode::InvalidResource,
                    "Cannot create a browser WebGPU device",
                )
                .with_details(error.to_string())
            })?;
        let capabilities = surface.get_capabilities(&adapter);
        let surface_format = capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .or_else(|| capabilities.formats.first().copied())
            .ok_or_else(|| {
                SdkError::new(
                    SdkErrorCode::InvalidResource,
                    "Browser WebGPU surface reports no supported format",
                )
            })?;
        surface.configure(
            &device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: surface_format,
                width,
                height,
                present_mode: wgpu::PresentMode::Fifo,
                alpha_mode: capabilities
                    .alpha_modes
                    .first()
                    .copied()
                    .unwrap_or(wgpu::CompositeAlphaMode::Opaque),
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            },
        );

        let device = Arc::new(device);
        let queue = Arc::new(queue);
        let backend = Arc::new(GpuBackend::from_shared(device.clone(), queue.clone()));
        let executor = GpuExecutor::new(backend.clone());
        let (present_bind_group_layout, present_pipeline) =
            create_present_pipeline(&device, surface_format);

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            surface,
            surface_format,
            backend,
            executor,
            present_bind_group_layout,
            present_pipeline,
        })
    }
}
impl BrowserGpuEnvironment {
    pub fn upload_frame(&mut self, node_id: &str, frame: &BrowserFrame) -> Result<(), SdkError> {
        let width = frame.width().max(1);
        let height = frame.height().max(1);
        let texture = self
            .backend
            .create_texture(width, height, TextureFormat::Rgba8Unorm);
        self.queue.copy_external_image_to_texture(
            &wgpu::CopyExternalImageSourceInfo {
                source: wgpu::ExternalImageSource::ImageBitmap(frame.bitmap.clone()),
                origin: wgpu::Origin2d::ZERO,
                flip_y: false,
            },
            wgpu::TexelCopyTextureInfo {
                texture: texture.texture.as_ref(),
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            }
            .to_tagged(wgpu::PredefinedColorSpace::Srgb, false),
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.executor
            .register_external_texture(
                node_id,
                GpuOutputHandle {
                    texture: texture.texture,
                    view: texture.view,
                    sampler: texture.sampler,
                    width,
                    height,
                    format: texture.format,
                },
            )
            .map_err(|error| {
                SdkError::new(SdkErrorCode::InvalidResource, error.to_string()).for_node(node_id)
            })
    }

    pub fn present(&self, output: &GpuOutputHandle) -> Result<(), SdkError> {
        let frame = self.surface.get_current_texture().map_err(|error| {
            SdkError::new(
                SdkErrorCode::InvalidResource,
                "Cannot acquire browser surface frame",
            )
            .with_details(error.to_string())
        })?;
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("open-quartz-browser-present-bind-group"),
            layout: &self.present_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&output.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&output.sampler),
                },
            ],
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("open-quartz-browser-present"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("open-quartz-browser-present-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.present_pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        frame.present();
        Ok(())
    }
}

fn create_present_pipeline(
    device: &wgpu::Device,
    surface_format: wgpu::TextureFormat,
) -> (wgpu::BindGroupLayout, wgpu::RenderPipeline) {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("open-quartz-browser-present-layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("open-quartz-browser-present-pipeline-layout"),
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });
    let vertex = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("open-quartz-browser-present-vertex"),
        source: wgpu::ShaderSource::Wgsl(FULLSCREEN_VERT_WITH_UV.into()),
    });
    let fragment = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("open-quartz-browser-present-fragment"),
        source: wgpu::ShaderSource::Wgsl(BLIT_FRAG.into()),
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("open-quartz-browser-present-pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &vertex,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &fragment,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    });
    (layout, pipeline)
}
