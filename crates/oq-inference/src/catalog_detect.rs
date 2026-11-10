//! Resident YOLOv8 decoding, stable sorting and exact class-agnostic greedy NMS.
//!
//! The native FP32 output remains on the GPU. A slot owns one reusable detection
//! buffer and caches every binding before frame submission; encoding only records
//! dispatches. Keep the slot alive through submission of all buffer consumers.

use std::sync::Arc;

const CANDIDATE_COUNT: u32 = 16_384;
const RAW_BOX_COUNT: u32 = 8_400;
const RAW_BYTES: u64 = 84 * RAW_BOX_COUNT as u64 * 4;
const DETECTION_BYTES: u64 = 16 + CANDIDATE_COUNT as u64 * 32;
const WORKGROUP_SIZE: u32 = 64;
const SORT_STEP_COUNT: usize = 105;

pub(crate) struct Detector {
    layout: wgpu::BindGroupLayout,
    decode: wgpu::ComputePipeline,
    sort: wgpu::ComputePipeline,
    nms: wgpu::ComputePipeline,
    sort_steps: Box<[wgpu::Buffer]>,
}

/// GPU layout: 16-byte `vec4u` header `(count, 0, 0, 0)`, followed by
/// 16,384 records of 32 bytes: `bbox: vec4f`, `score: f32`, `class_id: u32`,
/// `index: u32`, `reserved: u32`. The first `count` records are the kept boxes,
/// ordered by descending score, then ascending original YOLO output index.
/// Coordinates are normalized, clamped source-image xyxy coordinates.
pub(crate) struct DetectionSlot {
    pub buffer: Arc<wgpu::Buffer>,
    bindings: Box<[wgpu::BindGroup]>,
}

impl Detector {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("catalog resident detector layout"),
            entries: &[
                layout_entry(0, wgpu::BufferBindingType::Storage { read_only: true }, RAW_BYTES),
                layout_entry(1, wgpu::BufferBindingType::Uniform, 48),
                layout_entry(2, wgpu::BufferBindingType::Storage { read_only: false }, DETECTION_BYTES),
                layout_entry(3, wgpu::BufferBindingType::Uniform, 16),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("catalog resident detector pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("catalog resident YOLOv8 postprocessing"),
            source: wgpu::ShaderSource::Wgsl(include_str!("gpu/catalog_detect.wgsl").into()),
        });
        let pipeline = |entry_point| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry_point),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some(entry_point),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };
        let decode = pipeline("decode");
        let sort = pipeline("sort");
        let nms = pipeline("nms");

        // Each dispatch binds immutable step data. Rewriting one uniform while
        // encoding would otherwise make all dispatches observe its final value.
        // The same step buffers are shared by every slot on this detector.
        let mut sort_steps = Vec::with_capacity(SORT_STEP_COUNT);
        let mut sequence = 2;
        while sequence <= CANDIDATE_COUNT {
            let mut stride = sequence / 2;
            while stride != 0 {
                let uniform = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("catalog immutable bitonic step"),
                    size: 16,
                    usage: wgpu::BufferUsages::UNIFORM,
                    mapped_at_creation: true,
                });
                {
                    let mut mapped = uniform.slice(..).get_mapped_range_mut().expect("newly created mapped sort uniform");
                    for (index, word) in [sequence, stride, 0, 0].into_iter().enumerate() {
                        mapped.slice(index * 4..index * 4 + 4).copy_from_slice(&word.to_le_bytes());
                    }
                }
                uniform.unmap();
                sort_steps.push(uniform);
                stride /= 2;
            }
            sequence *= 2;
        }
        Self { layout, decode, sort, nms, sort_steps: sort_steps.into_boxed_slice() }
    }

    /// `raw` is FLOAT [1, 84, 8400]; `parameters` is the shared 48-byte catalog
    /// uniform (`geometry: vec4u`, `result: vec4u`, `controls: vec4f`).
    pub(crate) fn prepare_slot(
        &self,
        device: &wgpu::Device,
        raw: &wgpu::Buffer,
        parameters: &wgpu::Buffer,
    ) -> DetectionSlot {
        let buffer = Arc::new(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("catalog resident sorted detections"),
            size: DETECTION_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }));
        let bindings = self.sort_steps.iter().map(|step| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("catalog cached detector bindings"),
                layout: &self.layout,
                entries: &[
                    buffer_entry(0, raw),
                    buffer_entry(1, parameters),
                    buffer_entry(2, &buffer),
                    buffer_entry(3, step),
                ],
            })
        }).collect::<Vec<_>>().into_boxed_slice();
        DetectionSlot { buffer, bindings }
    }

    /// Records 107 dispatches: decode, 105 bitonic steps, then exact greedy NMS.
    /// Separate passes make each global read/write dependency explicit.
    pub(crate) fn encode(&self, encoder: &mut wgpu::CommandEncoder, slot: &DetectionSlot) {
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("catalog YOLOv8 decode"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.decode);
            pass.set_bind_group(0, &slot.bindings[0], &[]);
            pass.dispatch_workgroups(CANDIDATE_COUNT / WORKGROUP_SIZE, 1, 1);
        }
        for binding in &slot.bindings {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("catalog stable bitonic detection sort"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.sort);
            pass.set_bind_group(0, binding, &[]);
            pass.dispatch_workgroups((CANDIDATE_COUNT / 2) / WORKGROUP_SIZE, 1, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("catalog exact greedy detection NMS"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.nms);
            pass.set_bind_group(0, &slot.bindings[0], &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
    }
}

fn layout_entry(binding: u32, ty: wgpu::BufferBindingType, size: u64) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(size),
        },
        count: None,
    }
}

fn buffer_entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource: buffer.as_entire_binding() }
}
