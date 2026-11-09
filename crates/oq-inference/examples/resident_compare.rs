//! GPU-generated image -> full resident network -> GPU output. CPU mapping occurs
//! only at the explicit reference-verification boundary after submission counters.
use oq_inference::{gpu::{GpuError, GpuImageSession}, gpu_model::GpuModel, InferenceSession};
use std::{error::Error, path::PathBuf, sync::Arc, time::{Duration, Instant}};
fn main() -> Result<(), Box<dyn Error>> { pollster::block_on(run()) }
async fn run() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let model_path = PathBuf::from(args.next().ok_or("expected selected SR ONNX path")?);
    let report_dir = PathBuf::from(args.next().ok_or("expected report directory")?);
    if args.next().is_some() { return Err("unexpected arguments".into()); }
    let bytes = std::fs::read(model_path)?;
    let compiled = GpuModel::from_onnx(&bytes)?;
    let hash = compiled.plan.source_sha256.clone();
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::METAL, ..wgpu::InstanceDescriptor::new_without_display_handle() });
    let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions::default()).await?;
    let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor::default()).await?;
    let (device, queue) = (Arc::new(device), Arc::new(queue));
    let mut session = GpuImageSession::prepare(device.clone(), queue.clone(), compiled)?;
    let input = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("resident acceptance GPU source"), size: wgpu::Extent3d { width: 8, height: 8, depth_or_array_layers: 1 },
        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[],
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("GPU gradient"), source: wgpu::ShaderSource::Wgsl(r#"
        @group(0) @binding(0) var image: texture_storage_2d<rgba8unorm,write>;
        @compute @workgroup_size(8,8) fn main(@builtin(global_invocation_id) p:vec3u) {
            textureStore(image,vec2i(p.xy),vec4f(f32(p.x*32u)/255.0,f32(p.y*32u)/255.0,80.0/255.0,1.0));
        }"#.into()) });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("GPU source"), layout: None, module: &shader, entry_point: Some("main"), compilation_options: Default::default(), cache: None });
    let view = input.create_view(&Default::default());
    let binding = device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &pipeline.get_bind_group_layout(0), entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }] });
    let mut encoder = device.create_command_encoder(&Default::default());
    { let mut pass = encoder.begin_compute_pass(&Default::default()); pass.set_pipeline(&pipeline); pass.set_bind_group(0,&binding,&[]); pass.dispatch_workgroups(1,1,1); }
    queue.submit([encoder.finish()]);
    let started = Instant::now();
    let output = session.submit_texture(&input,8,8)?;
    let counters = session.counters();
    assert_eq!(counters.host_image_bytes,0);
    assert_eq!(counters.submissions,1);
    assert_eq!(counters.compute_dispatches,6);
    assert_eq!((output.width(),output.height()),(24,24));
    let row = 256u32;
    let staging = device.create_buffer(&wgpu::BufferDescriptor { label: Some("explicit verification only"), size: row as u64 * 24, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(wgpu::TexelCopyTextureInfo { texture: output.texture(), mip_level:0, origin:wgpu::Origin3d::ZERO, aspect:wgpu::TextureAspect::All }, wgpu::TexelCopyBufferInfo { buffer:&staging,layout:wgpu::TexelCopyBufferLayout { offset:0,bytes_per_row:Some(row),rows_per_image:Some(24) } },wgpu::Extent3d {width:24,height:24,depth_or_array_layers:1});
    let submitted = queue.submit([encoder.finish()]);
    let (sender,receiver)=std::sync::mpsc::channel(); staging.map_async(wgpu::MapMode::Read,..,move|result|{let _=sender.send(result);});
    device.poll(wgpu::PollType::Wait { submission_index:Some(submitted),timeout:Some(Duration::from_secs(120)) })?;
    receiver.recv()??;
    let mapped=staging.get_mapped_range(..)?;
    let actual:Vec<u8> = mapped.chunks_exact(row as usize).flat_map(|row|row[..96].iter().copied()).collect();
    drop(mapped);staging.unmap();
    let first_completed_wall_ms=started.elapsed().as_secs_f64()*1000.0;
    // Independent ORT baseline; these host tensors are intentionally outside production.
    let mut reference=InferenceSession::from_memory(&bytes)?;
    let mut luma=vec![0.0f32;224*224];let mut cb=vec![0.0f32;224*224];let mut cr=vec![0.0f32;224*224];
    for y in 0..224 {for x in 0..224 {let i=y*224+x;let r=(x*8/224*32)as f32/255.0;let g=(y*8/224*32)as f32/255.0;let b=80.0/255.0;luma[i]=0.299*r+0.587*g+0.114*b;cb[i]=-0.169*r-0.331*g+0.5*b+0.5;cr[i]=0.5*r-0.419*g-0.081*b+0.5;}}
    let result=reference.run_f32(luma,vec![1,1,224,224])?;
    let mut expected=vec![0u8;24*24*4];
    for y in 0..24 {for x in 0..24 {let mx=x*672/24;let my=y*672/24;let ci=(my*224/672)*224+mx*224/672;let yy=result.data[my*672+mx];let u=cb[ci]-0.5;let v=cr[ci]-0.5;let rgb=[yy+1.402*v,yy-0.344*u-0.714*v,yy+1.772*u];let i=(y*24+x)*4;for c in 0..3 {expected[i+c]=(rgb[c].clamp(0.0,1.0)*255.0).round()as u8;}expected[i+3]=255;}}
    let max_error=actual.iter().zip(&expected).map(|(&a,&b)|a.abs_diff(b)).max().unwrap();
    let mean_error=actual.iter().zip(&expected).map(|(&a,&b)|f64::from(a.abs_diff(b))).sum::<f64>()/expected.len()as f64;
    if max_error>3||mean_error>0.5 {return Err(format!("GPU reference mismatch max={max_error} mean={mean_error}").into());}
    // Native cross-device inputs cannot reach an inference submission.
    let (other_device,_)=adapter.request_device(&wgpu::DeviceDescriptor::default()).await?;
    let foreign=other_device.create_texture(&wgpu::TextureDescriptor {label:None,size:wgpu::Extent3d{width:8,height:8,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8Unorm,usage:wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[]});
    assert!(session.bind_input(&foreign,8,8).is_err());assert_eq!(session.counters().submissions,1);
    let second=session.submit_texture(&input,8,8)?;let third=session.submit_texture(&input,8,8)?;
    assert!(matches!(session.submit_texture(&input,8,8),Err(GpuError::Capacity)));assert_eq!(session.counters().submissions,3);
    let released=second.texture_arc();drop(second);let replacement=session.submit_texture(&input,8,8)?;assert_eq!(replacement.texture(),released.as_ref());
    // Retained leases remain valid until all verification/consumption work is queued.
    drop((output,third,replacement));device.poll(wgpu::PollType::Wait {submission_index:None,timeout:Some(Duration::from_secs(120))})?;
    std::fs::create_dir_all(&report_dir)?;
    let report=serde_json::json!({"modelSha256":hash,"adapter":format!("{:?}",adapter.get_info()),"source":"GPU compute; texture has no COPY_DST usage","output":[24,24],"maxByteError":max_error,"meanByteError":mean_error,"hostImageBytesBeforeVerification":counters.host_image_bytes,"dispatchesPerInference":counters.compute_dispatches,"weightUploadBytes":counters.weight_upload_bytes,"scratchBytes":counters.scratch_bytes,"firstCompletedWallMsIncludingDiagnosticCopy":first_completed_wall_ms,"wrongDeviceRejectedBeforeSubmit":true,"outputPoolBound":3,"scope":"Only explicit verification maps pixels; production pre/infer/post are GPU compute on one Metal queue"});
    std::fs::write(report_dir.join("resident-comparison.json"),serde_json::to_vec_pretty(&report)?)?;
    println!("{}",serde_json::to_string_pretty(&report)?);Ok(())
}
