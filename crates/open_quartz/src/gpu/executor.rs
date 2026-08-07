use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use crate::engine::{ExecutionCommand, ExecutionPlan, FrameResult, NodeExecutionPlan};
use crate::wgsl::compiler::BindingDescriptor;

use super::{GpuBackend, RenderTarget, TextureFormat, TextureHandle};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuExecutionError {
    pub node_id: Option<String>,
    pub message: String,
}

impl GpuExecutionError {
    fn for_node(node_id: &str, message: impl Into<String>) -> Self {
        Self {
            node_id: Some(node_id.to_owned()),
            message: message.into(),
        }
    }
}

impl fmt::Display for GpuExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.node_id {
            Some(node_id) => write!(formatter, "GPU node {node_id}: {}", self.message),
            None => formatter.write_str(&self.message),
        }
    }
}

impl std::error::Error for GpuExecutionError {}

pub struct GpuOutput<'a> {
    pub texture: &'a wgpu::Texture,
    pub view: &'a wgpu::TextureView,
    pub sampler: &'a wgpu::Sampler,
    pub width: u32,
    pub height: u32,
    pub format: TextureFormat,
}

#[derive(Clone)]
pub struct GpuOutputHandle {
    pub texture: Arc<wgpu::Texture>,
    pub view: Arc<wgpu::TextureView>,
    pub sampler: Arc<wgpu::Sampler>,
    pub width: u32,
    pub height: u32,
    pub format: TextureFormat,
}

struct NodeGpuResources {
    fragment_code: String,
    width: u32,
    height: u32,
    format: TextureFormat,
    bindings: Vec<BindingDescriptor>,
    bind_group_layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    uniform_buffers: HashMap<String, wgpu::Buffer>,
    targets: Vec<RenderTarget>,
    sampler: Arc<wgpu::Sampler>,
    output_index: usize,
}

impl NodeGpuResources {
    fn matches(&self, plan: &NodeExecutionPlan) -> bool {
        let Some(shader) = &plan.shader else {
            return false;
        };
        let Some(target) = &plan.target else {
            return false;
        };
        self.fragment_code == shader.full_fragment_code
            && self.width == target.width
            && self.height == target.height
            && self.format == target_format(target.float)
    }

    fn output(&self) -> &RenderTarget {
        &self.targets[self.output_index]
    }
}

/// Native `wgpu` executor for the shared Rust execution plan.
///
/// It owns only GPU resources and command encoding. Graph scheduling, dirty
/// propagation, uniforms, and feedback indices remain in `ExecutionEngine`.
pub struct GpuExecutor {
    backend: Arc<GpuBackend>,
    nodes: HashMap<String, NodeGpuResources>,
    renderer_sources: HashMap<String, String>,
    textures: HashMap<String, TextureHandle>,
}

impl GpuExecutor {
    pub fn new(backend: Arc<GpuBackend>) -> Self {
        Self {
            backend,
            nodes: HashMap::new(),
            renderer_sources: HashMap::new(),
            textures: HashMap::new(),
        }
    }

    pub fn backend(&self) -> &Arc<GpuBackend> {
        &self.backend
    }

