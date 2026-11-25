use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use parking_lot::Mutex;

use super::TextureFormat;
use crate::compiler::shader::CompiledShader;

/// Device-generation counters. Driver pipeline memory is not measurable; cache
/// capacity bounds retained entries, not a fictitious GPU byte estimate.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct GpuRuntimeStats {
    pub pipeline_creations: u64,
    pub pipeline_cache_hits: u64,
    pub pipeline_cache_entries: usize,
    /// Retained shader/key bytes on the CPU, not driver GPU memory.
    pub pipeline_cache_source_bytes: usize,
    pub target_allocations: u64,
    pub operator_draws: u64,
    pub draws: u64,
    pub uniform_writes: u64,
}

#[derive(Default)]
pub(crate) struct RuntimeCounters {
    pub target_allocations: AtomicU64,
    pub operator_draws: AtomicU64,
    pub draws: AtomicU64,
    pub uniform_writes: AtomicU64,
}

#[derive(Clone)]
pub(crate) struct SharedPipeline {
    pub layout: wgpu::BindGroupLayout,
    pub pipeline: wgpu::RenderPipeline,
    pub static_buffer: Option<(u32, wgpu::Buffer)>,
}

#[derive(PartialEq, Eq)]
struct PipelineKey {
    fragment: String,
    vertex: String,
    layout: Vec<(u32, String, String, Option<String>)>,
    format: TextureFormat,
    domain: crate::operators::ColorDomain,
}

impl PipelineKey {
    fn source_bytes(&self) -> usize {
        self.fragment.len() + self.vertex.len() + self.layout.iter()
            .map(|(_, kind, name, ty)| kind.len() + name.len() + ty.as_ref().map_or(0, String::len)).sum::<usize>()
    }
}

struct Entry {
    key: PipelineKey,
    value: SharedPipeline,
}

pub(crate) struct PipelineCache {
    entries: VecDeque<Entry>,
    capacity: usize,
    creations: u64,
    hits: u64,
}

impl Default for PipelineCache {
    fn default() -> Self {
        Self { entries: VecDeque::new(), capacity: 64, creations: 0, hits: 0 }
    }
}

impl PipelineCache {
    #[cfg(test)]
    pub fn set_test_capacity(&mut self, capacity: usize) {
        assert!(capacity > 0);
        self.capacity = capacity;
        while self.entries.len() > capacity { self.entries.pop_front(); }
    }
    /// Compilation is serialized by the GPU owner's cache lock. Failed
    /// preparation never inserts an entry; retained pipeline clones survive LRU.
    pub fn get_or_create(
        &mut self,
        shader: &CompiledShader,
        format: TextureFormat,
        domain: crate::operators::ColorDomain,
        create: impl FnOnce() -> Result<SharedPipeline, super::GpuExecutionError>,
    ) -> Result<SharedPipeline, super::GpuExecutionError> {
        let layout = shader.bindings.iter().map(|b|
            (b.binding, b.kind.clone(), b.name.clone(), b.wgsl_type.clone())).collect();
        let key = PipelineKey { fragment: shader.full_fragment_code.clone(), vertex: shader.vertex_shader.clone(), layout, format, domain };
        if let Some(index) = self.entries.iter().position(|entry| entry.key == key) {
            let entry = self.entries.remove(index).expect("cache index exists");
            let value = entry.value.clone();
            self.entries.push_back(entry);
            self.hits += 1;
            return Ok(value);
        }
        let value = create()?;
        self.creations += 1;
        const MAX_SOURCE_BYTES: usize = 8 * 1024 * 1024;
        let bytes = key.source_bytes();
        if bytes > MAX_SOURCE_BYTES { return Ok(value); }
        while self.entries.len() >= self.capacity
            || self.entries.iter().map(|entry| entry.key.source_bytes()).sum::<usize>() + bytes > MAX_SOURCE_BYTES {
            self.entries.pop_front();
        }
        self.entries.push_back(Entry { key, value: value.clone() });
        Ok(value)
    }

    pub fn stats(&self, counters: &RuntimeCounters) -> GpuRuntimeStats {
        GpuRuntimeStats {
            pipeline_creations: self.creations,
            pipeline_cache_hits: self.hits,
            pipeline_cache_entries: self.entries.len(),
            pipeline_cache_source_bytes: self.entries.iter().map(|entry| entry.key.source_bytes()).sum(),
            target_allocations: counters.target_allocations.load(Ordering::Relaxed),
            operator_draws: counters.operator_draws.load(Ordering::Relaxed),
            draws: counters.draws.load(Ordering::Relaxed),
            uniform_writes: counters.uniform_writes.load(Ordering::Relaxed),
        }
    }
}

pub(crate) type DevicePipelineCache = Mutex<PipelineCache>;

pub(crate) fn prepare_pipeline(backend: &super::GpuBackend, shader: &CompiledShader, format: TextureFormat, domain: crate::operators::ColorDomain) -> Result<SharedPipeline, super::GpuExecutionError> {
    backend.pipeline_cache.lock().get_or_create(shader, format, domain, || {
        let mut entries = shader.bindings.iter().map(super::executor::binding_layout_entry).collect::<Result<Vec<_>, _>>()?;
        let mut fragment_code = std::borrow::Cow::Borrowed(shader.full_fragment_code.as_str());
        let mut static_buffer = None;
        if cfg!(windows) {
            let binding = entries.iter().map(|entry| entry.binding).max().map_or(0, |last| last + 1);
            if let Some((code, bytes)) = crate::compiler::shader::embedded_cube_buffer(&fragment_code, binding) {
                use wgpu::util::DeviceExt;
                static_buffer = Some((binding, backend.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Embedded CUBE samples"), contents: &bytes, usage: wgpu::BufferUsages::STORAGE,
                })));
                entries.push(wgpu::BindGroupLayoutEntry {
                    binding, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false, min_binding_size: None,
                    },
                    count: None,
                });
                fragment_code = code.into();
            }
        }
        let module = naga::front::wgsl::parse_str(&fragment_code).map_err(|error| super::GpuExecutionError { node_id: None, message: error.emit_to_string(&fragment_code) })?;
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module)
            .map_err(|error| super::GpuExecutionError { node_id: None, message: error.to_string() })?;
        let layout = backend.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("open-quartz-shared-bindings"), entries: &entries,
        });
        let pipeline_layout = backend.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("open-quartz-shared-pipeline-layout"), bind_group_layouts: &[Some(&layout)], immediate_size: 0,
        });
        let vertex = backend.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("open-quartz-shared-vertex"), source: wgpu::ShaderSource::Wgsl(shader.vertex_shader.clone().into()),
        });
        let fragment = backend.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("open-quartz-shared-fragment"), source: wgpu::ShaderSource::Wgsl(fragment_code),
        });
        let pipeline = backend.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("open-quartz-shared-pipeline"), layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &vertex, entry_point: Some("main"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(), depth_stencil: None, multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &fragment, entry_point: Some("main"), compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: format.into(), blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
            multiview_mask: None, cache: None,
        });
        Ok(SharedPipeline { layout, pipeline, static_buffer })
    })
}
