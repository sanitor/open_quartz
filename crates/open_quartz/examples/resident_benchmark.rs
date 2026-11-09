//! Compare serial CPU-transfer and resident Metal SR3 completion latency.
//! Usage: resident_benchmark MODEL.onnx REPORT.json [--samples N] [--warmup N]
use open_quartz::inference::{
    run_image_task_with_request_and_metrics, InferenceImageMetrics, InferenceSession,
    InferenceTask, RequestStamp,
};
use oq_inference::{gpu::GpuImageSession, gpu_model::GpuModel};
use serde_json::{json, Value};
use std::{error::Error, sync::{mpsc, Arc}, time::{Duration, Instant}};

type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;
const TIMEOUT: Duration = Duration::from_secs(120);
const MODEL_HASH: &str = "85f36ff88cc504a24af5e0602148bc56a8aa09a58eca8c0da2756f3e8186035e";

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
                label: Some("benchmark reusable readback"), size: u64::from(row) * u64::from(height),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
            rgba: vec![0; width as usize * height as usize * 4], width, height, row,
        }
    }
    fn read(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Result<()> {
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &self.staging, layout: wgpu::TexelCopyBufferLayout {
                offset: 0, bytes_per_row: Some(self.row), rows_per_image: Some(self.height),
            } },
            extent(self.width, self.height),
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

fn extent(width: u32, height: u32) -> wgpu::Extent3d {
    wgpu::Extent3d { width, height, depth_or_array_layers: 1 }
}
fn texture(device: &wgpu::Device, width: u32, height: u32, usage: wgpu::TextureUsages) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("benchmark RGBA8"), size: extent(width, height), mip_level_count: 1,
        sample_count: 1, dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm, usage, view_formats: &[],
    })
}
fn generate(device: &wgpu::Device, queue: &wgpu::Queue, source: &wgpu::Texture) -> Result<()> {
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

#[derive(Clone, Copy)]
struct CpuSample {
    total: f64, readback: f64, preprocess: f64, runtime: f64, postprocess: f64, upload: f64,
}
fn ms(start: Instant) -> f64 { start.elapsed().as_secs_f64() * 1000.0 }

struct Case<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    source: wgpu::Texture,
    cpu_output: wgpu::Texture,
    input: Readback,
    cpu: &'a mut InferenceSession,
    gpu: &'a mut GpuImageSession,
    request_id: u64,
}
impl Case<'_> {
    fn cpu(&mut self) -> Result<CpuSample> {
        let start = Instant::now();
        self.input.read(self.device, self.queue, &self.source)?;
        let readback = ms(start);
        self.request_id += 1;
        let stamp = RequestStamp { request_id: self.request_id, session_generation: self.cpu.generation() };
        let (output, metrics) = run_image_task_with_request_and_metrics(
            self.cpu, stamp,
            InferenceTask::SuperResolution, "super-resolution-3x", &self.input.rgba,
            self.input.width, self.input.height, 224, 0.25, 0.45,
        )?;
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
        drop(output);
        Ok(ms(start))
    }
    fn verify(&mut self) -> Result<Value> {
        self.cpu()?;
        let output = self.gpu.submit_texture(&self.source, self.input.width, self.input.height)?;
        wait(self.device, output.submission_index().clone())?;
        let mut cpu = Readback::new(self.device, output.width(), output.height());
        let mut gpu = Readback::new(self.device, output.width(), output.height());
        cpu.read(self.device, self.queue, &self.cpu_output)?;
        gpu.read(self.device, self.queue, output.texture())?;
        let mut max = 0u8;
        let mut sum = 0u64;
        for (&a, &b) in cpu.rgba.iter().zip(&gpu.rgba) {
            let difference = a.abs_diff(b);
            max = max.max(difference);
            sum += u64::from(difference);
        }
        let mean = sum as f64 / cpu.rgba.len() as f64;
        if max > 3 || mean > 0.5 { return Err(format!("output mismatch: max={max}, mean={mean}").into()); }
        Ok(json!({ "checkedBytes": cpu.rgba.len(), "maxByteDifference": max, "meanByteDifference": mean,
            "maxAllowed": 3, "meanAllowed": 0.5, "explicitReadbackOutsideMeasurements": true }))
    }
}

