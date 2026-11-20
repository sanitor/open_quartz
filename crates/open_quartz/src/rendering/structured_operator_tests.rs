use super::*;
use crate::evaluation::{FrameInputs, GraphEvaluator};
use crate::operators::{evaluate_color_corrector, ColorCorrectorConfig, ColorDomain};
use crate::rendering::RenderDevice;
use serde_json::json;

async fn hardware_backend() -> Arc<GpuBackend> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false, compatible_surface: None, apply_limit_buckets: false,
    }).await.expect("Corrector GPU oracle requires a hardware adapter");
    assert_ne!(adapter.get_info().device_type, wgpu::DeviceType::Cpu, "software adapters cannot satisfy the Corrector oracle");
    let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor::default()).await.unwrap();
    Arc::new(GpuBackend::from_device(device, queue))
}

fn graph(configs: &[(&str, &str, ColorCorrectorConfig)], renderer: bool) -> Graph {
    let mut nodes = vec![json!({"id":"source","type":"input","position":{"x":0,"y":0},"data":{
        "type":"input","label":"sRGB source","shaderCode":"","inputs":[],"outputs":[{"id":"source_out","label":"output","dataType":"sampler2D","direction":"output"}],
        "uniforms":{},"inputMode":"image","inputDataType":"sampler2D","imageWidth":4,"imageHeight":1}})];
    let mut edges = Vec::new();
    for (id, source, config) in configs {
        nodes.push(json!({"id":id,"type":"colorcorrector","position":{"x":0,"y":0},"data":{
            "type":"colorcorrector","label":"Corrector","shaderCode":"","inputs":[{"id":format!("{id}_in"),"label":"source","dataType":"sampler2D","direction":"input"}],
            "outputs":[{"id":format!("{id}_out"),"label":"result","dataType":"sampler2D","direction":"output"}],"uniforms":{},"colorCorrector":config}}));
        edges.push(json!({"id":format!("{id}_edge"),"source":source,"sourceHandle":format!("{source}_out"),"target":id,"targetHandle":format!("{id}_in")}));
    }
    if renderer {
        let source = configs.last().unwrap().0;
        nodes.push(json!({"id":"renderer","type":"renderer","position":{"x":0,"y":0},"data":{
            "type":"renderer","label":"SDR renderer","shaderCode":"","inputs":[{"id":"renderer_in","label":"inputTexture","dataType":"sampler2D","direction":"input"}],"outputs":[],"uniforms":{}}}));
        edges.push(json!({"id":"renderer_edge","source":source,"sourceHandle":format!("{source}_out"),"target":"renderer","targetHandle":"renderer_in"}));
    }
    let graph: Graph = serde_json::from_value(json!({"nodes":nodes,"edges":edges})).unwrap();
    graph.validate().unwrap();
    graph
}

fn inputs(frame: u64) -> FrameInputs {
    FrameInputs { time: 0.0, delta: 0.0, frame, date: [0.0;4], mouse: [0.0;4], resolution: [4.0,1.0,1.0], video_nodes: Vec::new() }
}

fn evaluate(executor: &mut GpuExecutor, evaluator: &mut GraphEvaluator, frame: u64) {
    let work = evaluator.run_frame(&inputs(frame));
    executor.execute(evaluator.plan(), &work).unwrap();
}

