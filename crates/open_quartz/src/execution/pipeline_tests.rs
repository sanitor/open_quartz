use super::*;
use crate::outputs::{
    DeliveryPolicy, OutputKey, OutputPayload, OutputSubscription, OutputTransport,
};
use serde_json::json;
use std::time::{Duration, Instant};

fn input_node(id: &str) -> serde_json::Value {
    json!({"id":id,"type":"input","position":{"x":0,"y":0},"data":{"type":"input","label":id,"shaderCode":"","inputMode":"image","inputDataType":"sampler2D","inputs":[],"outputs":[{"id":"out","label":"image","dataType":"sampler2D","direction":"output"}],"uniforms":{}}})
}
fn shader(id: &str, code: &str, inputs: serde_json::Value) -> serde_json::Value {
    json!({"id":id,"type":"shader","position":{"x":0,"y":0},"data":{"type":"shader","label":id,"shaderCode":code,"inputs":inputs,"outputs":[{"id":"out","label":"fragColor","dataType":"vec4","direction":"output"}],"uniforms":{},"autoSize":false,"width":2,"height":2}})
}
fn texture_port() -> serde_json::Value {
    json!([{"id":"in","label":"image","dataType":"sampler2D","direction":"input"}])
}
fn image_graph() -> Graph {
    serde_json::from_value(json!({"nodes":[input_node("image"),shader("output","@fragment fn main(@location(0) uv:vec2f)->@location(0) vec4f {return textureSample(image,imageSampler,uv);}",texture_port())],"edges":[{"id":"e","source":"image","sourceHandle":"out","target":"output","targetHandle":"in"}]})).unwrap()
}
fn animated_graph() -> Graph {
    serde_json::from_value(json!({"nodes":[shader("output","@fragment fn main()->@location(0) vec4f {return vec4f(iTime,0.0,1.0-iTime,1.0);}",json!([]))],"edges":[]})).unwrap()
}
async fn device() -> RenderDevice {
    RenderDevice::new().await.expect("real GPU required").0
}
fn identity() -> FrameSnapshotIdentity {
    FrameSnapshotIdentity {
        output_node_id: "image".into(),
        graph_revision: 1,
        evaluation_stamp: None,
        content_stamp: None,
        color_space: FrameColorSpace::Srgb,
        alpha_mode: FrameAlphaMode::Opaque,
    }
}
fn capture(device: &mut RenderDevice, pool: &StableFramePool, pixel: [u8; 4]) -> StableFrame {
    device.upload_rgba("image", &pixel.repeat(4), 2, 2).unwrap();
    pool.try_capture(device, "image", identity()).unwrap()
}
fn pipeline(device: RenderDevice, graph: &Graph) -> FramePipeline {
    let mut pipeline =
        FramePipeline::from_render_device(device, PlaybackCapabilities { data_paths: vec![] });
    pipeline.set_graph_snapshot(graph).unwrap();
    pipeline
}
fn demand(
    pipeline: &FramePipeline,
    id: u64,
    epoch: u64,
    time: u64,
    sources: Vec<ExternalFrameInput>,
) -> FrameDemand {
    FrameDemand {
        request_id: id,
        graph_revision: pipeline.revision(),
        output_node_id: "output".into(),
        sources,
        input: FrameEvaluationInput {
            stamp: FrameStamp {
                epoch,
                frame: id,
                timeline_ns: time,
                deadline_ns: time + 10_000,
            },
            previous_timeline_ns: time.saturating_sub(1),
            date: [0.0; 4],
            mouse: [0.0; 4],
            resolution: [2.0, 2.0, 1.0],
        },
    }
}
fn accepted(pipeline: &mut FramePipeline, demand: FrameDemand) {
    match pipeline.submit(demand) {
        FrameAdmission::Accepted => {}
        FrameAdmission::Rejected { error, .. } => panic!("{error}"),
        FrameAdmission::WouldBlock(_) => panic!("unexpected backpressure"),
    }
}
fn complete(pipeline: &mut FramePipeline) -> FrameCompletion {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        pipeline.progress();
        if let Some(result) = pipeline.poll_completion() {
            return result;
        }
        assert!(
            Instant::now() < deadline,
            "frame did not reach a terminal outcome"
        );
        std::thread::yield_now();
    }
}
fn rendered(completion: FrameCompletion) -> StableFrame {
    match completion.outcome {
        FrameOutcome::Submitted(frame) => frame,
        FrameOutcome::Failed(e) => panic!("{e}"),
        FrameOutcome::Cancelled => panic!("unexpected cancellation"),
    }
}
fn pixels(pipeline: &FramePipeline, frame: &StableFrame) -> Vec<u8> {
    frame.ready.wait(Duration::from_secs(5)).unwrap();
    pollster::block_on(pipeline.backend().read_texture_rgba(
        &frame.output().texture,
        frame.image.width,
        frame.image.height,
    ))
    .unwrap()
}
#[test]
fn explicit_time_frames_have_one_terminal_and_return_blocked_input_ownership() {
    pollster::block_on(async {
        let mut p = pipeline(device().await, &animated_graph());
        let first = demand(&p, 1, 3, 250_000_000, vec![]);
        accepted(&mut p, first);
        let second = demand(&p, 2, 3, 750_000_000, vec![]);
        let returned = match p.submit(second) {
            FrameAdmission::WouldBlock(d) => d,
            _ => panic!("active request must return demand"),
        };
        assert_eq!(returned.request_id, 2);
        p.progress();
        assert!(
            matches!(p.submit(returned), FrameAdmission::WouldBlock(_)),
            "unconsumed terminal also occupies admission capacity"
        );
        let completion = p.poll_completion().unwrap();
        assert_eq!(completion.request_id, 1);
        assert_eq!(completion.stamp.epoch, 3);
        let red = rendered(completion);
        let next = demand(&p, 2, 3, 750_000_000, vec![]);
        accepted(&mut p, next);
        let blue = rendered(complete(&mut p));
        assert!(pixels(&p, &red)
            .chunks_exact(4)
            .all(|px| px == [64, 0, 191, 255]));
        assert!(pixels(&p, &blue)
            .chunks_exact(4)
            .all(|px| px == [191, 0, 64, 255]));
        p.progress();
        assert!(p.poll_completion().is_none());
        assert!(!p.has_in_flight());
        let duplicate = demand(&p, 2, 3, 750_000_000, vec![]);
        assert!(matches!(
            p.submit(duplicate),
            FrameAdmission::Rejected { .. }
        ));
        let old_epoch = demand(&p, 9, 2, 0, vec![]);
        assert!(matches!(
            p.submit(old_epoch),
            FrameAdmission::Rejected { .. }
        ));
        let new_epoch = demand(&p, 1, 4, 0, vec![]);
        accepted(&mut p, new_epoch);
        assert_eq!(
            rendered(complete(&mut p))
                .identity
                .evaluation_stamp
                .unwrap()
                .epoch,
            4
        );
    });
}
#[test]
fn external_source_versions_and_wrong_owner_reject_before_mutation() {
    pollster::block_on(async {
        let mut render = device().await;
        let pool = StableFramePool::new(render.backend(), StableFrameBudget::default()).unwrap();
        let red = capture(&mut render, &pool, [255, 0, 0, 255]);
        let blue = capture(&mut render, &pool, [0, 0, 255, 255]);
        let mut p = pipeline(render, &image_graph());
        let first = demand(
            &p,
            1,
            1,
            0,
            vec![ExternalFrameInput {
                node_id: "image".into(),
                version: 1,
                frame: red.clone(),
            }],
        );
        accepted(&mut p, first);
        let original = rendered(complete(&mut p));
        assert_eq!(pixels(&p, &original), [255, 0, 0, 255].repeat(4));
        let reused = demand(
            &p,
            2,
            1,
            100,
            vec![ExternalFrameInput {
                node_id: "image".into(),
                version: 1,
                frame: blue.clone(),
            }],
        );
        let returned = match p.submit(reused) {
            FrameAdmission::Rejected { demand, .. } => demand,
            _ => panic!("version alias was not rejected"),
        };
        assert_eq!(
            returned.sources[0].frame.output().texture.as_ref(),
            blue.output().texture.as_ref()
        );
        assert_eq!(
            p.render_device.read_output_rgba("image").unwrap().0,
            [255, 0, 0, 255].repeat(4)
        );
        let mut foreign = device().await;
        let foreign_pool =
            StableFramePool::new(foreign.backend(), StableFrameBudget::default()).unwrap();
        let foreign_blue = capture(&mut foreign, &foreign_pool, [0, 0, 255, 255]);
        let foreign_demand = demand(
            &p,
            2,
            1,
            100,
            vec![ExternalFrameInput {
                node_id: "image".into(),
                version: 2,
                frame: foreign_blue,
            }],
        );
        assert!(matches!(
            p.submit(foreign_demand),
            FrameAdmission::Rejected { .. }
        ));
        for invalid in [0, 1] {
            let mut request = demand(&p, 2, 1, 100, vec![]);
            if invalid == 0 {
                request.graph_revision += 1;
            } else {
                request.output_node_id = "missing".into();
            }
            assert!(matches!(p.submit(request), FrameAdmission::Rejected { .. }));
        }
        let next = demand(
            &p,
            2,
            1,
            100,
            vec![ExternalFrameInput {
                node_id: "image".into(),
                version: 2,
                frame: blue,
            }],
        );
        accepted(&mut p, next);
        let changed = rendered(complete(&mut p));
        assert_eq!(pixels(&p, &changed), [0, 0, 255, 255].repeat(4));
        assert_eq!(pixels(&p, &original), [255, 0, 0, 255].repeat(4));
    });
}
#[test]
fn output_pool_backpressure_cancellation_and_close_keep_exactly_one_terminal() {
    pollster::block_on(async {
        let mut p = pipeline(device().await, &animated_graph());
        let mut held = Vec::new();
        for id in 1..=3 {
            let d = demand(&p, id, 1, id * 100, vec![]);
            accepted(&mut p, d);
            held.push(rendered(complete(&mut p)));
        }
        let d = demand(&p, 4, 1, 400, vec![]);
        accepted(&mut p, d);
        p.progress();
        assert!(p.poll_completion().is_none());
        assert_eq!(p.pending_stamp().unwrap().frame, 4);
        assert!(p.cancel_pending());
        assert!(!p.cancel_pending());
        p.progress();
        let cancelled = p.poll_completion().unwrap();
        assert_eq!(cancelled.request_id, 4);
        assert!(matches!(cancelled.outcome, FrameOutcome::Cancelled));
        assert!(p.poll_completion().is_none());
        let d = demand(&p, 5, 1, 500, vec![]);
        accepted(&mut p, d);
        p.progress();
        p.dispose().unwrap();
        assert!(matches!(
            p.poll_completion().unwrap().outcome,
            FrameOutcome::Cancelled
        ));
        p.progress();
        assert!(p.poll_completion().is_none());
        let d = demand(&p, 6, 1, 600, vec![]);
        assert!(matches!(p.submit(d), FrameAdmission::Rejected { .. }));
        for frame in &held {
            assert_eq!(pixels(&p, frame), [0, 0, 255, 255].repeat(4));
        }
    });
}