fn stats(samples: &[f64]) -> Value {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    let mean = samples.iter().sum::<f64>() / n as f64;
    // Retain the baseline's lower nearest-rank median and P95 convention.
    json!({ "n": n, "meanMs": mean, "p50Ms": sorted[((n as f64 * 0.5).ceil() as usize).saturating_sub(1)],
        "p95Ms": sorted[((n as f64 * 0.95).ceil() as usize).saturating_sub(1)],
        "minMs": sorted[0], "maxMs": sorted[n - 1], "serialEquivalentFps": 1000.0 / mean })
}
fn stage(samples: &[CpuSample], select: fn(&CpuSample) -> f64) -> Value {
    let values: Vec<_> = samples.iter().map(select).collect();
    json!({ "summary": stats(&values), "samplesMs": values })
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let model_path = args.next().ok_or("Usage: resident_benchmark MODEL.onnx REPORT.json [--samples N] [--warmup N]")?;
    let report_path = args.next().ok_or("Missing REPORT.json")?;
    let mut samples = 60usize;
    let mut warmup = 10usize;
    while let Some(flag) = args.next() {
        let value = args.next().ok_or("Missing option value")?.parse::<usize>()?;
        match flag.as_str() { "--samples" => samples = value, "--warmup" => warmup = value,
            _ => return Err(format!("Unknown option: {flag}").into()) }
    }
    if samples < 3 { return Err("--samples must be at least 3".into()); }
    let bytes = std::fs::read(&model_path)?;
    let model = GpuModel::from_onnx(&bytes)?;
    if model.plan.source_sha256 != MODEL_HASH {
        return Err(format!("Expected original validated SR3 SHA256 {MODEL_HASH}, got {}", model.plan.source_sha256).into());
    }
    let model_hash = model.plan.source_sha256.clone();
    let mut cpu = InferenceSession::from_memory(&bytes)?;
    let cpu_plan = serde_json::to_value(cpu.plan())?;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL, ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))?;
    let info = adapter.get_info();
    if info.backend != wgpu::Backend::Metal || info.device_type == wgpu::DeviceType::Cpu {
        return Err("Hardware Metal adapter required; fallback forbidden".into());
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("shared benchmark Metal device"),
        ..Default::default()
    }))?;
    let (device, queue) = (Arc::new(device), Arc::new(queue));
    let mut gpu = GpuImageSession::prepare(device.clone(), queue.clone(), model)?;
    let mut cases = Vec::with_capacity(3);
    for (width, height) in [(224, 224), (640, 360), (1280, 720)] {
        let source = texture(&device, width, height, wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING);
        generate(&device, &queue, &source)?;
        let cpu_output = texture(&device, width * 3, height * 3,
            wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING);
        let mut case = Case { device: &device, queue: &queue, source, cpu_output,
            input: Readback::new(&device, width, height), cpu: &mut cpu, gpu: &mut gpu, request_id: 0 };
        let verification = case.verify()?;
        for _ in 0..warmup { case.cpu()?; case.gpu()?; }
        let mut cpu_samples = Vec::with_capacity(samples);
        let mut gpu_samples = Vec::with_capacity(samples);
        let mut blocks = Vec::with_capacity(3);
        for block in 0..3 {
            let count = samples / 3 + usize::from(block < samples % 3);
            let cpu_begin = cpu_samples.len();
            let gpu_begin = gpu_samples.len();
            // Alternate which path runs first to avoid always giving one path the same order.
            for path in 0..2 {
                if (block + path) % 2 == 0 {
                    for _ in 0..count { cpu_samples.push(case.cpu()?); }
                } else {
                    for _ in 0..count { gpu_samples.push(case.gpu()?); }
                }
            }
            let cpu_block: Vec<_> = cpu_samples[cpu_begin..].iter().map(|sample| sample.total).collect();
            blocks.push(json!({ "old": stats(&cpu_block), "new": stats(&gpu_samples[gpu_begin..]),
                "order": if block % 2 == 0 { "cpu,gpu" } else { "gpu,cpu" } }));
        }
        let cpu_totals: Vec<_> = cpu_samples.iter().map(|sample| sample.total).collect();
        let counters = case.gpu.counters();
        if counters.host_image_bytes != 0 { return Err("Resident host image counter must remain zero".into()); }
        cases.push(json!({ "source": [width, height], "output": [width * 3, height * 3],
            "networkInput": [224,224], "networkOutput": [672,672], "blocks": blocks,
            "cpuRoundtrip": stats(&cpu_totals), "resident": stats(&gpu_samples),
            "cpuSamplesMs": cpu_totals, "gpuSamplesMs": gpu_samples,
            "cpuStages": { "readback": stage(&cpu_samples, |s| s.readback),
                "preprocess": stage(&cpu_samples, |s| s.preprocess), "runtime": stage(&cpu_samples, |s| s.runtime),
                "postprocess": stage(&cpu_samples, |s| s.postprocess), "uploadAndFence": stage(&cpu_samples, |s| s.upload) },
            "verification": verification,
            "gpuExecutionTimeMs": null,
            "gpuExecutionTimeReason": "not measured; external Metal marker counters were stale/nonmonotonic in qualification",
            "residentHostImageBytes": counters.host_image_bytes,
            "residentCumulativeSubmissions": counters.submissions }));
    }
    let report = json!({ "adapter": format!("{info:?}"), "deviceName": info.name,
        "modelSha256": model_hash, "modelPath": model_path, "cpuPlan": cpu_plan,
        "samplesPerPathPerSize": samples, "warmupPerPathPerSize": warmup,
        "residentHostImageBytes": gpu.counters().host_image_bytes, "cases": cases,
        "method": "Release; sequential single-frame completion latency; three alternating blocks; old=preallocated staging GPU readback + existing CPU image pipeline/ORT defaults + upload to preallocated texture/empty submit/fence; new=resident submit + actual submission completion wait + release lease. Source generation, model preparation, verification, UI and decoding excluded. Submission-completion wall latency is authoritative; GPU execution time is not measured. Serial equivalent FPS is not UI FPS.",
        "methodFlags": { "sameMetalDeviceAndQueue": true, "sourceGeneratedOnGpu": true,
            "sourceCopyDst": false, "fp32": true, "cpuOrtDefaults": true,
            "runtimeGpuFallback": false, "preallocatedCpuStagingAndUploadTexture": true,
            "verificationEveryOutputByte": true, "verificationOutsideMeasurements": true,
            "gpuExecutionTimeMeasured": false, "boundedWaitSeconds": 120,
            "releaseBuild": !cfg!(debug_assertions) },
        "scopeLimitations": ["Single-frame serial completion latency, not pipelined throughput or application FPS",
            "GPU-generated deterministic gradient; no video decoding, display or UI costs",
            "Network always 224x224 -> 672x672; output source*3 uses reconstruction/resizing",
            "CPU uses retained current reference pipeline and default ORT session, not a historical binary",
            "CPU stage metrics do not partition all wrapper overhead; upload stage includes completion fence",
            "GPU execution time is not measured; external Metal marker counters were stale/nonmonotonic in qualification"] });
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    println!("Wrote {report_path}");
    Ok(())
}