fn read_half(executor: &GpuExecutor, id: &str) -> Vec<[f32;4]> {
    let output = executor.output_handle(id).unwrap();
    assert_eq!(output.format, TextureFormat::Rgba16Float);
    let buffer = executor.backend.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("corrector-oracle"), size: 256, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false,
    });
    let mut encoder = executor.backend.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(output.texture.as_image_copy(), wgpu::TexelCopyBufferInfo {
        buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256), rows_per_image: Some(1) },
    }, wgpu::Extent3d { width: output.width, height: 1, depth_or_array_layers: 1 });
    executor.backend.queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| { tx.send(result).unwrap(); });
    executor.backend.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    let mapped = buffer.slice(..).get_mapped_range().unwrap();
    let result = mapped[..output.width as usize * 8].chunks_exact(8).map(|pixel| {
        std::array::from_fn(|index| {
            let bits = u16::from_le_bytes(pixel[index*2..index*2+2].try_into().unwrap());
            let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
            let exponent = (bits >> 10) & 31;
            let fraction = f32::from(bits & 1023) / 1024.0;
            sign * if exponent == 0 { fraction * 2.0_f32.powi(-14) } else { (1.0 + fraction) * 2.0_f32.powi(i32::from(exponent)-15) }
        })
    }).collect();
    drop(mapped);
    buffer.unmap();
    result
}

const PIXELS: [u8;16] = [0,0,0,64, 128,64,200,128, 255,255,255,255, 32,180,96,32];
fn assert_oracle(actual: &[[f32;4]], config: &ColorCorrectorConfig, source: &[[f32;4]], domain: ColorDomain) {
    for (actual, source) in actual.iter().zip(source) {
        let expected = evaluate_color_corrector(config, domain, *source).unwrap();
        for (actual, expected) in actual.iter().zip(expected) {
            assert!((actual-expected).abs() <= 0.006 * expected.abs().max(1.0), "GPU {actual} != independent oracle {expected}");
        }
    }
}
fn source_pixels() -> Vec<[f32;4]> {
    PIXELS.chunks_exact(4).map(|pixel| std::array::from_fn(|index| f32::from(pixel[index])/255.0)).collect()
}

#[test]
fn corrector_gpu_signed_fp16_oracle_chaining_and_explicit_sdr_boundary() {
    pollster::block_on(async {
        let backend = hardware_backend().await;
        let mut config = ColorCorrectorConfig::default();
        config.primaries.exposure = 2.0;
        config.primaries.offset = [-0.5,0.25,0.5];
        config.primaries.white_balance = [0.1,-0.05];
        config.hdr_zones.blacks.exposure = 0.3;
        config.hdr_zones.highlights.saturation = 0.8;
        let mut second = ColorCorrectorConfig::default();
        second.primaries.exposure = -0.5;
        second.primaries.saturation = 1.2;
        let graph = graph(&[("first","source",config.clone()),("second","first",second.clone())], true);
        let mut evaluator = GraphEvaluator::prepare_with_options(graph.nodes, graph.edges, false);
        assert!(evaluator.plan().nodes.iter().filter(|node| node.node_type == crate::project::NodeType::ColorCorrector).all(|node| node.shader.is_none()));
        let mut executor = GpuExecutor::new(backend.clone());
        executor.upload_rgba("source", &PIXELS, 4, 1).unwrap();
        evaluate(&mut executor, &mut evaluator, 1);
        let first = read_half(&executor,"first");
        assert_oracle(&first,&config,&source_pixels(),ColorDomain::LegacySrgb);
        let mut preview = crate::rendering::GpuPreviewReader::new(backend.clone());
        assert!(preview.scale(&executor.output_handle("first").unwrap(),4).is_err(), "AP1 must never enter a legacy viewer without a display boundary");
        assert!(preview.scale_managed(&executor.output_handle("source").unwrap(),4).is_err(), "sRGB bytes are not AP1");
        assert!(first.iter().flatten().any(|v| *v > 1.0));
        assert!(first.iter().flatten().any(|v| *v < 0.0));
        let second_pixels = read_half(&executor,"second");
        assert_oracle(&second_pixels,&second,&first,ColorDomain::AcesCgLinear);
        assert_eq!(executor.output_domain("second"),ColorDomain::AcesCgLinear);
        assert_eq!(executor.output_handle("renderer").unwrap().format,TextureFormat::Rgba8Unorm);
        let display = executor.read_output_rgba("renderer").await.unwrap();
        assert_eq!(display.len(),16);
        assert!(display[0] < 4 && display[2] > 100, "SDR boundary must convert AP1 and clamp signed display values");
        assert_eq!(display[3],64, "alpha is neither graded nor transfer encoded");
    });
}

