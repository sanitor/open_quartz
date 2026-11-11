//! Hardware-only resident numeric, ownership and bounded-output regression.
//! All source pixels are produced by compute; readback is an explicit test consumer.
use super::*;
use crate::artifact::ArtifactInterface;

// Full production geometry, with one center-tap path through every convolution.
// The nine final channels repeat luma, so pixelshuffle must preserve a solid RGB
// source (within one byte for the documented BT.601 coefficient approximation).
fn identity_model() -> GpuModel {
    let layers = [(1, 64, 5, true), (64, 64, 3, true), (64, 32, 3, true), (32, 9, 3, false)]
        .into_iter()
        .enumerate()
        .map(|(index, (input, output, kernel, relu))| {
            let mut weights = vec![0.0; (input * output * kernel * kernel) as usize];
            for channel in 0..if index == 3 { output } else { 1 } {
                let center = (channel * input * kernel * kernel + (kernel * kernel) / 2) as usize;
                weights[center] = 1.0;
            }
            ConvLayer {
                weights, bias: vec![0.0; output as usize], in_channels: input,
                out_channels: output, kernel: [kernel; 2], stride: [1; 2],
                pads: [kernel / 2; 4], relu,
            }
        }).collect();
    GpuModel {
        plan: ModelPlan {
            input_width: 224, input_height: 224, output_width: 672, output_height: 672,
            scale: 3, input_name: "luma".into(), output_name: "upscaled_luma".into(),
            // This is an in-memory execution fixture, not a serialized artifact.
            source_sha256: String::new(), source_byte_length: 0,
            source_interface: ArtifactInterface {
                ir_version: None, producer_name: None, producer_version: None, domain: None,
                model_version: None, doc_string: None, metadata: Default::default(),
                opset_imports: vec![], graph_name: None, graph_doc_string: None,
                graph_metadata: Default::default(), inputs: vec![], outputs: vec![], initializers: vec![],
            },
        },
        layers,
    }
}

const REFERENCE_SIDE: usize = 224;
const REFERENCE_PIXELS: usize = REFERENCE_SIDE * REFERENCE_SIDE;
const REFERENCE_CHANNELS: [usize; 5] = [1, 64, 64, 32, 9];

fn dense_input_factor(layer: usize, channel: usize) -> f32 {
    if layer == 0 { return 1.0; }
    let factor = if (channel + layer) % 5 == 0 { -0.2 } else { 1.0 + (channel % 7) as f32 * 0.1 };
    factor / REFERENCE_CHANNELS[layer] as f32
}

fn dense_output_factor(layer: usize, channel: usize) -> f32 {
    if layer == 3 { return 0.7 + channel as f32 * 0.06; }
    let magnitude = 0.75 + channel as f32 * 0.65 / REFERENCE_CHANNELS[layer + 1] as f32;
    if channel % 4 == 0 { -magnitude } else { magnitude }
}

fn dense_bias(layer: usize, channel: usize) -> f32 {
    if layer == 3 { return 0.1 + channel as f32 * 0.04; }
    if channel % 4 == 0 {
        0.28 + channel as f32 * 0.2 / REFERENCE_CHANNELS[layer + 1] as f32
    } else {
        -0.02 + (channel % 7) as f32 * 0.01
    }
}

fn dense_spatial_kernel(layer: usize) -> &'static [f32] {
    // Asymmetric, dense, signed taps distinguish cross-correlation from a
    // flipped kernel and exercise every padding edge, not just the center tap.
    const FIVE: [f32; 25] = [
        0.01, -0.02, 0.03, 0.015, -0.01,
        0.025, 0.04, -0.03, 0.035, 0.02,
        -0.015, 0.06, 0.58, 0.08, -0.025,
        0.02, -0.01, 0.035, 0.015, 0.025,
        -0.005, 0.02, 0.03, -0.015, 0.01,
    ];
    const THREE: [[f32; 9]; 3] = [
        [0.035, -0.045, 0.075, 0.12, 0.70, -0.06, -0.025, 0.09, 0.11],
        [-0.025, 0.09, 0.11, -0.06, 0.70, 0.12, 0.075, -0.045, 0.035],
        [0.11, 0.12, -0.045, 0.09, 0.70, 0.035, -0.025, -0.06, 0.075],
    ];
    if layer == 0 { &FIVE } else { &THREE[layer - 1] }
}

