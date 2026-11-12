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
    pub(crate) fn persistent_buffer_bytes(&self)->u64 {
        self.sort_steps.iter().map(wgpu::Buffer::size).sum()
    }
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("catalog resident detector layout"),
            entries: &[
                layout_entry(
                    0,
                    wgpu::BufferBindingType::Storage { read_only: true },
                    RAW_BYTES,
                ),
                layout_entry(1, wgpu::BufferBindingType::Uniform, 48),
                layout_entry(
                    2,
                    wgpu::BufferBindingType::Storage { read_only: false },
                    DETECTION_BYTES,
                ),
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
                    let mut mapped = uniform
                        .slice(..)
                        .get_mapped_range_mut()
                        .expect("newly created mapped sort uniform");
                    for (index, word) in [sequence, stride, 0, 0].into_iter().enumerate() {
                        mapped
                            .slice(index * 4..index * 4 + 4)
                            .copy_from_slice(&word.to_le_bytes());
                    }
                }
                uniform.unmap();
                sort_steps.push(uniform);
                stride /= 2;
            }
            sequence *= 2;
        }
        Self {
            layout,
            decode,
            sort,
            nms,
            sort_steps: sort_steps.into_boxed_slice(),
        }
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
        let bindings = self
            .sort_steps
            .iter()
            .map(|step| {
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
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
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

fn layout_entry(
    binding: u32,
    ty: wgpu::BufferBindingType,
    size: u64,
) -> wgpu::BindGroupLayoutEntry {
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
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn metal_detector_decodes_nonempty_boxes_and_suppresses_overlap() {
        pollster::block_on(async {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::METAL,
                ..wgpu::InstanceDescriptor::new_without_display_handle()
            });
            let adapter = instance
                .request_adapter(&Default::default())
                .await
                .expect("hardware Metal adapter required");
            assert_eq!(adapter.get_info().backend, wgpu::Backend::Metal);
            assert_ne!(adapter.get_info().device_type, wgpu::DeviceType::Cpu);
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor::default())
                .await
                .expect("Metal device");
            let raw = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("detector overlapping raw ONNX boxes"),
                size: RAW_BYTES,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: true,
            });
            {
                let mut mapped = raw.slice(..).get_mapped_range_mut().unwrap();
                mapped.slice(..).fill(0);
                for index in 0..2 {
                    for (plane, value) in [(0,320.0f32),(1,320.0),(2,128.0),(3,96.0),
                        (if index == 0 {7} else {5},if index == 0 {0.9} else {0.8})] {
                        let offset=(plane * RAW_BOX_COUNT as usize + index)*4;
                        mapped.slice(offset..offset+4).copy_from_slice(&value.to_ne_bytes());
                    }
                }
            }
            raw.unmap();
            let parameters = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("detector source geometry and thresholds"),
                size: 48,
                usage: wgpu::BufferUsages::UNIFORM,
                mapped_at_creation: true,
            });
            {
                let mut mapped = parameters.slice(..).get_mapped_range_mut().unwrap();
                for (index, value) in [640u32, 480, 640, 640, 640, 480, 8400, 1]
                    .into_iter()
                    .enumerate()
                {
                    mapped
                        .slice(index * 4..index * 4 + 4)
                        .copy_from_slice(&value.to_ne_bytes());
                }
                for (index, value) in [0.25f32, 0.45, 4.0, 0.0].into_iter().enumerate() {
                    mapped
                        .slice(32 + index * 4..36 + index * 4)
                        .copy_from_slice(&value.to_ne_bytes());
                }
            }
            parameters.unmap();
            let detector = Detector::new(&device);
            assert_eq!(detector.sort_steps.len(), SORT_STEP_COUNT);
            let slot = detector.prepare_slot(&device, &raw, &parameters);
            let staging = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("detector result readback"),
                size: 48,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            detector.encode(&mut encoder, &slot);
            encoder.copy_buffer_to_buffer(&slot.buffer, 0, &staging, 0, 48);
            let submission = queue.submit([encoder.finish()]);
            let (sender, receiver) = std::sync::mpsc::channel();
            staging.map_async(wgpu::MapMode::Read, .., move |result| {
                let _ = sender.send(result);
            });
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submission),
                    timeout: Some(Duration::from_secs(120)),
                })
                .expect("bounded detector readback wait");
            receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("map callback after GPU wait")
                .expect("detector result mapping");
            let mapped = staging.get_mapped_range(..).unwrap();
            let words: Vec<u32> = mapped
                .chunks_exact(4)
                .map(|word| u32::from_ne_bytes(word.try_into().unwrap()))
                .collect();
            assert_eq!(&words[..4], [1,0,0,0]);
            for (actual,expected) in words[4..8].iter().map(|word|f32::from_bits(*word)).zip([0.4,0.4,0.6,0.6]) {
                assert!((actual-expected).abs()<1e-6);
            }
            assert_eq!(f32::from_bits(words[8]),0.9);
            assert_eq!(&words[9..12],[3,0,0]);
            drop(mapped);
            staging.unmap();
        });
    }
}