#[test]
fn corrector_value_edits_share_pipeline_keep_targets_and_isolate_instances() {
    pollster::block_on(async {
        let backend = hardware_backend().await;
        let mut a = ColorCorrectorConfig::default();
        a.primaries.exposure=1.0;
        let mut b = a.clone();
        b.primaries.exposure=2.0;
        let mut graph = graph(&[("a","source",a.clone()),("b","source",b.clone())], false);
        let mut evaluator = GraphEvaluator::prepare_with_options(graph.nodes.clone(),graph.edges.clone(),false);
        let mut executor=GpuExecutor::new(backend.clone());
        executor.upload_rgba("source",&PIXELS,4,1).unwrap();
        evaluate(&mut executor,&mut evaluator,1);
        let initial=backend.runtime_stats();
        assert_eq!(initial.pipeline_creations,1);
        assert_eq!(initial.pipeline_cache_hits,1);
        assert_eq!(initial.target_allocations,2);
        assert_oracle(&read_half(&executor,"a"),&a,&source_pixels(),ColorDomain::LegacySrgb);
        assert_oracle(&read_half(&executor,"b"),&b,&source_pixels(),ColorDomain::LegacySrgb);
        let target=executor.output_handle("a").unwrap();
        let source=executor.output_handle("source").unwrap();
        let program=evaluator.plan().nodes.iter().find(|n|n.id=="a").unwrap().structured.as_ref().unwrap().program.as_ref().unwrap().clone();
        for frame in 2..10 {
            a.primaries.exposure=frame as f32/5.0;
            graph.nodes.iter_mut().find(|n|n.id=="a").unwrap().data.color_corrector=Some(a.clone());
            evaluator.replace_graph_preserving_state(graph.nodes.clone(),graph.edges.clone(),&HashSet::from(["a".into()]));
            let next=evaluator.plan().nodes.iter().find(|n|n.id=="a").unwrap().structured.as_ref().unwrap().program.as_ref().unwrap();
            assert!(Arc::ptr_eq(&program,next),"slider edits retain immutable WGSL artifact");
            evaluate(&mut executor,&mut evaluator,frame);
            assert!(Arc::ptr_eq(&target.texture,&executor.output_handle("a").unwrap().texture));
        }
        let stats=backend.runtime_stats();
        assert_eq!(stats.pipeline_creations,initial.pipeline_creations);
        assert_eq!(stats.target_allocations,initial.target_allocations);
        assert_eq!(stats.pipeline_cache_entries,1);
        assert!(Arc::ptr_eq(&source.texture,&executor.output_handle("source").unwrap().texture));
        assert_eq!(stats.uniform_writes,initial.uniform_writes+8,"each slider edit uploads only the changed vec4");
        evaluator.mark_dirty("a");
        evaluate(&mut executor,&mut evaluator,10);
        assert_eq!(backend.runtime_stats().uniform_writes,stats.uniform_writes,"unchanged video-frame parameters are not repacked/uploaded");
        assert_oracle(&read_half(&executor,"a"),&a,&source_pixels(),ColorDomain::LegacySrgb);
        assert_oracle(&read_half(&executor,"b"),&b,&source_pixels(),ColorDomain::LegacySrgb);
        let preserved=executor.output_handle("a").unwrap();
        graph.nodes.iter_mut().find(|n|n.id=="a").unwrap().data.color_corrector.as_mut().unwrap().algorithm_version=99;
        let invalid=crate::compiler::build_execution_plan(&graph.nodes,&graph.edges);
        assert!(executor.sync_plan(&invalid).is_err());
        assert!(Arc::ptr_eq(&preserved.texture,&executor.output_handle("a").unwrap().texture));
        assert_eq!(backend.runtime_stats().pipeline_creations,stats.pipeline_creations);
    });
}