fn dense_model() -> GpuModel {
    let mut model = identity_model();
    for (index, layer) in model.layers.iter_mut().enumerate() {
        let taps = dense_spatial_kernel(index);
        for output in 0..layer.out_channels as usize {
            layer.bias[output] = dense_bias(index, output);
            for input in 0..layer.in_channels as usize {
                for (tap, spatial) in taps.iter().enumerate() {
                    let weight = (output * layer.in_channels as usize + input) * taps.len() + tap;
                    layer.weights[weight] = dense_output_factor(index, output)
                        * dense_input_factor(index, input) * spatial;
                }
            }
        }
        assert!(layer.weights.iter().all(|weight| *weight != 0.0));
        assert!(layer.weights.iter().any(|weight| *weight < 0.0));
    }
    model
}

fn patterned_rgb(x: usize, y: usize) -> [f64; 3] {
    let mut seed = (x as u32).wrapping_mul(1_664_525)
        .wrapping_add((y as u32).wrapping_mul(1_013_904_223))
        .wrapping_add(0x9e37_79b9);
    seed ^= seed >> 16;
    seed = seed.wrapping_mul(2_246_822_519);
    seed ^= seed >> 13;
    [64 + (seed & 127), 64 + ((seed >> 8) & 127), 64 + ((seed >> 16) & 127)]
        .map(|byte| f64::from(byte) / 255.0)
}

fn dense_reference() -> Vec<u8> {
    // Independent FP64 scalar reference: channel-separable weights allow the
    // channel contraction BEFORE the spatial cross-correlation. This computes
    // the complete 224x224 network without a debug-mode dense ~3G-MAC loop or
    // reusing the GPU's OIHW indexing, tiling, accumulation order, or buffers.
    let mut contracted: Vec<f64> = (0..REFERENCE_PIXELS).map(|pixel| {
        let [r, g, b] = patterned_rgb(pixel % REFERENCE_SIDE, pixel / REFERENCE_SIDE);
        0.299 * r + 0.587 * g + 0.114 * b
    }).collect();
    let mut final_planes = Vec::new();
    let mut clipped = 0usize;
    for layer in 0..4 {
        let kernel = if layer == 0 { 5 } else { 3 };
        let radius = (kernel / 2) as isize;
        let mut spatial = vec![0.0; REFERENCE_PIXELS];
        for (pixel, value) in spatial.iter_mut().enumerate() {
            let (x, y) = (pixel % REFERENCE_SIDE, pixel / REFERENCE_SIDE);
            for (tap, weight) in dense_spatial_kernel(layer).iter().enumerate() {
                let sx = x as isize + (tap % kernel) as isize - radius;
                let sy = y as isize + (tap / kernel) as isize - radius;
                if (0..REFERENCE_SIDE as isize).contains(&sx) && (0..REFERENCE_SIDE as isize).contains(&sy) {
                    *value += contracted[sy as usize * REFERENCE_SIDE + sx as usize] * f64::from(*weight);
                }
            }
        }
        let channels = REFERENCE_CHANNELS[layer + 1];
        let mut next = vec![0.0; REFERENCE_PIXELS];
        if layer == 3 { final_planes.resize(channels * REFERENCE_PIXELS, 0.0); }
        for channel in 0..channels {
            let mut active = false;
            for (pixel, value) in spatial.iter().enumerate() {
                let raw = value * f64::from(dense_output_factor(layer, channel)) + f64::from(dense_bias(layer, channel));
                let activation = if layer < 3 { raw.max(0.0) } else { raw };
                active |= activation > 0.0;
                clipped += usize::from(layer < 3 && raw < 0.0);
                if layer == 3 {
                    final_planes[channel * REFERENCE_PIXELS + pixel] = activation;
                } else {
                    next[pixel] += activation * f64::from(dense_input_factor(layer + 1, channel));
                }
            }
            assert!(active, "fixture layer {layer} channel {channel} must affect the result");
        }
        contracted = next;
    }
    assert!(clipped > 0, "fixture must exercise negative preactivations and ReLU");
    let side = REFERENCE_SIDE * 3;
    let mut expected = Vec::with_capacity(side * side * 4);
    for y in 0..side {
        for x in 0..side {
            let pixel = (y / 3) * REFERENCE_SIDE + x / 3;
            let channel = (y % 3) * 3 + x % 3;
            let luma = final_planes[channel * REFERENCE_PIXELS + pixel];
            let [r, g, b] = patterned_rgb(x / 3, y / 3);
            let cb = -0.169 * r - 0.331 * g + 0.5 * b;
            let cr = 0.5 * r - 0.419 * g - 0.081 * b;
            let rgb = [luma + 1.402 * cr, luma - 0.344 * cb - 0.714 * cr, luma + 1.772 * cb];
            expected.extend(rgb.map(|value| (value.clamp(0.0, 1.0) * 255.0 + 0.5).floor() as u8));
            expected.push(255);
        }
    }
    expected
}