    pub fn upload_rgba(
        &mut self,
        node_id: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Result<(), GpuExecutionError> {
        let recreate = self.textures.get(node_id).map_or(true, |texture| {
            texture.width != width
                || texture.height != height
                || texture.format != TextureFormat::Rgba8Unorm
        });
        if recreate {
            self.textures.insert(
                node_id.to_owned(),
                self.backend
                    .create_texture(width, height, TextureFormat::Rgba8Unorm),
            );
        }
        let texture = self.textures.get(node_id).ok_or_else(|| {
            GpuExecutionError::for_node(node_id, "image texture allocation failed")
        })?;
        self.backend
            .upload_rgba(texture, rgba)
            .map_err(|message| GpuExecutionError::for_node(node_id, message))
    }

    #[cfg(windows)]
    pub fn upload_d3d12_yuv(
        &mut self,
        node_id: &str,
        frame: &super::D3d12VideoFrame,
    ) -> Result<(), GpuExecutionError> {
        let recreate = self.textures.get(node_id).is_none_or(|texture| {
            texture.width != frame.width
                || texture.height != frame.height
                || texture.format != TextureFormat::Rgba8Unorm
        });
        if recreate {
            self.textures.insert(
                node_id.to_owned(),
                self.backend
                    .create_texture(frame.width, frame.height, TextureFormat::Rgba8Unorm),
            );
        }
        let texture = self.textures.get(node_id).ok_or_else(|| {
            GpuExecutionError::for_node(node_id, "video texture allocation failed")
        })?;
        self.backend
            .upload_d3d12_yuv(frame, texture)
            .map_err(|message| GpuExecutionError::for_node(node_id, message))
    }

    pub fn register_external_texture(
        &mut self,
        node_id: &str,
        texture: GpuOutputHandle,
    ) -> Result<(), GpuExecutionError> {
        if node_id.is_empty() || texture.width == 0 || texture.height == 0 {
            return Err(GpuExecutionError::for_node(
                node_id,
                "external texture requires a node ID and non-zero dimensions",
            ));
        }
        self.textures.insert(
            node_id.to_owned(),
            TextureHandle {
                texture: texture.texture,
                view: texture.view,
                sampler: texture.sampler,
                width: texture.width,
                height: texture.height,
                format: texture.format,
            },
        );
        Ok(())
    }

    pub fn remove_texture(&mut self, node_id: &str) {
        self.textures.remove(node_id);
        self.renderer_sources.retain(|_, source| source != node_id);
    }

    pub fn output_texture(&self, node_id: &str) -> Option<GpuOutput<'_>> {
        let resolved = self
            .renderer_sources
            .get(node_id)
            .map(String::as_str)
            .unwrap_or(node_id);
        if let Some(resources) = self.nodes.get(resolved) {
            let target = resources.output();
            return Some(GpuOutput {
                texture: target.texture.as_ref(),
                view: target.view.as_ref(),
                sampler: resources.sampler.as_ref(),
                width: target.width,
                height: target.height,
                format: target.format,
            });
        }
        self.textures.get(resolved).map(|texture| GpuOutput {
            texture: texture.texture.as_ref(),
            view: texture.view.as_ref(),
            sampler: texture.sampler.as_ref(),
            width: texture.width,
            height: texture.height,
            format: texture.format,
        })
    }

    pub fn output_handle(&self, node_id: &str) -> Option<GpuOutputHandle> {
        let resolved = self
            .renderer_sources
            .get(node_id)
            .map(String::as_str)
            .unwrap_or(node_id);
        if let Some(resources) = self.nodes.get(resolved) {
            let target = resources.output();
            return Some(GpuOutputHandle {
                texture: target.texture.clone(),
                view: target.view.clone(),
                sampler: resources.sampler.clone(),
                width: target.width,
                height: target.height,
                format: target.format,
            });
        }
        self.textures.get(resolved).map(|texture| GpuOutputHandle {
            texture: texture.texture.clone(),
            view: texture.view.clone(),
            sampler: texture.sampler.clone(),
            width: texture.width,
            height: texture.height,
            format: texture.format,
        })
    }

    pub async fn read_output_rgba(&self, node_id: &str) -> Result<Vec<u8>, GpuExecutionError> {
        let output = self.output_texture(node_id).ok_or_else(|| {
            GpuExecutionError::for_node(node_id, "output has no native GPU texture")
        })?;
        self.backend
            .read_texture_rgba(output.texture, output.width, output.height)
            .await
            .map_err(|message| GpuExecutionError::for_node(node_id, message))
    }

    /// Synchronize pipelines and targets with a new plan while preserving
    /// resources whose shader and target descriptor did not change.
    pub fn sync_plan(&mut self, plan: &ExecutionPlan) -> Result<(), GpuExecutionError> {
        let shader_ids = plan
            .nodes
            .iter()
            .filter(|node| node.shader.is_some())
            .map(|node| node.id.as_str())
            .collect::<HashSet<_>>();
        self.nodes
            .retain(|node_id, _| shader_ids.contains(node_id.as_str()));
        self.renderer_sources
            .retain(|node_id, _| plan.nodes.iter().any(|node| node.id == *node_id));

        for node in plan.nodes.iter().filter(|node| node.shader.is_some()) {
            if !node.validation_errors.is_empty() {
                return Err(GpuExecutionError::for_node(
                    &node.id,
                    node.validation_errors[0].message.clone(),
                ));
            }
            if self
                .nodes
                .get(&node.id)
                .is_some_and(|resources| resources.matches(node))
            {
                continue;
            }
            let resources = self.create_node_resources(node)?;
            self.nodes.insert(node.id.clone(), resources);
        }
        Ok(())
    }