#[test]
fn released_consumer_finishes_the_same_admitted_request_without_resubmission() {
    pollster::block_on(async {
        let mut p = pipeline(device().await, &animated_graph());
        let mut held = Vec::new();
        for id in 1..=3 {
            let request = demand(&p, id, 1, id * 100, vec![]);
            accepted(&mut p, request);
            held.push(rendered(complete(&mut p)));
        }
        let request = demand(&p, 4, 1, 750_000_000, vec![]);
        accepted(&mut p, request);
        p.progress();
        assert!(p.poll_completion().is_none());
        drop(held.remove(0));
        let completion = complete(&mut p);
        assert_eq!(completion.request_id, 4);
        assert_eq!(
            pixels(&p, &rendered(completion)),
            [191, 0, 64, 255].repeat(4)
        );
        p.progress();
        assert!(p.poll_completion().is_none());
    });
}
#[test]
fn rejected_graph_staging_keeps_request_and_valid_replacement_cancels_old_revision() {
    pollster::block_on(async {
        let graph = image_graph();
        let mut p = pipeline(device().await, &graph);
        let request = demand(&p, 1, 1, 0, vec![]);
        accepted(&mut p, request);
        let mut invalid = graph.clone();
        invalid.nodes.push(invalid.nodes[0].clone());
        assert!(p.set_graph_snapshot(&invalid).is_err());
        assert_eq!(p.pending_stamp().unwrap().frame, 1);
        let next_revision = p.set_graph_snapshot(&animated_graph()).unwrap();
        let cancelled = p.poll_completion().unwrap();
        assert_eq!(cancelled.graph_revision, next_revision - 1);
        assert!(matches!(cancelled.outcome, FrameOutcome::Cancelled));
        let fresh = demand(&p, 2, 1, 0, vec![]);
        accepted(&mut p, fresh);
        let frame = rendered(complete(&mut p));
        assert_eq!(frame.identity.graph_revision, next_revision);
        assert_eq!(pixels(&p, &frame), [0, 0, 255, 255].repeat(4));
    });
}
#[test]
fn every_preflight_restores_external_binding_and_retries_without_losing_demand() {
    pollster::block_on(async {
        let mut render = device().await;
        let pool = StableFramePool::new(render.backend(), StableFrameBudget::default()).unwrap();
        let red = capture(&mut render, &pool, [255, 0, 0, 255]);
        let blue = capture(&mut render, &pool, [0, 0, 255, 255]);
        let mut graph = image_graph();
        let math=serde_json::from_value(json!({"id":"math","type":"math","position":{"x":0,"y":0},"data":{"type":"math","label":"sum","shaderCode":"","mathOp":"add","inputs":[],"outputs":[{"id":"out","label":"out","dataType":"float","direction":"output"}],"uniforms":{"a":2.0,"b":3.0}}})).unwrap();
        graph.nodes.push(math);
        let mut p = pipeline(render, &graph);
        p.set_every_queue_capacity(1);
        p.subscribe_output(OutputSubscription {
            subscription_id: "all".into(),
            output: OutputKey::new("math", "out"),
            delivery: DeliveryPolicy::Every,
            transport: OutputTransport::Value,
            max_width: None,
            max_height: None,
        })
        .unwrap();
        let first = demand(
            &p,
            1,
            1,
            0,
            vec![ExternalFrameInput {
                node_id: "image".into(),
                version: 1,
                frame: red,
            }],
        );
        accepted(&mut p, first);
        let _first = rendered(complete(&mut p));
        p.session.mark_dirty("math").unwrap();
        let next = demand(
            &p,
            2,
            1,
            100,
            vec![ExternalFrameInput {
                node_id: "image".into(),
                version: 2,
                frame: blue,
            }],
        );
        let retry = match p.submit(next) {
            FrameAdmission::Rejected { demand, .. } => demand,
            _ => panic!("Every capacity must reject before mutation"),
        };
        assert_eq!(p.input_versions["image"], 1);
        assert_eq!(p.session.last_evaluation().unwrap().frame, 1);
        assert_eq!(
            p.render_device.read_output_rgba("image").unwrap().0,
            [255, 0, 0, 255].repeat(4)
        );
        let delivered = p.drain_deliveries();
        assert_eq!(delivered.deliveries.len(), 1);
        assert_eq!(
            delivered.deliveries[0].state.payload,
            OutputPayload::Float(5.0)
        );
        accepted(&mut p, retry);
        let second = rendered(complete(&mut p));
        assert_eq!(pixels(&p, &second), [0, 0, 255, 255].repeat(4));
        assert_eq!(
            p.drain_deliveries().deliveries[0]
                .state
                .evaluation_stamp
                .frame,
            2
        );
    });
}
#[test]
fn unavailable_source_fails_once_instead_of_using_old_or_blank_output() {
    pollster::block_on(async {
        let mut p = pipeline(device().await, &image_graph());
        let d = demand(&p, 1, 1, 0, vec![]);
        accepted(&mut p, d);
        assert!(matches!(complete(&mut p).outcome, FrameOutcome::Failed(_)));
        p.progress();
        assert!(p.poll_completion().is_none());
        let retry = demand(&p, 2, 1, 100, vec![]);
        accepted(&mut p, retry);
        assert!(matches!(complete(&mut p).outcome, FrameOutcome::Failed(_)));
    });
}
#[cfg(target_os = "macos")]
#[test]
fn real_gpu_inference_keeps_each_demand_source_until_same_frame_result_and_cancels_unload() {
    pollster::block_on(async {
        let mut render = device().await;
        let pool = StableFramePool::new(render.backend(), StableFrameBudget::default()).unwrap();
        let red = capture(&mut render, &pool, [255, 0, 0, 255]);
        let blue = capture(&mut render, &pool, [0, 0, 255, 255]);
        let graph:Graph=serde_json::from_value(json!({"nodes":[input_node("image"),
            {"id":"model","type":"inference","position":{"x":0,"y":0},"data":{"type":"inference","label":"SR3","shaderCode":"","inputs":texture_port(),"outputs":[{"id":"out","label":"image","dataType":"sampler2D","direction":"output"}],"uniforms":{},"inferenceModelId":"super-resolution-3x","inferenceTask":"super-resolution"}},
            {"id":"output","type":"renderer","position":{"x":0,"y":0},"data":{"type":"renderer","label":"output","shaderCode":"","inputs":texture_port(),"outputs":[],"uniforms":{}}}],
            "edges":[{"id":"a","source":"image","sourceHandle":"out","target":"model","targetHandle":"in"},{"id":"b","source":"model","sourceHandle":"out","target":"output","targetHandle":"in"}]})).unwrap();
        let mut p = pipeline(render, &graph);
        let model = oq_inference::gpu_model::GpuModel::from_onnx(
            &crate::inference::resident_test_model::sr3_model(),
        )
        .unwrap();
        let backend = p.backend();
        let model = oq_inference::gpu::GpuImageSession::prepare(
            backend.device.clone(),
            backend.queue.clone(),
            model,
        )
        .unwrap();
        p.load_resident_inference(
            "model".into(),
            crate::inference::ResidentInferenceSession::Sr3(model),
            InferenceConfig {
                model_id: "super-resolution-3x".into(),
                task: crate::inference::InferenceTask::SuperResolution,
                target_size: 224,
                policy: crate::inference::ExecutionPolicy::resident(),
                ..Default::default()
            },
        )
        .unwrap();
        p.render_device.remove_texture("image");
        let missing = demand(&p, 1, 0, 0, vec![]);
        accepted(&mut p, missing);
        assert!(matches!(complete(&mut p).outcome, FrameOutcome::Failed(_)));
        p.subscribe_output(OutputSubscription {
            subscription_id: "consumer".into(),
            output: OutputKey::new("model", "out"),
            delivery: DeliveryPolicy::Every,
            transport: OutputTransport::Preview,
            max_width: None,
            max_height: None,
        })
        .unwrap();
        let first = demand(
            &p,
            1,
            1,
            123,
            vec![ExternalFrameInput {
                node_id: "image".into(),
                version: 1,
                frame: red.clone(),
            }],
        );
        accepted(&mut p, first);
        p.progress();
        assert!(
            p.poll_completion().is_none(),
            "model submit is not complete graph output"
        );
        let second = demand(
            &p,
            2,
            1,
            456,
            vec![ExternalFrameInput {
                node_id: "image".into(),
                version: 2,
                frame: blue,
            }],
        );
        let second = match p.submit(second) {
            FrameAdmission::WouldBlock(d) => d,
            _ => panic!("next source must not overwrite the waiting model input"),
        };
        let first = rendered(complete(&mut p));
        assert!(pixels(&p, &first)
            .chunks_exact(4)
            .all(|px| px == [255, 0, 0, 255]));
        assert_eq!(
            first
                .identity
                .evaluation_stamp
                .as_ref()
                .unwrap()
                .timeline_ns,
            123
        );
        accepted(&mut p, second);
        let second = rendered(complete(&mut p));
        assert!(pixels(&p, &second)
            .chunks_exact(4)
            .all(|px| px == [0, 0, 255, 255]));
        assert_eq!(
            second
                .identity
                .evaluation_stamp
                .as_ref()
                .unwrap()
                .timeline_ns,
            456
        );
        let delivered = p.drain_deliveries().deliveries;
        assert_eq!(
            delivered
                .iter()
                .map(|d| (
                    d.subscription_id.as_str(),
                    d.state.evaluation_stamp.timeline_ns
                ))
                .collect::<Vec<_>>(),
            vec![("consumer", 123), ("consumer", 456)]
        );
        let third = demand(
            &p,
            3,
            1,
            789,
            vec![ExternalFrameInput {
                node_id: "image".into(),
                version: 3,
                frame: red,
            }],
        );
        accepted(&mut p, third);
        p.progress();
        p.unload_inference("model");
        assert!(matches!(
            p.poll_completion().unwrap().outcome,
            FrameOutcome::Cancelled
        ));
        p.progress();
        assert!(p.poll_completion().is_none());
        let metrics = p.inference_metrics();
        assert_eq!(
            metrics.readback_bytes
                + metrics.upload_bytes
                + metrics.input_cpu_bytes
                + metrics.output_cpu_bytes,
            0
        );
        assert_eq!(pixels(&p, &first)[..4], [255, 0, 0, 255]);
    });
}