fn texture(device: &wgpu::Device, width: u32, height: u32, format: wgpu::TextureFormat, usage: wgpu::TextureUsages) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("resident regression source"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
        format, usage, view_formats: &[],
    })
}

fn generate(device: &wgpu::Device, queue: &wgpu::Queue, image: &wgpu::Texture, color: &str) {
    assert!(!image.usage().contains(wgpu::TextureUsages::COPY_DST));
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("test GPU-only source generator"),
        source: wgpu::ShaderSource::Wgsl(r#"
            @group(0) @binding(0) var image: texture_storage_2d<rgba8unorm, write>;
            fn store(p: vec3u, color: vec4f) {
                if (all(p.xy < textureDimensions(image))) { textureStore(image, vec2i(p.xy), color); }
            }
            @compute @workgroup_size(8, 8) fn blue(@builtin(global_invocation_id) p: vec3u) {
                store(p, vec4f(0.0, 0.0, 1.0, 1.0));
            }
            @compute @workgroup_size(8, 8) fn red(@builtin(global_invocation_id) p: vec3u) {
                store(p, vec4f(1.0, 0.0, 0.0, 1.0));
            }
            @compute @workgroup_size(8, 8) fn green(@builtin(global_invocation_id) p: vec3u) {
                store(p, vec4f(0.0, 1.0, 0.0, 1.0));
            }
            @compute @workgroup_size(8, 8) fn pattern(@builtin(global_invocation_id) p: vec3u) {
                var seed = p.x * 1664525u + p.y * 1013904223u + 0x9e3779b9u;
                seed = seed ^ (seed >> 16u);
                seed = seed * 2246822519u;
                seed = seed ^ (seed >> 13u);
                // Extract explicitly bounded byte fields; no non-power-of-two
                // integer remainder or implicit vector conversion is needed.
                let r = f32(64u + (seed & 127u)) / 255.0;
                let g = f32(64u + ((seed >> 8u) & 127u)) / 255.0;
                let b = f32(64u + ((seed >> 16u) & 127u)) / 255.0;
                store(p, vec4f(r, g, b, 1.0));
            }
        "#.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("test source compute"), layout: None, module: &shader,
        entry_point: Some(color), compilation_options: Default::default(), cache: None,
    });
    let view = image.create_view(&Default::default());
    let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None, layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &binding, &[]);
        pass.dispatch_workgroups(image.width().div_ceil(8), image.height().div_ceil(8), 1);
    }
    queue.submit([encoder.finish()]);
}

fn readback(device: &wgpu::Device, queue: &wgpu::Queue, output: &GpuOutput) -> Vec<u8> {
    readback_texture(device, queue, output.texture())
}

fn readback_texture(device: &wgpu::Device, queue: &wgpu::Queue, output: &wgpu::Texture) -> Vec<u8> {
    let row = (output.width() * 4).div_ceil(256) * 256;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test explicit readback boundary"), size: u64::from(row) * u64::from(output.height()),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture: output, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo { buffer: &staging, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(output.height()) } },
        wgpu::Extent3d { width: output.width(), height: output.height(), depth_or_array_layers: 1 },
    );
    let submission = queue.submit([encoder.finish()]);
    let (sender, receiver) = std::sync::mpsc::channel();
    staging.map_async(wgpu::MapMode::Read, .., move |result| { let _ = sender.send(result); });
    device.poll(wgpu::PollType::Wait {
        submission_index: Some(submission), timeout: Some(Duration::from_secs(120)),
    }).expect("bounded GPU readback wait");
    receiver.recv_timeout(Duration::from_secs(1)).expect("map callback after GPU wait").expect("readback mapping");
    let mapped = staging.get_mapped_range(..).expect("mapped verification bytes");
    let bytes = mapped.chunks_exact(row as usize)
        .flat_map(|row| row[..output.width() as usize * 4].iter().copied()).collect();
    drop(mapped);
    staging.unmap();
    bytes
}

