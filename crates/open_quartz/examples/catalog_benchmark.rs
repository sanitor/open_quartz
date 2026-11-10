//! One-artifact CPU roundtrip versus resident Metal completion benchmark.
//!
//! Usage: catalog_benchmark MODEL_ID MODEL.onnx REPORT.json [--width 1280]
//!        [--height 720] [--warmup 2] [--samples 10]
//! Run with native-runtime,native-onnx in release mode. SR3 retains its separate
//! resident_benchmark. All explicit diagnostic reads below are outside samples.
use open_quartz::inference::{
    detect_postprocess, run_image_task_with_request_and_metrics, Detection,
    InferenceImageMetrics, InferenceSession, InferenceTask, NamedTensor, RequestStamp,
};
use oq_inference::{
    catalog_gpu::{CatalogGpuSession, CatalogOptions},
    catalog_model::CatalogModel,
    gpu::GpuCounters,
};
use serde_json::{json, Value};
use std::{collections::BTreeMap, error::Error, sync::{mpsc, Arc}, time::{Duration, Instant}};

type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;
const TIMEOUT: Duration = Duration::from_secs(120);
const SCORE: f32 = 0.25;
const IOU: f32 = 0.45;
const ABS: f64 = 1e-3;
const REL: f64 = 1e-3;

fn wait(device: &wgpu::Device, submission: wgpu::SubmissionIndex) -> Result<()> {
    device.poll(wgpu::PollType::Wait { submission_index: Some(submission), timeout: Some(TIMEOUT) })?;
    Ok(())
}
fn map(device: &wgpu::Device, buffer: &wgpu::Buffer, submission: wgpu::SubmissionIndex) -> Result<()> {
    let (sender, receiver) = mpsc::sync_channel(1);
    buffer.map_async(wgpu::MapMode::Read, .., move |result| { let _ = sender.send(result); });
    wait(device, submission)?;
    receiver.recv_timeout(TIMEOUT)??;
    Ok(())
}
fn extent(width: u32, height: u32) -> wgpu::Extent3d {
    wgpu::Extent3d { width, height, depth_or_array_layers: 1 }
}
fn texture(device: &wgpu::Device, width: u32, height: u32, usage: wgpu::TextureUsages) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("catalog benchmark RGBA8"), size: extent(width, height), mip_level_count: 1,
        sample_count: 1, dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm, usage, view_formats: &[],
    })
}

struct Readback {
    staging: wgpu::Buffer,
    rgba: Vec<u8>,
    width: u32,
    height: u32,
    row: u32,
}
impl Readback {
    fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let row = (width * 4).div_ceil(256) * 256;
        Self {
            staging: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("catalog benchmark reusable readback"), size: u64::from(row) * u64::from(height),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
            rgba: vec![0; width as usize * height as usize * 4], width, height, row,
        }
    }
    fn read(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, source: &wgpu::Texture) -> Result<()> {
        if source.width() != self.width || source.height() != self.height {
            return Err("readback texture dimensions differ from the prepared case".into());
        }
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: source, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &self.staging, layout: wgpu::TexelCopyBufferLayout {
                offset: 0, bytes_per_row: Some(self.row), rows_per_image: Some(self.height),
            } }, extent(self.width, self.height),
        );
        map(device, &self.staging, queue.submit([encoder.finish()]))?;
        let mapped = self.staging.get_mapped_range(..)?;
        for (source, target) in mapped.chunks_exact(self.row as usize)
            .zip(self.rgba.chunks_exact_mut(self.width as usize * 4)) {
            target.copy_from_slice(&source[..target.len()]);
        }
        drop(mapped);
        self.staging.unmap();
        Ok(())
    }
}

fn generate(device: &wgpu::Device, queue: &wgpu::Queue, source: &wgpu::Texture) -> Result<()> {
    // Keep this source byte-for-byte identical to resident_benchmark's gradient.
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("benchmark exact retained gradient"),
        source: wgpu::ShaderSource::Wgsl(r#"
            @group(0) @binding(0) var image: texture_storage_2d<rgba8unorm, write>;
            @compute @workgroup_size(8, 8) fn main(@builtin(global_invocation_id) p: vec3u) {
                if (any(p.xy >= textureDimensions(image))) { return; }
                let r = f32((p.x * 13u + p.y * 3u) % 256u) / 255.0;
                let g = f32((p.y * 7u + p.x * 2u) % 256u) / 255.0;
                textureStore(image, vec2i(p.xy), vec4f(r, g, 80.0 / 255.0, 1.0));
            }
        "#.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("benchmark source generation"), layout: None, module: &module,
        entry_point: Some("main"), compilation_options: Default::default(), cache: None,
    });
    let view = source.create_view(&Default::default());
    let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None, layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &binding, &[]);
        pass.dispatch_workgroups(source.width().div_ceil(8), source.height().div_ceil(8), 1);
    }
    wait(device, queue.submit([encoder.finish()]))
}

