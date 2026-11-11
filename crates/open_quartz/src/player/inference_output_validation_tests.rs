use crate::outputs::{ContentStamp, DeliveryPolicy, OutputKey, OutputPayload, OutputState, OutputSubscription, OutputTransport};
use crate::rendering::GpuOutputHandle;

use super::*;

fn graph() -> Graph {
    serde_json::from_value(serde_json::json!({"nodes":[
        {"id":"onnx","type":"onnx","position":{"x":0,"y":0},"data":{"type":"onnx","label":"Model","shaderCode":"","inputs":[],"outputs":[{"id":"out","label":"output","dataType":"sampler2D","direction":"output"}],"uniforms":{}}},
        {"id":"renderer","type":"renderer","position":{"x":1,"y":0},"data":{"type":"renderer","label":"Renderer","shaderCode":"","inputs":[{"id":"in","label":"image","dataType":"sampler2D","direction":"input"}],"outputs":[],"uniforms":{}}}
    ],"edges":[{"id":"edge","source":"onnx","sourceHandle":"out","target":"renderer","targetHandle":"in"}]})).unwrap()
}

fn advance_and_drain(player: &mut NativePlayer, now_ns: u64) -> Vec<ExecutionCommand> {
    player.pipeline.session.evaluate(&FrameEvaluationInput {
        stamp: FrameStamp { epoch: 1, frame: now_ns, timeline_ns: now_ns, deadline_ns: now_ns },
        previous_timeline_ns: now_ns.saturating_sub(1),
        date: [0.0; 4], mouse: [0.0; 4], resolution: [1.0, 1.0, 1.0],
    }).unwrap();
    player.pipeline.session.drain_commands()
}

fn queue_new_image(player: &mut NativePlayer) {
    let backend = player.backend();
    let texture = backend.create_texture(1, 1, crate::rendering::TextureFormat::Rgba8Unorm);
    backend.upload_rgba(&texture, &[0, 0, 255, 255]).unwrap();
    let pipeline = &mut player.pipeline;
    let stamp = pipeline.pending_stamp().cloned().unwrap_or(FrameStamp { epoch: 1, frame: 1, timeline_ns: 1, deadline_ns: 1 });
    pipeline.inference.queue_test_completion("onnx", &pipeline.session, GpuOutputHandle {
        texture: texture.texture, view: texture.view, sampler: texture.sampler,
        width: 1, height: 1, format: texture.format,
        lease: None,
    }, stamp);
}

#[test]
fn invalid_output_contract_rejects_completion_before_texture_or_dirty_mutation() {
    let (mut player, _) = pollster::block_on(NativePlayer::new()).unwrap();
    let mut graph = graph();
    player.set_graph_snapshot(&graph).unwrap();
    let old = [255, 0, 0, 255];
    player.pipeline.render_device.upload_rgba("onnx", &old, 1, 1).unwrap();
    // Change the session contract while retaining the inference owner's descriptor.
    // The completion has current task/revision/generation identity but an invalid payload.
    graph.nodes[0].data.outputs[0].data_type = crate::project::model::DataType::Float;
    player.pipeline.session.set_graph(&graph).unwrap();
    advance_and_drain(&mut player, 1);
    queue_new_image(&mut player);
    assert!(player.drain_inference_completions().is_err());
    assert_eq!(player.render_device.read_output_rgba("onnx").unwrap().0, old);
    assert!(player.take_inference_events().is_empty());
    assert!(player.session.drain_deliveries().deliveries.is_empty());
    assert!(!advance_and_drain(&mut player, 2).iter().any(|command| command.node_id == "renderer"));
    assert_eq!(player.inference_metrics().rejected, 1);
    assert_eq!(player.inference_metrics().accepted, 0);
    assert_eq!(player.inference_metrics().upload_bytes, 0);
}

#[test]
fn every_backpressure_rejects_completion_before_texture_or_dirty_mutation() {
    let (mut player, _) = pollster::block_on(NativePlayer::new()).unwrap();
    player.set_graph_snapshot(&graph()).unwrap();
    let old = [255, 0, 0, 255];
    player.pipeline.render_device.upload_rgba("onnx", &old, 1, 1).unwrap();
    advance_and_drain(&mut player, 1);
    player.pipeline.session.set_every_queue_capacity(1);
    player.pipeline.session.subscribe_output(OutputSubscription {
        subscription_id: "every".into(), output: OutputKey::new("onnx", "out"),
        delivery: DeliveryPolicy::Every, transport: OutputTransport::Preview,
        max_width: None, max_height: None,
    }).unwrap();
    player.pipeline.session.publish_output(OutputState {
        output: OutputKey::new("onnx", "out"), graph_revision: player.pipeline.session.revision(),
        output_generation: u64::from(player.pipeline.session.node_generation("onnx").unwrap()),
        evaluation_stamp: FrameStamp { epoch: 1, frame: 0, timeline_ns: 0, deadline_ns: 0 },
        content_stamp: ContentStamp { epoch: 1, timeline_ns: 0, media_pts_ns: None },
        payload: OutputPayload::Resource { handle: 99 },
    }).unwrap();
    queue_new_image(&mut player);
    assert!(player.drain_inference_completions().is_err());
    assert_eq!(player.render_device.read_output_rgba("onnx").unwrap().0, old);
    assert!(player.take_inference_events().is_empty());
    let deliveries = player.pipeline.session.drain_deliveries();
    assert_eq!(deliveries.deliveries.iter().filter(|delivery| delivery.subscription_id == "every").count(), 1);
    assert!(deliveries.deliveries.iter().all(|delivery| matches!(delivery.state.payload, OutputPayload::Resource { handle: 99 })));
    assert!(!advance_and_drain(&mut player, 2).iter().any(|command| command.node_id == "renderer"));
    assert_eq!(player.inference_metrics().rejected, 1);
    assert_eq!(player.inference_metrics().accepted, 0);
    assert_eq!(player.inference_metrics().upload_bytes, 0);
}

#[test]
fn detection_before_image_output_keeps_preview_and_renderer_delivery() {
    let (mut player, _) = pollster::block_on(NativePlayer::new()).unwrap();
    let mut graph = graph();
    graph.nodes[0].data.outputs.insert(0, crate::project::model::Port {
        id: "detections".into(), label: "detections".into(),
        data_type: crate::project::model::DataType::Roi,
        direction: crate::project::model::PortDirection::Output,
        default_value: None, description: None,
    });
    player.set_graph_snapshot(&graph).unwrap();
    player.start_playback().unwrap();
    player.render_next().unwrap();
    queue_new_image(&mut player);
    player.render_next().unwrap();
    let (pixels, width, height) = player.pipeline.render_device.read_output_rgba("renderer").unwrap();
    assert_eq!((width, height), (1, 1));
    assert_eq!(pixels, [0, 0, 255, 255]);
    let events = player.take_inference_events();
    assert_eq!(events.len(), 1);
    assert_eq!((events[0].node_id.as_str(), events[0].width, events[0].height), ("onnx", 1, 1));
    assert_eq!(player.inference_metrics().accepted, 1);
    assert_eq!(player.inference_metrics().rejected, 0);
}