#[test]
fn corrector_bypass_alias_chains_keep_logical_identity_and_source_lease_without_draws() {
    pollster::block_on(async {
        let backend=hardware_backend().await;
        let mut bypass=ColorCorrectorConfig::default();
        bypass.enabled=false;
        bypass.primaries.exposure=3.0;
        let graph=graph(&[("a","source",bypass.clone()),("b","a",bypass)],true);
        let mut evaluator=GraphEvaluator::prepare_with_options(graph.nodes,graph.edges,false);
        let mut executor=GpuExecutor::new(backend.clone());
        executor.upload_rgba("source",&PIXELS,4,1).unwrap();
        let mut source=executor.output_handle("source").unwrap();
        let lease: Arc<dyn std::any::Any+Send+Sync>=Arc::new(123_u64);
        source.lease=Some(lease.clone());
        executor.register_external_texture("source",source.clone()).unwrap();
        evaluate(&mut executor,&mut evaluator,1);
        for id in ["a","b","renderer"] {
            let output=executor.output_handle(id).unwrap();
            assert!(Arc::ptr_eq(&output.texture,&source.texture));
            assert!(Arc::ptr_eq(output.lease.as_ref().unwrap(),&lease));
            assert_eq!(executor.read_output_rgba(id).await.unwrap(),PIXELS);
        }
        let stats=backend.runtime_stats();
        assert_eq!((stats.draws,stats.operator_draws,stats.target_allocations,stats.pipeline_creations),(0,0,0,0));
        assert!(executor.nodes.is_empty());
        let pool=crate::rendering::StableFramePool::new(backend,crate::rendering::StableFrameBudget::default()).unwrap();
        let frame=pool.capture(executor.output_handle("b").unwrap(),crate::outputs::FrameSnapshotIdentity {
            output_node_id:"b".into(),graph_revision:7,evaluation_stamp:None,content_stamp:None,
            color_space:crate::outputs::FrameColorSpace::Srgb,alpha_mode:crate::outputs::FrameAlphaMode::Straight,
        }).unwrap();
        assert_eq!(frame.identity.output_node_id,"b");
        assert_eq!(frame.identity.graph_revision,7);
        frame.ready.wait(std::time::Duration::from_secs(5)).unwrap();
    });
}

#[test]
fn stage_variants_use_bounded_lru_and_retained_pipeline_survives_eviction() {
    pollster::block_on(async {
        let backend=hardware_backend().await;
        backend.pipeline_cache.lock().set_test_capacity(2);
        let mut executor=GpuExecutor::new(backend.clone());
        executor.upload_rgba("source",&PIXELS,4,1).unwrap();
        let mut retained=None;
        for (index,(primaries,zones)) in [(true,false),(false,true),(true,true),(true,false)].into_iter().enumerate() {
            let mut config=ColorCorrectorConfig::default();
            config.primaries.enabled=primaries;
            config.hdr_zones.enabled=zones;
            config.primaries.exposure=1.0;
            config.hdr_zones.shadows.exposure=0.5;
            let graph=graph(&[("a","source",config.clone())],false);
            let mut evaluator=GraphEvaluator::prepare_with_options(graph.nodes,graph.edges,false);
            evaluate(&mut executor,&mut evaluator,index as u64+1);
            assert_oracle(&read_half(&executor,"a"),&config,&source_pixels(),ColorDomain::LegacySrgb);
            if index==0 { retained=Some(executor.nodes["a"].clone()); }
            assert!(backend.runtime_stats().pipeline_cache_entries<=2);
        }
        assert_eq!(backend.runtime_stats().pipeline_creations,4,"old variant must actually be evicted");
        assert_eq!(backend.runtime_stats().target_allocations,1,"stage changes retain compatible FP16 target");
        let retained=retained.unwrap();
        let mut command=crate::evaluation::ExecutionCommand {
            node_id:"a".into(),kind:"operator".into(),output_port_id:None,texture_inputs:BTreeMap::from([("source".into(),"source".into())]),
            uniforms:BTreeMap::new(),target_width:Some(4),target_height:Some(1),feedback_read_index:None,feedback_write_index:None,clear_feedback:false,scalar_output:None,
        };
        let mut config=ColorCorrectorConfig::default();
        config.primaries.exposure=1.0;
        config.hdr_zones.enabled=false;
        command.uniforms=config.parameter_values().unwrap();
        executor.nodes.insert("a".into(),retained);
        let mut encoder=backend.device.create_command_encoder(&Default::default());
        executor.encode_shader(&mut encoder,&command,None).unwrap();
        backend.queue.submit([encoder.finish()]);
        assert_oracle(&read_half(&executor,"a"),&config,&source_pixels(),ColorDomain::LegacySrgb);
    });
}