// These buffers are allocated only for opt-in diagnostics, never in either path's samples.
fn copy_buffer(device: &wgpu::Device, queue: &wgpu::Queue, source: &wgpu::Buffer) -> Result<wgpu::Buffer> {
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("catalog benchmark explicit diagnostic read"), size: source.size(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(source, 0, &staging, 0, source.size());
    map(device, &staging, queue.submit([encoder.finish()]))?;
    Ok(staging)
}
fn read_f32(device: &wgpu::Device, queue: &wgpu::Queue, source: &wgpu::Buffer, elements: usize) -> Result<Vec<f32>> {
    if source.size() != elements as u64 * 4 { return Err("diagnostic tensor buffer length differs from CatalogModel shape".into()); }
    let staging = copy_buffer(device, queue, source)?;
    let mapped = staging.get_mapped_range(..)?;
    let values = mapped.chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect();
    drop(mapped);
    staging.unmap();
    Ok(values)
}
fn read_bytes(device: &wgpu::Device, queue: &wgpu::Queue, source: &wgpu::Buffer) -> Result<Vec<u8>> {
    let staging = copy_buffer(device, queue, source)?;
    let mapped = staging.get_mapped_range(..)?;
    let values = mapped.to_vec();
    drop(mapped);
    staging.unmap();
    Ok(values)
}
fn elements(shape: &[usize]) -> Result<usize> {
    shape.iter().try_fold(1usize, |n, &d| n.checked_mul(d)).filter(|&n| n != 0)
        .ok_or_else(|| "invalid diagnostic tensor shape".into())
}
fn ms(start: Instant) -> f64 { start.elapsed().as_secs_f64() * 1000.0 }

#[derive(Clone, Copy)]
struct CpuSample {
    total: f64, readback: f64, preprocess: f64, runtime: f64, postprocess: f64, upload: f64,
}
struct Case<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    source: wgpu::Texture,
    cpu_output: wgpu::Texture,
    input: Readback,
    cpu: &'a mut InferenceSession,
    gpu: &'a mut CatalogGpuSession,
    model_id: &'a str,
    task: InferenceTask,
    target_size: u32,
    request_id: u64,
}
impl Case<'_> {
    fn stamp(&mut self) -> RequestStamp {
        self.request_id += 1;
        RequestStamp { request_id: self.request_id, session_generation: self.cpu.generation() }
    }
    fn cpu(&mut self) -> Result<CpuSample> {
        let start = Instant::now();
        self.input.read(self.device, self.queue, &self.source)?;
        let readback = ms(start);
        let stamp = self.stamp();
        let (output, metrics) = run_image_task_with_request_and_metrics(
            self.cpu, stamp, self.task, self.model_id, &self.input.rgba,
            self.input.width, self.input.height, self.target_size, SCORE, IOU,
        )?;
        if output.width != self.cpu_output.width() || output.height != self.cpu_output.height()
            || output.rgba.len() != output.width as usize * output.height as usize * 4 {
            return Err("CPU image task returned dimensions/length different from the prepared case".into());
        }
        let upload_start = Instant::now();
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &self.cpu_output, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &output.rgba,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(output.width * 4), rows_per_image: Some(output.height) },
            extent(output.width, output.height),
        );
        wait(self.device, self.queue.submit([]))?;
        let upload = ms(upload_start);
        let total = ms(start);
        let InferenceImageMetrics { preprocess_ns, runtime_ns, postprocess_ns, .. } = metrics;
        Ok(CpuSample { total, readback, preprocess: preprocess_ns as f64 / 1e6,
            runtime: runtime_ns as f64 / 1e6, postprocess: postprocess_ns as f64 / 1e6, upload })
    }
    fn gpu(&mut self) -> Result<f64> {
        let start = Instant::now();
        let output = self.gpu.submit_texture(&self.source, self.input.width, self.input.height)?;
        wait(self.device, output.submission_index().clone())?;
        drop(output); // Release the pooled output lease before the next sample.
        Ok(ms(start))
    }
    fn verify(&mut self, metadata: &Metadata, report: &mut Value) -> Result<()> {
        // CPU expected pixels originate from actual GPU texture readback, never host upload.
        self.cpu()?;
        let mut source_mismatches = 0usize;
        for y in 0..self.input.height {
            for x in 0..self.input.width {
                let index = (y as usize * self.input.width as usize + x as usize) * 4;
                let expected = [((x * 13 + y * 3) % 256) as u8,
                    ((y * 7 + x * 2) % 256) as u8, 80, 255];
                source_mismatches += self.input.rgba[index..index + 4].iter().zip(expected)
                    .filter(|(a, b)| **a != *b).count();
            }
        }
        report["parity"]["source"] = json!({"passed": source_mismatches == 0,
            "checkedBytes": self.input.rgba.len(), "mismatchedBytes": source_mismatches,
            "actualGpuSourceReadback": true, "hostUploadUsed": false});
        let output = self.gpu.submit_texture(&self.source, self.input.width, self.input.height)?;
        wait(self.device, output.submission_index().clone())?;
        if output.width() != self.cpu_output.width() || output.height() != self.cpu_output.height() {
            return Err("GPU image dimensions differ from CPU image dimensions".into());
        }
        let input = read_f32(self.device, self.queue, &self.gpu.input_buffer(), elements(&metadata.input_shape)?)?;
        if input.iter().any(|v| !v.is_finite()) { return Err("prepared GPU input contains non-finite values".into()); }
        let shape: Vec<i64> = metadata.input_shape.iter().map(|&v| v as i64).collect();
        let (reference, actual_shape) = if metadata.output_dtype == 2 {
            let (data, shape) = self.cpu.run_u8(input, shape)?;
            (data.into_iter().map(f32::from).collect::<Vec<_>>(), shape)
        } else {
            let stamp = self.stamp();
            let expected_shape: Vec<i64> = metadata.output_shape.iter().map(|&dimension| dimension as i64).collect();
            let shape_override = [(metadata.output_name.as_str(), expected_shape.as_slice())];
            let mut result = self.cpu.invoke_with_output_shapes(stamp, vec![NamedTensor {
                name: metadata.input_name.clone(), data: input, shape,
            }], std::slice::from_ref(&metadata.output_name), &shape_override)?;
            if result.outputs.len() != 1 || result.outputs[0].name != metadata.output_name {
                return Err("CPU diagnostic did not return the exact selected catalog output".into());
            }
            let result = result.outputs.remove(0);
            (result.data, result.shape)
        };
        let expected_shape: Vec<i64> = metadata.output_shape.iter().map(|&v| v as i64).collect();
        report["parity"]["rawShape"] = json!({"expected": expected_shape, "cpu": actual_shape,
            "selectedOutput": metadata.output_name, "inputShape": metadata.input_shape,
            "inputReadFromGpu": true});
        if actual_shape != expected_shape || reference.len() != elements(&metadata.output_shape)? {
            return Err("CPU diagnostic output shape differs from CatalogModel.output_shape".into());
        }
        let actual = read_f32(self.device, self.queue, &output.raw_result_buffer(), reference.len())?;
        if self.model_id == "midas-small" {
            let partial=output.depth_partial_buffer().ok_or("MiDaS result has no partial extrema")?;
            let partials=read_f32(self.device,self.queue,&partial,512)?;
            let range = output.depth_range_buffer().ok_or("MiDaS GPU result lacks its resident range")?;
            let values = read_f32(self.device, self.queue, &range, 2)?;
            let cpu_min = reference.iter().copied().fold(f32::INFINITY, f32::min);
            let cpu_max = reference.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let tolerance = ABS as f32 + REL as f32 * cpu_min.abs().max(cpu_max.abs());
            let passed = (values[0] - cpu_min).abs() <= tolerance && (values[1] - cpu_max).abs() <= tolerance;
            report["parity"]["depthPartialSamples"] = json!({"first":&partials[..16],"last":&partials[496..512]});
            report["parity"]["depthRange"] = json!({"gpuMinimum":values[0],"gpuMaximum":values[1],"cpuMinimum":cpu_min,"cpuMaximum":cpu_max,"tolerance":tolerance,"passed":passed});
            if !passed { return Err("GPU depth range does not match the original CPU image normalization".into()); }
        }
        let raw = raw_parity(&reference, &actual, metadata.output_dtype == 2);
        let raw_passed = raw["passed"] == true;
        report["parity"]["rawModel"] = raw;
        let mut detection_passed = true;
        if self.model_id == "yolov8n" {
            let buffer = output.detection_buffer().ok_or("YOLO GPU output lacks its structured detection buffer")?;
            let bytes = read_bytes(self.device, self.queue, &buffer)?;
            let diagnostic = detection_parity(&bytes, &reference, &actual, self.input.width, self.input.height)?;
            detection_passed = diagnostic["passed"] == true;
            report["parity"]["detections"] = diagnostic;
        }
        // Release both full raw tensors before allocating large ESR RGBA readbacks.
        drop(reference);
        drop(actual);
        let mut cpu = Readback::new(self.device, output.width(), output.height());
        let mut gpu = Readback::new(self.device, output.width(), output.height());
        cpu.read(self.device, self.queue, &self.cpu_output)?;
        gpu.read(self.device, self.queue, output.texture())?;
        let image = image_parity(&cpu.rgba, &gpu.rgba,
            !matches!(self.task, InferenceTask::BackgroundRemoval));
        let image_passed = image["passed"] == true;
        report["parity"]["image"] = image;
        report["parity"]["explicitReadbackOutsideMeasurements"] = json!(true);
        let passed = source_mismatches == 0 && raw_passed && detection_passed && image_passed;
        report["parity"]["passed"] = json!(passed);
        drop(output);
        if !passed { return Err("catalog parity failed; raw/image/detection statistics are preserved in the report".into()); }
        Ok(())
    }
}

