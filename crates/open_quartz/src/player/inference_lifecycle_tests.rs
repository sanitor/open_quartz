use crate::inference::{InferenceTask, PreparedInferenceModel, ExecutionPolicy};
use super::*;

fn config() -> InferenceConfig {
    InferenceConfig { model_id: "super-resolution-3x".to_owned(), task: InferenceTask::SuperResolution,
        target_size: 224, policy: ExecutionPolicy::resident(), ..Default::default() }
}
fn graph() -> String {
    serde_json::json!({"nodes":[
        {"id":"image","type":"shader","position":{"x":0,"y":0},"data":{
            "type":"shader","label":"GPU blue","shaderCode":"@fragment fn main() -> @location(0) vec4f { return vec4f(0.0,0.0,1.0,1.0); }",
            "inputs":[],"outputs":[{"id":"out","label":"image","dataType":"sampler2D","direction":"output"}],
            "uniforms":{},"autoSize":false,"width":2,"height":2}},
        {"id":"model","type":"inference","position":{"x":1,"y":0},"data":{
            "type":"inference","label":"SR3","shaderCode":"","inputs":[{"id":"in","label":"image","dataType":"sampler2D","direction":"input"}],
            "outputs":[{"id":"out","label":"image","dataType":"sampler2D","direction":"output"}],"uniforms":{},
            "inferenceModelId":"super-resolution-3x","inferenceTask":"super-resolution"}},
        {"id":"renderer","type":"renderer","position":{"x":2,"y":0},"data":{
            "type":"renderer","label":"Renderer","shaderCode":"","inputs":[{"id":"in","label":"image","dataType":"sampler2D","direction":"input"}],
            "outputs":[],"uniforms":{}}}
    ],"edges":[
        {"id":"a","source":"image","sourceHandle":"out","target":"model","targetHandle":"in"},
        {"id":"b","source":"model","sourceHandle":"out","target":"renderer","targetHandle":"in"}
    ]}).to_string()
}
#[test]
fn configured_resident_model_runs_on_the_renderers_original_device_and_queue() {
    let (mut host, info) = pollster::block_on(NativePlayer::new()).unwrap();
    assert_eq!(info.backend, "Metal");
    assert!(info.shared_inference_wgpu_device);
    assert!(!info.native_inference_cpu);
    host.set_graph(&graph()).unwrap();
    let bytes = crate::inference::resident_test_model::sr3_model();
    let prepared = PreparedInferenceModel::from_memory(&bytes, config()).unwrap();
    let attached = prepared.attach(&mut host, "model".to_owned()).unwrap();
    assert_eq!(attached.plan.configured_providers, [oq_inference::InferenceProvider::Metal]);
    assert_eq!(attached.plan.transfer_path, oq_inference::TransferPath::Device);
    assert_eq!(attached.plan.provider_placement, oq_inference::ProviderPlacement::Gpu);
    host.start_playback().unwrap();
    host.render_next().unwrap();
    let frame = host.render_next().unwrap();
    assert_eq!((frame.width, frame.height), (6, 6));
    // Test-only observation. Native inference itself never calls read_output.
    let pixels = host.render_device.read_output_rgba("renderer").unwrap();
    assert_eq!((pixels.1, pixels.2), (6, 6));
    assert!(pixels.0.chunks_exact(4).all(|pixel| pixel == [0, 0, 255, 255]));
    let metrics = host.inference_metrics();
    assert_eq!(metrics.submitted, 1); assert_eq!(metrics.completed, 1); assert_eq!(metrics.accepted, 1);
    assert_eq!(metrics.in_flight, 0); assert_eq!(metrics.queue_depth, 0);
    assert_eq!(metrics.readback_bytes, 0); assert_eq!(metrics.upload_bytes, 0);
    assert_eq!(metrics.input_cpu_bytes, 0); assert_eq!(metrics.output_cpu_bytes, 0);
    assert_eq!(metrics.device_submissions, 1); assert_eq!(metrics.compute_dispatches, 6);
    assert_eq!(metrics.gpu_time_ns, None);
    assert_eq!(metrics.failed, 0); assert_eq!(metrics.rejected, 0);
    assert!(host.take_inference_events().iter().any(|event| event.node_id == "model" && event.backend == "metal"));
    // Re-evaluation with a retained prior output exercises independent output leases.
    host.session.mark_dirty("model").unwrap();
    host.render_next().unwrap(); host.render_next().unwrap();
    assert_eq!(host.render_device.read_output_rgba("renderer").unwrap().0, pixels.0);
    host.stop().unwrap(); host.unload_inference("model");
    assert!(!host.has_output_texture("model"));
    host.close().unwrap(); assert_eq!(host.inference_metrics().in_flight, 0);
}
#[test]
fn foreign_device_session_is_rejected_without_pixel_or_resource_mutation() {
    let (mut host, _) = pollster::block_on(NativePlayer::new()).unwrap();
    let (foreign, _) = pollster::block_on(NativePlayer::new()).unwrap();
    host.set_graph(&graph()).unwrap();
    let model = oq_inference::gpu_model::GpuModel::from_onnx(&crate::inference::resident_test_model::sr3_model()).unwrap();
    let foreign = foreign.backend();
    let session = oq_inference::gpu::GpuImageSession::prepare(foreign.device.clone(), foreign.queue.clone(), model).unwrap();
    let error = host.load_resident_inference("model".to_owned(), session, config()).unwrap_err();
    assert!(error.contains("exact GPU device and queue"), "{error}");
    assert!(!host.has_output_texture("model"));
    assert_eq!(host.inference_metrics().submitted, 0);
    assert_eq!(host.inference_metrics().readback_bytes, 0);
}