fn assert_color(bytes: &[u8], expected: [u8; 4]) {
    assert!(!bytes.is_empty());
    for (index, pixel) in bytes.chunks_exact(4).enumerate() {
        for channel in 0..3 {
            assert!(pixel[channel].abs_diff(expected[channel]) <= 1, "pixel {index}: {pixel:?}, expected {expected:?}");
        }
        assert_eq!(pixel[3], expected[3], "pixel {index} alpha");
    }
}

fn assert_counters(session: &GpuImageSession, submissions: u64) {
    let counters = session.counters();
    assert_eq!(counters.host_image_bytes, 0);
    assert_eq!(counters.submissions, submissions);
    assert_eq!(counters.compute_dispatches, submissions * 6);
}

#[test]
fn metal_resident_numeric_resources_and_device_lifecycle() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL, ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions::default()).await.expect("hardware Metal adapter required");
        assert_eq!(adapter.get_info().backend, wgpu::Backend::Metal);
        assert_ne!(adapter.get_info().device_type, wgpu::DeviceType::Cpu);
        // The production device and queue come from the same, single request.
        let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor::default()).await.expect("Metal device");
        let (device, queue) = (Arc::new(device), Arc::new(queue));
        let mut session = GpuImageSession::prepare(device.clone(), queue.clone(), identity_model()).expect("full four-convolution profile");
        assert!(session.counters().weight_upload_bytes > 0);
        assert_counters(&session, 0);
        let source = texture(&device, 2, 2, wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING);
        generate(&device, &queue, &source, "blue");
        let input = session.bind_input(&source, 2, 2).unwrap();
        let first = session.submit(&input).unwrap();
        assert_counters(&session, 1);
        assert_eq!((first.width(), first.height()), (6, 6));
        let first_bytes = readback(&device, &queue, &first);
        assert_eq!(first_bytes.len(), 6 * 6 * 4);
        assert_color(&first_bytes, [0, 0, 255, 255]);

        // Distinct content makes an accidental overwrite observable, not just an
        // assertion about handles or forwarded ownership fields.
        generate(&device, &queue, &source, "green");
        let second = session.submit(&input).unwrap();
        assert_color(&readback(&device, &queue, &second), [0, 255, 0, 255]);
        generate(&device, &queue, &source, "red");
        let third = session.submit(&input).unwrap();
        let third_bytes = readback(&device, &queue, &third);
        assert_color(&third_bytes, [255, 0, 0, 255]);
        assert!(matches!(session.submit(&input), Err(GpuError::Capacity)));
        assert_counters(&session, 3);
        assert_eq!(readback(&device, &queue, &first), first_bytes);
        assert_eq!(readback(&device, &queue, &third), third_bytes);
        let released_texture = second.texture().clone();
        drop(second);
        generate(&device, &queue, &source, "green");
        let replacement = session.submit(&input).unwrap();
        assert_eq!(replacement.texture(), &released_texture);
        assert_ne!(replacement.texture(), first.texture());
        assert_ne!(replacement.texture(), third.texture());
        assert_color(&readback(&device, &queue, &replacement), [0, 255, 0, 255]);
        assert_eq!(readback(&device, &queue, &first), first_bytes);
        assert_eq!(readback(&device, &queue, &third), third_bytes);
        assert_counters(&session, 4);

        let mut other_session = GpuImageSession::prepare(device.clone(), queue.clone(), identity_model()).unwrap();
        assert!(matches!(other_session.submit(&input), Err(GpuError::WrongSession)));
        assert_counters(&other_session, 0);
        let other_input = other_session.bind_input(&source, 2, 2).unwrap();
        assert!(matches!(session.submit(&other_input), Err(GpuError::WrongSession)));
        assert_counters(&session, 4);
        drop(other_session);

        // This second request is intentional: actual native cross-device binding
        // must be rejected by wgpu validation before inference queue submission.
        let (foreign_device, _foreign_queue) = adapter.request_device(&wgpu::DeviceDescriptor::default()).await.unwrap();
        let foreign = texture(&foreign_device, 2, 2, wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureUsages::TEXTURE_BINDING);
        assert!(matches!(session.bind_input(&foreign, 2, 2), Err(GpuError::Validation(_))));
        assert_counters(&session, 4);
        let missing_binding = texture(&device, 2, 2, wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureUsages::COPY_SRC);
        let srgb = texture(&device, 2, 2, wgpu::TextureFormat::Rgba8UnormSrgb, wgpu::TextureUsages::TEXTURE_BINDING);
        for (image, width, height) in [(&missing_binding, 2, 2), (&srgb, 2, 2), (&source, 0, 2), (&source, 2, 0), (&source, 1, 2), (&source, 2, 3)] {
            assert!(matches!(session.submit_texture(image, width, height), Err(GpuError::InvalidImage(_))));
            assert_counters(&session, 4);
        }

        drop((first, third, replacement));
        let odd = texture(&device, 3, 1, wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING);
        generate(&device, &queue, &odd, "blue");
        let odd_output = session.submit_texture(&odd, 3, 1).unwrap();
        assert_eq!((odd_output.width(), odd_output.height()), (9, 3));
        let odd_bytes = readback(&device, &queue, &odd_output);
        assert_eq!(odd_bytes.len(), 9 * 3 * 4);
        assert_color(&odd_bytes, [0, 0, 255, 255]);
        assert_counters(&session, 5);
    });
}