struct Metadata {
    input_name: String,
    input_shape: Vec<usize>,
    output_name: String,
    output_shape: Vec<usize>,
    output_dtype: i32,
}
fn raw_parity(cpu: &[f32], gpu: &[f32], exact: bool) -> Value {
    let mut max_abs = 0.0f64;
    let mut max_scaled = 0.0f64;
    let mut sum_abs = 0.0f64;
    let mut worst = 0usize;
    let mut worst_abs = 0usize;
    let mut mismatches = 0usize;
    let mut nonfinite = 0usize;
    for (index, (&a, &b)) in cpu.iter().zip(gpu).enumerate() {
        if !a.is_finite() || !b.is_finite() {
            if nonfinite == 0 { worst = index; worst_abs = index; }
            nonfinite += 1;
            mismatches += 1;
            // JSON serializes these undefined aggregate errors as null, not a
            // deceptively finite maximum or mean. The offending index is kept.
            max_abs = f64::INFINITY;
            max_scaled = f64::INFINITY;
            sum_abs = f64::INFINITY;
            continue;
        }
        let difference = (f64::from(a) - f64::from(b)).abs();
        let scaled = difference / (ABS + REL * f64::from(a).abs());
        sum_abs += difference;
        if difference > max_abs { max_abs = difference; worst_abs = index; }
        if scaled > max_scaled { max_scaled = scaled; worst = index; }
        if (exact && a != b) || (!exact && scaled > 1.0) { mismatches += 1; }
    }
    json!({"passed": mismatches == 0 && cpu.len() == gpu.len(), "checkedValues": cpu.len(),
        "cpuValues": cpu.len(), "gpuValues": gpu.len(), "mismatchedValues": mismatches,
        "nonFinitePairs": nonfinite, "maxAbsoluteDifference": max_abs,
        "meanAbsoluteDifference": sum_abs / cpu.len() as f64, "maxScaledError": max_scaled,
        "worstIndex": worst, "maxAbsoluteIndex": worst_abs, "worstCpu": cpu[worst], "worstGpu": gpu[worst],
        "absoluteTolerance": ABS, "relativeTolerance": REL, "exactClassIdsRequired": exact,
        "comparison": if exact { "every UInt8 class ID equals its GPU FP32 export exactly" }
            else { "abs(gpu-cpu) <= 1e-3 + 1e-3*abs(cpu), every original selected output value" }})
}
fn image_parity(cpu: &[u8], gpu: &[u8], opaque: bool) -> Value {
    let mut max = 0u8;
    let mut sum = 0u64;
    let mut worst = 0usize;
    let mut changed = 0usize;
    let mut alpha_changed = 0usize;
    let mut nonopaque = 0usize;
    let mut cpu_alpha = [255u8, 0u8];
    let mut gpu_alpha = [255u8, 0u8];
    for (index, (&a, &b)) in cpu.iter().zip(gpu).enumerate() {
        let difference = a.abs_diff(b);
        if difference > max { max = difference; worst = index; }
        sum += u64::from(difference);
        changed += usize::from(a != b);
        if index % 4 == 3 {
            alpha_changed += usize::from(a != b);
            nonopaque += usize::from(b != 255);
            cpu_alpha = [cpu_alpha[0].min(a), cpu_alpha[1].max(a)];
            gpu_alpha = [gpu_alpha[0].min(b), gpu_alpha[1].max(b)];
        }
    }
    let mean = sum as f64 / cpu.len() as f64;
    let first_pixels = (0..cpu.len().min(64)).step_by(4).map(|i| json!({"index":i/4,"cpu":&cpu[i..i+4],"gpu":&gpu[i..i+4]})).collect::<Vec<_>>();
    json!({"passed": cpu.len() == gpu.len() && max <= 3 && mean <= 0.5 && (!opaque || nonopaque == 0),
        "checkedBytes": cpu.len(), "changedBytes": changed, "maxByteDifference": max,
        "meanByteDifference": mean, "worstByteIndex": worst, "maxAllowed": 3, "meanAllowed": 0.5,
        "alpha": {"expected": if opaque { "255" } else { "CPU model alpha mask" },
            "cpuRange": cpu_alpha, "gpuRange": gpu_alpha, "changedBytes": alpha_changed,
            "gpuNonOpaquePixels": nonopaque}, "samplePixels": first_pixels,
        "classificationBoundaryExceptions": false})
}
fn decode_reference(raw: &[f32], width: u32, height: u32) -> Vec<Detection> {
    let scale = (640.0 / width as f32).min(640.0 / height as f32);
    let resized_width = (width as f32 * scale).round();
    let resized_height = (height as f32 * scale).round();
    detect_postprocess(raw, width, height, scale, (640.0 - resized_width) * 0.5,
        (640.0 - resized_height) * 0.5, SCORE, IOU)
}
fn detection_parity(bytes: &[u8], cpu_raw: &[f32], gpu_raw: &[f32], width: u32, height: u32) -> Result<Value> {
    if bytes.len() < 16 || (bytes.len() - 16) % 32 != 0 { return Err("invalid detection buffer byte length".into()); }
    let word = |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    let count = word(0) as usize;
    if count > 8400 || count > (bytes.len() - 16) / 32 { return Err("detection buffer count exceeds its capacity".into()); }
    let header_valid = word(4) == 0 && word(8) == 0 && word(12) == 0;
    let mut actual = Vec::with_capacity(count);
    let mut records = Vec::with_capacity(count);
    let mut valid = header_valid;
    let mut prior: Option<(f32, u32)> = None;
    for i in 0..count {
        let offset = 16 + i * 32;
        let bbox = std::array::from_fn(|axis| f32::from_bits(word(offset + axis * 4)));
        let score = f32::from_bits(word(offset + 16));
        let class = word(offset + 20);
        let index = word(offset + 24);
        valid &= score.is_finite() && class < 80 && index < 8400 && word(offset + 28) == 0
            && bbox.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v));
        if let Some((previous_score, previous_index)) = prior {
            valid &= score < previous_score || (score == previous_score && index > previous_index);
        }
        prior = Some((score, index));
        actual.push(Detection { bbox, score, class_id: class as usize });
        records.push(json!({"bbox": bbox, "score": score, "classId": class, "originalIndex": index}));
    }
    let expected = decode_reference(cpu_raw, width, height);
    let gpu_decoder_reference = decode_reference(gpu_raw, width, height);
    let compare = |reference: &[Detection]| {
        let mut max_bbox = 0.0f64;
        let mut max_score = 0.0f64;
        let mut passed = reference.len() == actual.len();
        for (a, b) in reference.iter().zip(&actual) {
            passed &= a.class_id == b.class_id;
            for (x, y) in a.bbox.iter().zip(b.bbox) {
                let delta = (f64::from(*x) - f64::from(y)).abs();
                max_bbox = max_bbox.max(delta);
                passed &= delta <= ABS + REL * f64::from(*x).abs();
            }
            let delta = (f64::from(a.score) - f64::from(b.score)).abs();
            max_score = max_score.max(delta);
            passed &= delta <= ABS + REL * f64::from(a.score).abs();
        }
        json!({"passed": passed, "cpuCount": reference.len(), "gpuCount": actual.len(),
            "maxBboxDifference": max_bbox, "maxScoreDifference": max_score})
    };
    let cpu_compare = compare(&expected);
    let decoder_compare = compare(&gpu_decoder_reference);
    Ok(json!({"passed": valid && cpu_compare["passed"] == true && decoder_compare["passed"] == true,
        "count": count, "recordBytes": 32, "headerBytes": 16, "headerAndRecordsValid": valid,
        "cpuOriginalOutputComparison": cpu_compare, "sameGpuRawDecoderComparison": decoder_compare,
        "cpuDetections": expected, "gpuRecords": records, "orderComparedExactly": true,
        "absoluteTolerance": ABS, "relativeTolerance": REL, "nonemptyEvidence": count > 0,
        "fixtureLimitation": "Gradient only; empty detections do not establish nonempty decoder coverage"}))
}
fn stats(samples: &[f64]) -> Value {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    let mean = samples.iter().sum::<f64>() / n as f64;
    json!({"n": n, "meanMs": mean,
        "p50Ms": sorted[((n as f64 * 0.5).ceil() as usize).saturating_sub(1)],
        "p95Ms": sorted[((n as f64 * 0.95).ceil() as usize).saturating_sub(1)],
        "minMs": sorted[0], "maxMs": sorted[n - 1], "serialEquivalentFps": 1000.0 / mean})
}
fn stage(samples: &[CpuSample], select: fn(&CpuSample) -> f64) -> Value {
    let values: Vec<_> = samples.iter().map(select).collect();
    json!({"summary": stats(&values), "samplesMs": values})
}
fn counters(c: GpuCounters) -> Value {
    json!({"submissions": c.submissions, "computeDispatches": c.compute_dispatches,
        "hostImageBytes": c.host_image_bytes, "weightUploadBytes": c.weight_upload_bytes,
        "scratchBytes": c.scratch_bytes, "lastEncodeWallMs": c.last_wall_time.as_secs_f64() * 1000.0,
        "totalEncodeWallMs": c.total_wall_time.as_secs_f64() * 1000.0,
        "gpuExecutionTimeMs": c.gpu_time.map(|t| t.as_secs_f64() * 1000.0)})
}
struct Args {
    model_id: String, model_path: String, report_path: String,
    width: u32, height: u32, warmup: usize, samples: usize,
}
impl Args {
    fn parse() -> Result<Self> {
        let mut args = std::env::args().skip(1);
        let mut result = Self {
            model_id: args.next().ok_or("Usage: catalog_benchmark MODEL_ID MODEL.onnx REPORT.json [--width 1280] [--height 720] [--warmup 2] [--samples 10]")?,
            model_path: args.next().ok_or("Missing MODEL.onnx")?,
            report_path: args.next().ok_or("Missing REPORT.json")?,
            width: 1280, height: 720, warmup: 2, samples: 10,
        };
        while let Some(flag) = args.next() {
            let value = args.next().ok_or("Missing option value")?;
            match flag.as_str() {
                "--width" => result.width = value.parse()?, "--height" => result.height = value.parse()?,
                "--warmup" => result.warmup = value.parse()?, "--samples" => result.samples = value.parse()?,
                _ => return Err(format!("Unknown option: {flag}").into()),
            }
        }
        if result.width == 0 || result.height == 0 || result.samples == 0 {
            return Err("width, height, and samples must be at least one".into());
        }
        Ok(result)
    }
}
fn verify_resource_lifecycle(device: &wgpu::Device, queue: &wgpu::Queue, gpu: &mut CatalogGpuSession,
    model_id: &str, source: &wgpu::Texture, width: u32, height: u32) -> Result<Value> {
    let mut held = Vec::with_capacity(3);
    for _ in 0..3 { held.push(gpu.submit_texture(source, width, height)?); }
    wait(device, held.last().ok_or("no resident outputs were created")?.submission_index().clone())?;
    let capacity_rejected = matches!(gpu.submit_texture(source, width, height), Err(oq_inference::gpu::GpuError::Capacity));
    if !capacity_rejected { return Err("resident output pool did not enforce its three-lease limit".into()); }
    let released = held.remove(1);
    let reusable_texture = released.texture_arc();
    drop(released);
    let reused = gpu.submit_texture(source, width, height)?;
    wait(device, reused.submission_index().clone())?;
    let released_slot_reused = reused.texture() == reusable_texture.as_ref();
    if !released_slot_reused { return Err("resident output slot was not reused after releasing its consumer lease".into()); }
    drop(reused);
    drop(held);
    let source_size_transition = if model_id == "realesrgan-x4" {
        if width < 2 || height < 2 { return Err("ESR lifecycle requires a source size that can be halved".into()); }
        let (small_width, small_height) = (width / 2, height / 2);
        let small_source = texture(device, small_width, small_height,
            wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING);
        generate(device, queue, &small_source)?;
        let smaller = gpu.submit_texture(&small_source, small_width, small_height)?;
        wait(device, smaller.submission_index().clone())?;
        let smaller_shape = (smaller.width(), smaller.height());
        drop(smaller);
        if smaller_shape != (small_width * 4, small_height * 4) {
            return Err("ESR output shape did not follow the smaller source profile".into());
        }
        let restored = gpu.submit_texture(source, width, height)?;
        wait(device, restored.submission_index().clone())?;
        let restored_shape = (restored.width(), restored.height());
        drop(restored);
        if restored_shape != (width * 4, height * 4) {
            return Err("ESR output shape did not return to the original source profile".into());
        }
        true
    } else { false };
    Ok(json!({"threeConsumerLeasesRetained": true, "fourthSubmissionRejected": capacity_rejected,
        "releasedSlotReused": released_slot_reused, "sourceSizeTransitionAndRestore": source_size_transition, "sourceBytes": 0}))
}