    pub fn execute(
        &mut self,
        plan: &ExecutionPlan,
        frame: &FrameResult,
    ) -> Result<(), GpuExecutionError> {
        self.execute_commands(plan, &frame.commands)
    }

    pub fn execute_commands(
        &mut self,
        plan: &ExecutionPlan,
        commands: &[ExecutionCommand],
    ) -> Result<(), GpuExecutionError> {
        self.sync_plan(plan)?;
        let mut encoder =
            self.backend
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("open-quartz-frame"),
                });
        let mut encoded_gpu_work = false;

        for command in commands {
            match command.kind.as_str() {
                "shader" | "constant" => {
                    self.encode_shader(&mut encoder, command)?;
                    encoded_gpu_work = true;
                }
                "renderer" => {
                    let source = command.texture_inputs.values().next().ok_or_else(|| {
                        GpuExecutionError::for_node(
                            &command.node_id,
                            "renderer has no connected texture input",
                        )
                    })?;
                    self.output_texture(source).ok_or_else(|| {
                        GpuExecutionError::for_node(
                            &command.node_id,
                            format!("renderer source {source} has no GPU output"),
                        )
                    })?;
                    self.renderer_sources
                        .insert(command.node_id.clone(), source.clone());
                }
                "math" => {}
                "onnx" => {
                    return Err(GpuExecutionError::for_node(
                        &command.node_id,
                        "ONNX GPU execution belongs to the inference adapter and is not connected yet",
                    ));
                }
                other => {
                    return Err(GpuExecutionError::for_node(
                        &command.node_id,
                        format!("unsupported execution command {other}"),
                    ));
                }
            }
        }

        if encoded_gpu_work {
            self.backend.queue.submit([encoder.finish()]);
        }
        Ok(())
    }

    pub fn output_target(&self, node_id: &str) -> Option<&RenderTarget> {
        let resolved = self
            .renderer_sources
            .get(node_id)
            .map(String::as_str)
            .unwrap_or(node_id);
        self.nodes.get(resolved).map(NodeGpuResources::output)
    }

    fn sampled_texture(
        &self,
        node_id: &str,
    ) -> Option<(Arc<wgpu::TextureView>, Arc<wgpu::Sampler>)> {
        let resolved = self
            .renderer_sources
            .get(node_id)
            .map(String::as_str)
            .unwrap_or(node_id);
        if let Some(resources) = self.nodes.get(resolved) {
            return Some((resources.output().view.clone(), resources.sampler.clone()));
        }
        self.textures
            .get(resolved)
            .map(|texture| (texture.view.clone(), texture.sampler.clone()))
    }

    fn create_node_resources(
        &self,
        plan: &NodeExecutionPlan,
    ) -> Result<NodeGpuResources, GpuExecutionError> {
        let shader = plan.shader.as_ref().expect("filtered shader plan");
        let target = plan.target.as_ref().ok_or_else(|| {
            GpuExecutionError::for_node(&plan.id, "shader execution plan has no target")
        })?;
        if !shader.external_texture_bindings.is_empty() {
            return Err(GpuExecutionError::for_node(
                &plan.id,
                "native external video textures are not available before the media adapter stage",
            ));
        }
        let format = target_format(target.float);
        if format == TextureFormat::Rgba32Float
            && !self
                .backend
                .supported_features()
                .contains(wgpu::Features::FLOAT32_FILTERABLE)
        {
            return Err(GpuExecutionError::for_node(
                &plan.id,
                "rgba32float sampling requires FLOAT32_FILTERABLE on this adapter",
            ));
        }

        let layout_entries = shader
            .bindings
            .iter()
            .map(binding_layout_entry)
            .collect::<Result<Vec<_>, _>>()?;
        let bind_group_layout =
            self.backend
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("open-quartz-node-bindings"),
                    entries: &layout_entries,
                });
        let pipeline_layout =
            self.backend
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("open-quartz-node-pipeline-layout"),
                    bind_group_layouts: &[&bind_group_layout],
                    push_constant_ranges: &[],
                });
        let vertex_module =
            self.backend
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("open-quartz-fullscreen-vertex"),
                    source: wgpu::ShaderSource::Wgsl(shader.vertex_shader.clone().into()),
                });
        let fragment_module =
            self.backend
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("open-quartz-node-fragment"),
                    source: wgpu::ShaderSource::Wgsl(shader.full_fragment_code.clone().into()),
                });
        let pipeline =
            self.backend
                .device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("open-quartz-node-pipeline"),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &vertex_module,
                        entry_point: Some("main"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &fragment_module,
                        entry_point: Some("main"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: format.into(),
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview: None,
                    cache: None,
                });

        let uniform_buffers = shader
            .bindings
            .iter()
            .filter(|binding| binding.kind == "uniform")
            .map(|binding| {
                let size = uniform_size(binding.wgsl_type.as_deref().unwrap_or("f32"));
                let buffer = self.backend.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("open-quartz-uniform"),
                    size,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                (binding.name.clone(), buffer)
            })
            .collect();
        let target_count = if plan.feedback { 2 } else { 1 };
        let targets = (0..target_count)
            .map(|_| {
                self.backend
                    .create_target(target.width, target.height, format)
            })
            .collect();
        let sampler = Arc::new(
            self.backend
                .device
                .create_sampler(&wgpu::SamplerDescriptor {
                    label: Some("open-quartz-node-sampler"),
                    address_mode_u: wgpu::AddressMode::ClampToEdge,
                    address_mode_v: wgpu::AddressMode::ClampToEdge,
                    address_mode_w: wgpu::AddressMode::ClampToEdge,
                    mag_filter: wgpu::FilterMode::Linear,
                    min_filter: wgpu::FilterMode::Linear,
                    mipmap_filter: wgpu::FilterMode::Nearest,
                    ..Default::default()
                }),
        );

        Ok(NodeGpuResources {
            fragment_code: shader.full_fragment_code.clone(),
            width: target.width,
            height: target.height,
            format,
            bindings: shader.bindings.clone(),
            bind_group_layout,
            pipeline,
            uniform_buffers,
            targets,
            sampler,
            output_index: 0,
        })
    }

    fn encode_shader(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        command: &ExecutionCommand,
    ) -> Result<(), GpuExecutionError> {
        let mut sampled = BTreeMap::<String, (Arc<wgpu::TextureView>, Arc<wgpu::Sampler>)>::new();
        for (name, source_id) in &command.texture_inputs {
            let (view, sampler) = self.sampled_texture(source_id).ok_or_else(|| {
                GpuExecutionError::for_node(
                    &command.node_id,
                    format!("texture input {name} references unavailable source {source_id}"),
                )
            })?;
            sampled.insert(name.clone(), (view, sampler));
        }

        let resources = self.nodes.get_mut(&command.node_id).ok_or_else(|| {
            GpuExecutionError::for_node(&command.node_id, "shader GPU resources are missing")
        })?;
        let read_index = command.feedback_read_index.map(usize::from);
        let write_index = command.feedback_write_index.map(usize::from).unwrap_or(0);
        if let Some(index) = read_index {
            let target = resources.targets.get(index).ok_or_else(|| {
                GpuExecutionError::for_node(&command.node_id, "invalid feedback read index")
            })?;
            sampled.insert(
                "previousFrame".to_owned(),
                (target.view.clone(), resources.sampler.clone()),
            );
            if command.clear_feedback {
                clear_target(encoder, target);
            }
        }

        for (name, values) in &command.uniforms {
            let Some(buffer) = resources.uniform_buffers.get(name) else {
                continue;
            };
            let wgsl_type = resources
                .bindings
                .iter()
                .find(|binding| binding.name == *name)
                .and_then(|binding| binding.wgsl_type.as_deref())
                .unwrap_or("f32");
            let bytes = pack_uniform(wgsl_type, values);
            self.backend.queue.write_buffer(buffer, 0, &bytes);
        }

        let entries = resources
            .bindings
            .iter()
            .map(|binding| bind_group_entry(binding, resources, &sampled))
            .collect::<Result<Vec<_>, _>>()?;
        let bind_group = self
            .backend
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("open-quartz-node-bind-group"),
                layout: &resources.bind_group_layout,
                entries: &entries,
            });
        let target = resources.targets.get(write_index).ok_or_else(|| {
            GpuExecutionError::for_node(&command.node_id, "invalid feedback write index")
        })?;
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("open-quartz-node-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&resources.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        resources.output_index = write_index;
        Ok(())
    }
}