#[test]
fn metal_resident_dense_tiled_numeric_parity() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL, ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions::default()).await.expect("hardware Metal adapter required");
        assert_eq!(adapter.get_info().backend, wgpu::Backend::Metal);
        assert_ne!(adapter.get_info().device_type, wgpu::DeviceType::Cpu);
        let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor::default()).await.expect("Metal device");
        let (device, queue) = (Arc::new(device), Arc::new(queue));
        let mut session = GpuImageSession::prepare(device.clone(), queue.clone(), dense_model()).expect("dense FP32 four-convolution profile");
        let source = texture(&device, 224, 224, wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC);
        generate(&device, &queue, &source, "pattern");
        let source_bytes = readback_texture(&device, &queue, &source);
        for (pixel, actual) in source_bytes.chunks_exact(4).enumerate() {
            let expected = patterned_rgb(pixel % REFERENCE_SIDE, pixel / REFERENCE_SIDE)
                .map(|value| (value * 255.0).round() as u8);
            assert_eq!(&actual[..3], &expected, "GPU source pattern at ({}, {})", pixel % REFERENCE_SIDE, pixel / REFERENCE_SIDE);
            assert_eq!(actual[3], 255, "GPU source alpha at pixel {pixel}");
        }
        let input = session.bind_input(&source, 224, 224).unwrap();
        let output = session.submit(&input).unwrap();
        assert_eq!((output.width(), output.height()), (672, 672));
        assert_counters(&session, 1);
        let actual = readback(&device, &queue, &output);
        let expected = dense_reference();
        assert_eq!(actual.len(), expected.len());
        let mut total_error = 0u64;
        let mut max_error = 0u8;
        let mut worst = (0usize, 0usize);
        // Inspect every pixel, including corners, both sides of every spatial
        // workgroup boundary, all hidden channels, and the final 9-channel tail.
        for (pixel, (actual, expected)) in actual.chunks_exact(4).zip(expected.chunks_exact(4)).enumerate() {
            assert_eq!(actual[3], 255, "pixel {pixel} alpha");
            for channel in 0..3 {
                let error = actual[channel].abs_diff(expected[channel]);
                total_error += u64::from(error);
                if error > max_error {
                    max_error = error;
                    worst = (pixel, channel);
                }
            }
        }
        let mean_error = total_error as f64 / (672 * 672 * 3) as f64;
        assert!(max_error <= 1, "dense FP32 max RGB byte error {max_error} at ({}, {}) component {}; actual {:?}, expected {:?}; mean {mean_error}", worst.0 % 672, worst.0 / 672, worst.1, &actual[worst.0 * 4..worst.0 * 4 + 4], &expected[worst.0 * 4..worst.0 * 4 + 4]);
        assert!(mean_error <= 0.05, "dense FP32 mean RGB byte error {mean_error}; max {max_error}");
    });
}

#[path = "gpu_boundary_tests.rs"]
mod boundary_tests;

#[path = "catalog_gpu_behavior_tests.rs"]
mod catalog_behavior;
