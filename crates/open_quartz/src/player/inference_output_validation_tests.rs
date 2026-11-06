use crate::inference::NativeOnnxImageOutput;
use crate::outputs::{ContentStamp, DeliveryPolicy, OutputKey, OutputPayload, OutputState, OutputSubscription, OutputTransport};

use super::*;

fn graph() -> Graph {
    serde_json::from_value(serde_json::json!({"nodes":[
        {"id":"onnx","type":"onnx","position":{"x":0,"y":0},"data":{"type":"onnx","label":"Model","shaderCode":"","inputs":[],"outputs":[{"id":"out","label":"output","dataType":"sampler2D","direction":"output"}],"uniforms":{}}},
        {"id":"renderer","type":"renderer","position":{"x":1,"y":0},"data":{"type":"renderer","label":"Renderer","shaderCode":"","inputs":[{"id":"in","label":"image","dataType":"sampler2D","direction":"input"}],"outputs":[],"uniforms":{}}}
    ],"edges":[{"id":"edge","source":"onnx","sourceHandle":"out","target":"renderer","targetHandle":"in"}]})).unwrap()
}

fn advance_and_drain(player: &mut NativePlayer, now_ns: u64) -> Vec<ExecutionCommand> {
    player.session.advance(&SessionFrameInput {
        now_ns,
        date: [0.0; 4],
        mouse: [0.0; 4],
        resolution: [1.0, 1.0, 1.0],
    }).unwrap();
    player.session.drain_commands()
}

fn queue_new_image(player: &mut NativePlayer) {
    player.inference.queue_test_completion("onnx", &player.session, NativeOnnxImageOutput {
        rgba: vec![0, 0, 255, 255], width: 1, height: 1, data: None,
    });
}

#[test]
fn invalid_output_contract_rejects_completion_before_texture_or_dirty_mutation() {
    let (mut player, _) = pollster::block_on(NativePlayer::new()).unwrap();
    let mut graph = graph();
    player.set_graph_snapshot(&graph).unwrap();
    let old = [255, 0, 0, 255];
    player.render_device.upload_rgba("onnx", &old, 1, 1).unwrap();
    // Change the session contract while retaining the inference owner's descriptor.
    // The completion has current task/revision/generation identity but an invalid payload.
    graph.nodes[0].data.outputs[0].data_type = crate::project::model::DataType::Float;
    player.session.set_graph(&graph).unwrap();
    player.session.play(0).unwrap();
    advance_and_drain(&mut player, 1);
    queue_new_image(&mut player);
    assert!(player.drain_inference_completions().is_err());
    assert_eq!(player.render_device.read_output_rgba("onnx").unwrap().0, old);
    assert!(player.take_output_events().is_empty());
    assert!(player.session.drain_deliveries().deliveries.is_empty());
    assert!(!advance_and_drain(&mut player, 2).iter().any(|command| command.node_id == "renderer"));
}

#[test]
fn every_backpressure_rejects_completion_before_texture_or_dirty_mutation() {
    let (mut player, _) = pollster::block_on(NativePlayer::new()).unwrap();
    player.set_graph_snapshot(&graph()).unwrap();
    let old = [255, 0, 0, 255];
    player.render_device.upload_rgba("onnx", &old, 1, 1).unwrap();
    player.session.play(0).unwrap();
    advance_and_drain(&mut player, 1);
    player.session.set_every_queue_capacity(1);
    player.session.subscribe_output(OutputSubscription {
        subscription_id: "every".into(), output: OutputKey::new("onnx", "out"),
        delivery: DeliveryPolicy::Every, transport: OutputTransport::Preview,
        max_width: None, max_height: None,
    }).unwrap();
    player.session.publish_output(OutputState {
        output: OutputKey::new("onnx", "out"), graph_revision: player.session.revision(),
        output_generation: u64::from(player.session.node_generation("onnx").unwrap()),
        evaluation_stamp: FrameStamp { epoch: 1, frame: 0, timeline_ns: 0, deadline_ns: 0 },
        content_stamp: ContentStamp { epoch: 1, timeline_ns: 0, media_pts_ns: None },
        payload: OutputPayload::Resource { handle: 99 },
    }).unwrap();
    queue_new_image(&mut player);
    assert!(player.drain_inference_completions().is_err());
    assert_eq!(player.render_device.read_output_rgba("onnx").unwrap().0, old);
    assert!(player.take_output_events().is_empty());
    let deliveries = player.session.drain_deliveries();
    assert_eq!(deliveries.deliveries.iter().filter(|delivery| delivery.subscription_id == "every").count(), 1);
    assert!(deliveries.deliveries.iter().all(|delivery| matches!(delivery.state.payload, OutputPayload::Resource { handle: 99 })));
    assert!(!advance_and_drain(&mut player, 2).iter().any(|command| command.node_id == "renderer"));
}