fn target_format(float: bool) -> TextureFormat {
    if float {
        TextureFormat::Rgba32Float
    } else {
        TextureFormat::Rgba8Unorm
    }
}

fn binding_layout_entry(
    binding: &BindingDescriptor,
) -> Result<wgpu::BindGroupLayoutEntry, GpuExecutionError> {
    let ty = match binding.kind.as_str() {
        "uniform" => wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        "texture" => wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        "sampler" => wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        "externalTexture" => {
            return Err(GpuExecutionError {
                node_id: None,
                message: "native external textures require the media adapter".to_owned(),
            });
        }
        other => {
            return Err(GpuExecutionError {
                node_id: None,
                message: format!("unsupported shader binding kind {other}"),
            });
        }
    };
    Ok(wgpu::BindGroupLayoutEntry {
        binding: binding.binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty,
        count: None,
    })
}

fn bind_group_entry<'a>(
    binding: &BindingDescriptor,
    resources: &'a NodeGpuResources,
    sampled: &'a BTreeMap<String, (Arc<wgpu::TextureView>, Arc<wgpu::Sampler>)>,
) -> Result<wgpu::BindGroupEntry<'a>, GpuExecutionError> {
    let resource = match binding.kind.as_str() {
        "uniform" => resources
            .uniform_buffers
            .get(&binding.name)
            .map(wgpu::Buffer::as_entire_binding)
            .ok_or_else(|| GpuExecutionError {
                node_id: None,
                message: format!("uniform buffer {} is missing", binding.name),
            })?,
        "texture" => sampled
            .get(&binding.name)
            .map(|(view, _)| wgpu::BindingResource::TextureView(view))
            .ok_or_else(|| GpuExecutionError {
                node_id: None,
                message: format!("texture binding {} is missing", binding.name),
            })?,
        "sampler" => {
            let texture_name = binding
                .name
                .strip_suffix("Sampler")
                .unwrap_or(&binding.name);
            sampled
                .get(texture_name)
                .map(|(_, sampler)| wgpu::BindingResource::Sampler(sampler))
                .ok_or_else(|| GpuExecutionError {
                    node_id: None,
                    message: format!("sampler binding {} is missing", binding.name),
                })?
        }
        other => {
            return Err(GpuExecutionError {
                node_id: None,
                message: format!("unsupported bind group resource {other}"),
            });
        }
    };
    Ok(wgpu::BindGroupEntry {
        binding: binding.binding,
        resource,
    })
}

