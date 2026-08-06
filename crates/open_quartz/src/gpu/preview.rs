use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{
    GpuBackend, GpuOutputHandle, RenderTarget, TextureFormat, BLIT_FRAG, FULLSCREEN_VERT_WITH_UV,
};

pub struct GpuPreviewImage {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub scale_submit: Duration,
    pub readback: Duration,
}

/// Reusable GPU scaler and bounded readback for UI previews.
///
/// The source texture remains owned by the executor. Cloned wgpu handles let
/// preview work proceed without holding the native runtime mutex.
pub struct GpuPreviewReader {
    backend: Arc<GpuBackend>,
    bind_group_layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    target: Option<RenderTarget>,
}

impl GpuPreviewReader {
    pub fn new(backend: Arc<GpuBackend>) -> Self {
        let bind_group_layout =
            backend
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("open-quartz-preview-bindings"),
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
        let pipeline_layout =
            backend
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("open-quartz-preview-pipeline-layout"),
                    bind_group_layouts: &[&bind_group_layout],
                    push_constant_ranges: &[],
                });
        let vertex = backend
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("open-quartz-preview-vertex"),
                source: wgpu::ShaderSource::Wgsl(FULLSCREEN_VERT_WITH_UV.into()),
            });
        let fragment = backend
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("open-quartz-preview-fragment"),
                source: wgpu::ShaderSource::Wgsl(BLIT_FRAG.into()),
            });
        let pipeline = backend
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("open-quartz-preview-pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &vertex,
                    entry_point: Some("main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &fragment,
                    entry_point: Some("main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview: None,
                cache: None,
            });
        Self {
            backend,
            bind_group_layout,
            pipeline,
            target: None,
        }
    }

    pub fn scale(
        &mut self,
        source: &GpuOutputHandle,
        max_dimension: u32,
    ) -> Result<GpuOutputHandle, String> {
        if max_dimension == 0 {
            return Err("GPU preview max dimension must be greater than zero".to_owned());
        }
        if source.format != TextureFormat::Rgba8Unorm {
            return Err(format!(
                "GPU preview requires rgba8unorm, got {:?}",
                source.format
            ));
        }
        let largest = source.width.max(source.height);
        let scale_dimension = largest.min(max_dimension);
        let width = ((u64::from(source.width) * u64::from(scale_dimension)) / u64::from(largest))
            .max(1) as u32;
        let height = ((u64::from(source.height) * u64::from(scale_dimension)) / u64::from(largest))
            .max(1) as u32;
        let recreate = self.target.as_ref().is_none_or(|target| {
            target.width != width || target.height != height
        });
        if recreate {
            self.target = Some(self.backend.create_target(
                width,
                height,
                TextureFormat::Rgba8Unorm,
            ));
        }
        let target = self
            .target
            .as_ref()
            .ok_or_else(|| "GPU preview target allocation failed".to_owned())?;
        let bind_group = self
            .backend
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("open-quartz-preview-bind-group"),
                layout: &self.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&source.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&source.sampler),
                    },
                ],
            });
        let mut encoder =
            self.backend
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("open-quartz-preview-scale"),
                });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("open-quartz-preview-scale-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
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
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.backend.queue.submit([encoder.finish()]);
        Ok(GpuOutputHandle {
            texture: target.texture.clone(),
            view: target.view.clone(),
            sampler: source.sampler.clone(),
            width,
            height,
            format: TextureFormat::Rgba8Unorm,
        })
    }

    pub async fn read(
        &mut self,
        source: &GpuOutputHandle,
        max_dimension: u32,
    ) -> Result<GpuPreviewImage, String> {
        let scale_started = Instant::now();
        let scaled = self.scale(source, max_dimension)?;
        let scale_submit = scale_started.elapsed();
        let target = self
            .target
            .as_ref()
            .ok_or_else(|| "GPU preview target allocation failed".to_owned())?;
        let readback_started = Instant::now();
        let rgba = self.backend.read_target_rgba(target).await?;
        let readback = readback_started.elapsed();
        Ok(GpuPreviewImage {
            rgba,
            width: scaled.width,
            height: scaled.height,
            scale_submit,
            readback,
        })
    }
}