#[test]
fn legacy_shader_helpers_sample_managed_inputs_through_explicit_srgb_boundary() {
    pollster::block_on(async {
        let backend=hardware_backend().await;
        let mut config=ColorCorrectorConfig::default();
        config.primaries.exposure=1.0;
        let mut graph=graph(&[("a","source",config.clone())],false);
        graph.nodes.push(serde_json::from_value(json!({"id":"legacy","type":"shader","position":{"x":0,"y":0},"data":{
            "type":"shader","label":"legacy helper","shaderCode":"fn oq_ap1_to_srgb(value: vec4f) -> vec4f { return value; } fn cc_srgb_encode(value: vec4f) -> vec4f { return value; } fn sample_image(t: texture_2d<f32>, s: sampler, uv: vec2f) -> vec4f { return textureSampleLevel /* ) /* nested ( */ ) */ (t,s,uv + vec2f(0.0),max(0.0,0.0)); } @fragment fn main(@location(0) uv:vec2f) -> @location(0) vec4f { return oq_ap1_to_srgb(cc_srgb_encode(sample_image(image,imageSampler,uv))); }",
            "inputs":[{"id":"legacy_in","label":"image","dataType":"sampler2D","direction":"input"}],
            "outputs":[{"id":"legacy_out","label":"output","dataType":"sampler2D","direction":"output"}],"uniforms":{}}})).unwrap());
        graph.edges.push(serde_json::from_value(json!({"id":"legacy_edge","source":"a","sourceHandle":"a_out","target":"legacy","targetHandle":"legacy_in"})).unwrap());
        graph.validate().unwrap();
        let mut evaluator=GraphEvaluator::prepare_with_options(graph.nodes,graph.edges,false);
        let mut executor=GpuExecutor::new(backend);
        executor.upload_rgba("source",&PIXELS,4,1).unwrap();
        evaluate(&mut executor,&mut evaluator,1);
        let managed=read_half(&executor,"a");
        let pixels=executor.read_output_rgba("legacy").await.unwrap();
        let matrix=[[1.7048586763,-0.6217160219,-0.0831426544],[-0.1300768242,1.1407357748,-0.0106589506],[-0.0239640729,-0.1289755083,1.1529395812]];
        for (managed,display) in managed.iter().zip(pixels.chunks_exact(4)) {
            for channel in 0..3 {
                let linear: f64=(0..3).map(|i|f64::from(managed[i])*matrix[channel][i]).sum();
                let encoded=linear.signum()*if linear.abs()<=0.0031308 {linear.abs()*12.92} else {1.055*linear.abs().powf(1.0/2.4)-0.055};
                let expected=(encoded.clamp(0.0,1.0)*255.0).round() as i32;
                assert!((i32::from(display[channel])-expected).abs()<=2,"legacy helper did not receive sRGB encoded pixels");
            }
        }
        assert_eq!(executor.output_domain("legacy"),ColorDomain::LegacySrgb);
    });
}