fn run(args: &Args, report: &mut Value) -> Result<()> {
    let (task, target) = match args.model_id.as_str() {
        "yolov8n" => (InferenceTask::Detection, 640),
        "yolo26n-sem" => (InferenceTask::Segmentation, 640),
        "u2netp" => (InferenceTask::BackgroundRemoval, 320),
        "modnet" => (InferenceTask::BackgroundRemoval, 512),
        "midas-small" => (InferenceTask::DepthEstimation, 256),
        "realesrgan-x4" => (InferenceTask::SuperResolution, args.width),
        _ => return Err(format!("Unsupported catalog benchmark model {}; use resident_benchmark for SR3", args.model_id).into()),
    };
    let bytes = std::fs::read(&args.model_path)?;
    let (input_width, input_height) = if args.model_id == "realesrgan-x4" {
        (args.width, args.height)
    } else { (target, target) };
    let model = CatalogModel::from_onnx(&bytes, &args.model_id, input_width, input_height)?;
    let graph: Value = serde_json::from_slice(&model.graph_json)?;
    let nodes = graph["nodes"].as_array().ok_or("catalog graph has no node array")?;
    let mut operators = BTreeMap::<String, usize>::new();
    for node in nodes {
        let op = node["op"].as_str().ok_or("catalog graph node has no operator")?;
        *operators.entry(op.to_owned()).or_default() += 1;
    }
    let metadata = Metadata {
        input_name: model.plan.input_name.clone(), input_shape: model.input_shape.clone(),
        output_name: model.output_name.clone(), output_shape: model.output_shape.clone(), output_dtype: model.output_dtype,
    };
    let scale = if args.model_id == "realesrgan-x4" { 4 } else { 1 };
    let output_width = args.width.checked_mul(scale).ok_or("output width overflow")?;
    let output_height = args.height.checked_mul(scale).ok_or("output height overflow")?;
    report["modelSha256"] = json!(model.plan.source_sha256);
    report["modelBytes"] = json!(model.plan.source_byte_length);
    report["sourceInterface"] = serde_json::to_value(&model.plan.source_interface)?;
    report["preparedInput"] = json!({"name": metadata.input_name, "shape": metadata.input_shape, "dtype": "float32"});
    report["preparedOutput"] = json!({"name": metadata.output_name, "shape": metadata.output_shape,
        "onnxDtype": metadata.output_dtype, "gpuExportDtype": "float32"});
    report["topology"] = json!({"nodeCount": nodes.len(), "operatorCounts": operators,
        "initializerCount": graph["initializers"].as_array().map(Vec::len), "sourceGraphUnmodified": true});
    report["output"] = json!([output_width, output_height]);
    let mut cpu = InferenceSession::from_memory(&bytes)?;
    report["cpuPlan"] = serde_json::to_value(cpu.plan())?;
    if cpu.plan().configured_backend != oq_inference::BackendPreference::OrtCpu {
        return Err("CPU baseline must use the default ORT CPU backend".into());
    }
    drop(bytes);
    drop(graph);
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL, ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))?;
    let info = adapter.get_info();
    if info.backend != wgpu::Backend::Metal || info.device_type == wgpu::DeviceType::Cpu {
        return Err("Hardware Metal adapter required; fallback forbidden".into());
    }
    let mut limits = wgpu::Limits::default();
    if args.model_id == "realesrgan-x4" {
        // 1280x720 -> 5120x2880 RGB FP32 is 176,947,200 bytes. Never downscale it.
        let supported = adapter.limits();
        limits.max_storage_buffer_binding_size = supported.max_storage_buffer_binding_size;
        limits.max_buffer_size = supported.max_buffer_size;
    }
    if output_width > limits.max_texture_dimension_2d || output_height > limits.max_texture_dimension_2d {
        return Err("requested case exceeds texture limits; resolution lowering is forbidden".into());
    }
    report["adapter"] = json!(format!("{info:?}"));
    report["deviceName"] = json!(info.name);
    report["requestedLimits"] = json!({"maxStorageBufferBindingSize": limits.max_storage_buffer_binding_size,
        "maxBufferSize": limits.max_buffer_size, "maxTextureDimension2d": limits.max_texture_dimension_2d});
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("shared catalog benchmark Metal device"), required_limits: limits, ..Default::default()
    }))?;
    let (device, queue) = (Arc::new(device), Arc::new(queue));
    let mut gpu = CatalogGpuSession::prepare(device.clone(), queue.clone(), model,
        CatalogOptions { score_threshold: SCORE, iou_threshold: IOU })?;
    let source = texture(&device, args.width, args.height, wgpu::TextureUsages::COPY_SRC
        | wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING);
    generate(&device, &queue, &source)?;
    let lifecycle = verify_resource_lifecycle(&device, &queue, &mut gpu, &args.model_id, &source, args.width, args.height)?;
    report["resourceLifecycle"] = lifecycle;
    let cpu_output = texture(&device, output_width, output_height,
        wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING);
    let mut case = Case { device: &device, queue: &queue, source, cpu_output,
        input: Readback::new(&device, args.width, args.height), cpu: &mut cpu, gpu: &mut gpu,
        model_id: &args.model_id, task, target_size: target, request_id: 0 };
    case.verify(&metadata, report)?;
    for _ in 0..args.warmup { case.cpu()?; case.gpu()?; }
    let before = case.gpu.counters();
    report["countersBeforeSamples"] = counters(before);
    let mut cpu_samples = Vec::with_capacity(args.samples);
    let mut gpu_samples = Vec::with_capacity(args.samples);
    let mut blocks = Vec::with_capacity(args.samples.min(3));
    let block_count = args.samples.min(3);
    for block in 0..block_count {
        let count = args.samples / block_count + usize::from(block < args.samples % block_count);
        let cpu_begin = cpu_samples.len();
        let gpu_begin = gpu_samples.len();
        // Serial path-only blocks, alternating which path goes first. No overlap.
        for path in 0..2 {
            if (block + path) % 2 == 0 {
                for _ in 0..count { cpu_samples.push(case.cpu()?); }
            } else {
                for _ in 0..count { gpu_samples.push(case.gpu()?); }
            }
        }
        let cpu_block: Vec<_> = cpu_samples[cpu_begin..].iter().map(|s| s.total).collect();
        blocks.push(json!({"order": if block % 2 == 0 { "cpu,gpu" } else { "gpu,cpu" },
            "cpuRoundtrip": stats(&cpu_block), "gpuCompletionWall": stats(&gpu_samples[gpu_begin..])}));
    }
    let after = case.gpu.counters();
    let cpu_totals: Vec<_> = cpu_samples.iter().map(|s| s.total).collect();
    report["blocks"] = json!(blocks);
    report["cpuRoundtrip"] = stats(&cpu_totals);
    report["gpuCompletionWall"] = stats(&gpu_samples);
    report["cpuSamplesMs"] = json!(cpu_totals);
    report["gpuSamplesMs"] = json!(gpu_samples);
    report["cpuStages"] = json!({"readback": stage(&cpu_samples, |s| s.readback),
        "preprocess": stage(&cpu_samples, |s| s.preprocess), "runtime": stage(&cpu_samples, |s| s.runtime),
        "postprocess": stage(&cpu_samples, |s| s.postprocess), "uploadAndFence": stage(&cpu_samples, |s| s.upload)});
    report["countersAfterSamples"] = counters(after);
    report["sampleCounterDelta"] = json!({"submissions": after.submissions - before.submissions,
        "computeDispatches": after.compute_dispatches - before.compute_dispatches,
        "hostImageBytes": after.host_image_bytes - before.host_image_bytes});
    if after.host_image_bytes != 0 { return Err("resident host image counter must remain zero".into()); }
    if after.submissions - before.submissions != args.samples as u64 {
        return Err("resident submission counter does not match measured GPU samples".into());
    }
    let final_output = case.gpu.submit_texture(&case.source, args.width, args.height)?;
    let final_submission = final_output.submission_index().clone();
    let final_dimensions = (final_output.width(), final_output.height());
    drop(case);
    drop(gpu);
    wait(&device, final_submission)?;
    if (final_output.texture().width(), final_output.texture().height()) != final_dimensions {
        return Err("consumer output lease did not survive session destruction".into());
    }
    report["resourceLifecycle"]["sessionDropRetainsOutputLease"] = json!(true);
    drop(final_output);
    Ok(())
}
fn main() -> Result<()> {
    let args = Args::parse()?;
    let mut report = json!({"status": "running", "modelId": args.model_id, "modelPath": args.model_path,
        "source": [args.width, args.height], "samplesPerPath": args.samples, "warmupPerPath": args.warmup,
        "thresholds": {"score": SCORE, "iou": IOU}, "parity": {},
        "gpuExecutionTimeMs": null,
        "method": "One artifact and source size per process; serial single-frame completion latency; up to three alternating path-only blocks. CPU=preallocated source staging/readback + actual image task preprocessing/default ORT CPU/postprocessing + preallocated texture upload and submission completion fence. GPU=resident submit + actual submission completion wait + release output lease. Source generation, preparation and diagnostic reads excluded. Completion wall latency, not GPU-only execution time or application FPS.",
        "methodFlags": {"sameMetalDeviceAndQueue": true, "sourceGeneratedOnGpu": true, "sourceCopyDst": false,
            "fp32OriginalGraph": true, "cpuOrtDefaults": true, "runtimeGpuFallback": false,
            "preallocatedCpuStagingAndUploadTexture": true, "verificationOutsideMeasurements": true,
            "verificationEveryOutputByte": true, "rawModelVerificationEveryValue": true,
            "gpuExecutionTimeMeasured": false, "boundedWaitSeconds": 120, "releaseBuild": !cfg!(debug_assertions)},
        "scopeLimitations": ["Deterministic GPU-generated gradient only; no decoding, UI, display, or concurrent benchmark jobs",
            "Single-frame serial completion latency, not pipelined throughput",
            "Gradient detections can be empty; nonempty fixture coverage must be established separately",
            "CPU stage metrics exclude some wrapper overhead; upload includes the completion fence"]});
    let result = run(&args, &mut report);
    report["status"] = json!(if result.is_ok() { "passed" } else { "failed" });
    if let Err(error) = &result { report["error"] = json!(error.to_string()); }
    // Preserve raw parity statistics before returning a failing process exit status.
    std::fs::write(&args.report_path, serde_json::to_vec_pretty(&report)?)?;
    println!("Wrote {} ({})", args.report_path, report["status"]);
    result
}