fn clear_target(encoder: &mut wgpu::CommandEncoder, target: &RenderTarget) {
    let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("open-quartz-feedback-clear"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &target.view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    });
}

fn uniform_size(wgsl_type: &str) -> u64 {
    match wgsl_type {
        "vec2f" | "vec2i" | "vec2u" => 8,
        "vec3f" | "vec3i" | "vec3u" | "vec4f" | "vec4i" | "vec4u" => 16,
        "mat2x2f" => 16,
        "mat3x3f" => 48,
        "mat4x4f" => 64,
        _ => 4,
    }
}

fn pack_uniform(wgsl_type: &str, values: &[f32]) -> Vec<u8> {
    let size = uniform_size(wgsl_type) as usize;
    let mut bytes = vec![0; size];
    let integer = wgsl_type.ends_with('i');
    let unsigned = wgsl_type.ends_with('u') || wgsl_type == "u32";
    if wgsl_type == "mat3x3f" {
        for column in 0..3 {
            for row in 0..3 {
                let value = values.get(column * 3 + row).copied().unwrap_or_default();
                bytes[column * 16 + row * 4..column * 16 + row * 4 + 4]
                    .copy_from_slice(&value.to_le_bytes());
            }
        }
        return bytes;
    }
    for (index, value) in values.iter().copied().take(size / 4).enumerate() {
        let encoded = if integer {
            (value as i32).to_le_bytes()
        } else if unsigned {
            (value as u32).to_le_bytes()
        } else {
            value.to_le_bytes()
        };
        bytes[index * 4..index * 4 + 4].copy_from_slice(&encoded);
    }
    bytes
}