#[test]
fn stage_bypass_reuses_warm_variant_and_rejects_ambiguous_helper_domains() {
    pollster::block_on(async {
        let backend=hardware_backend().await;
        let mut executor=GpuExecutor::new(backend.clone());
        executor.upload_rgba("source",&PIXELS,4,1).unwrap();
        for (frame,zones) in [(1,true),(2,false),(3,true)] {
            let mut config=ColorCorrectorConfig::default();
            config.primaries.exposure=1.0;
            config.hdr_zones.enabled=zones;
            config.hdr_zones.highlights.exposure=-0.25;
            let graph=graph(&[("a","source",config.clone())],false);
            let mut evaluator=GraphEvaluator::prepare_with_options(graph.nodes,graph.edges,false);
            evaluate(&mut executor,&mut evaluator,frame);
            assert_oracle(&read_half(&executor,"a"),&config,&source_pixels(),ColorDomain::LegacySrgb);
        }
        let stats=backend.runtime_stats();
        assert_eq!((stats.pipeline_creations,stats.pipeline_cache_hits,stats.target_allocations),(2,1,1));
        let code="@group(0) @binding(0) var managed:texture_2d<f32>; @group(0) @binding(1) var legacy:texture_2d<f32>; @group(0) @binding(2) var s:sampler; fn sample_image(t:texture_2d<f32>, uv:vec2f)->vec4f{return textureSample(t,s,uv);} @fragment fn main(@location(0) uv:vec2f)->@location(0) vec4f {return sample_image(managed,uv)+sample_image(legacy,uv);}";
        assert!(crate::compiler::color_boundary::convert_managed_samples(code,&HashSet::from(["managed"])).is_err());
        let gather="@group(0) @binding(0) var managed:texture_2d<f32>; @group(0) @binding(1) var s:sampler; @fragment fn main(@location(0) uv:vec2f)->@location(0) vec4f {return textureGather(0,managed,s,uv);}";
        assert!(crate::compiler::color_boundary::convert_managed_samples(gather,&HashSet::from(["managed"])).is_err());
    });
}

#[test]
fn render_device_reset_releases_display_scratch_and_graph_budget_across_reuse() {
    pollster::block_on(async {
        let (mut render,_)=RenderDevice::new().await.unwrap();
        let backend=render.backend();
        let budget=backend.frame_budget();
        let mut config=ColorCorrectorConfig::default();
        config.primaries.exposure=1.0;
        let graph=graph(&[("a","source",config)],false);
        assert_eq!(budget.usage(),(0,0));
        for frame in 1..=2 {
            let mut evaluator=GraphEvaluator::prepare_with_options(graph.nodes.clone(),graph.edges.clone(),false);
            render.upload_rgba("source",&PIXELS,4,1).unwrap();
            assert!(render.try_prepare_frame_targets(evaluator.plan(),&HashMap::from([("source".into(),(4,1))])).unwrap());
            let commands=evaluator.run_frame(&inputs(frame));
            render.execute(evaluator.plan(),&commands.commands).unwrap();
            assert_eq!(budget.usage(),(1,32),"the managed graph owns one FP16 output");
            let display=render.display_output_handle("a").unwrap();
            assert_eq!((display.width,display.height,display.format),(4,1,TextureFormat::Rgba8Unorm));
            let repeated=render.display_output_handle("a").unwrap();
            assert!(Arc::ptr_eq(&display.texture,&repeated.texture),"display calls reuse their scratch allocation");
            let scratch_texture=Arc::downgrade(&display.texture);
            let scratch_view=Arc::downgrade(&display.view);
            drop(repeated);
            render.reset().unwrap();
            assert!(render.output_handle("a").is_none());
            assert!(scratch_texture.upgrade().is_some(),"reset preserves consumer-held handles");
            drop(display);
            assert!(scratch_texture.upgrade().is_none(),"reset must not retain the display allocation");
            assert!(scratch_view.upgrade().is_none(),"reset must not retain the display view");
            for _ in 0..3 {
                backend.device.poll(wgpu::PollType::Wait {
                    submission_index:None,timeout:Some(std::time::Duration::from_secs(5)),
                }).unwrap();
            }
            assert_eq!(budget.usage(),(0,0),"reset releases graph allocation credits before reuse");
        }
    });
}