#[test]
fn retained_output_capacity_coalesces_without_failing_and_retries_after_release() {
    let (mut host, _) = pollster::block_on(NativePlayer::new()).unwrap();
    host.set_graph(&graph()).unwrap();
    PreparedInferenceModel::from_memory(&crate::inference::resident_test_model::sr3_model(), config())
        .unwrap().attach(&mut host, "model".to_owned()).unwrap();
    host.start_playback().unwrap();
    let mut consumers = Vec::new();
    for _ in 0..3 {
        host.session.mark_dirty("model").unwrap();
        host.render_next().unwrap();
        host.render_next().unwrap();
        consumers.push(host.render_device.output_handle("model").unwrap());
    }
    let before = host.inference_metrics();
    assert_eq!(before.device_submissions, 3);
    assert_eq!(before.accepted, 3);
    let current = host.render_device.output_handle("model").unwrap();
    host.take_inference_events();
    host.session.mark_dirty("model").unwrap();
    host.render_next().unwrap();
    host.render_next().unwrap();
    let blocked = host.inference_metrics();
    assert_eq!(blocked.submitted, before.submitted);
    assert_eq!(blocked.device_submissions, before.device_submissions);
    assert_eq!(blocked.completed, before.completed);
    assert_eq!(blocked.accepted, before.accepted);
    assert_eq!(blocked.failed, 0);
    assert_eq!(blocked.in_flight, 0);
    assert_eq!(blocked.queue_depth, 1);
    assert!(host.take_inference_events().is_empty());
    assert!(Arc::ptr_eq(&current.texture, &host.render_device.output_handle("model").unwrap().texture));
    let pixels = host.render_device.read_output_rgba("renderer").unwrap().0;
    assert!(pixels.chunks_exact(4).all(|pixel| pixel == [0, 0, 255, 255]));
    drop(consumers.remove(0));
    host.render_next().unwrap();
    host.render_next().unwrap();
    let retried = host.inference_metrics();
    assert_eq!(retried.device_submissions, before.device_submissions + 1);
    assert_eq!(retried.accepted, before.accepted + 1);
    assert_eq!(retried.failed, 0);
    assert_eq!(retried.queue_depth, 0);
    assert_eq!(host.render_device.read_output_rgba("renderer").unwrap().0, pixels);
    assert_eq!(retried.readback_bytes, 0);
    assert_eq!(retried.upload_bytes, 0);
    host.close().unwrap();
}
