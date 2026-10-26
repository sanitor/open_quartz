use super::*;
use crate::runtime::{DeliveryPolicy, OutputKey, OutputTransport};
use serde_json::json;

#[test]
fn host_forwards_dirty_video_unsubscribe_and_events() {
    let mut host = PlayerHost::new(RuntimeCapabilities { data_paths: vec![] });
    let graph: Graph = serde_json::from_value(json!({
        "nodes":[{"id":"math","type":"math","position":{"x":0.0,"y":0.0},"data":{"type":"math","label":"Math","shaderCode":"","mathOp":"add","inputs":[],"outputs":[{"id":"out","label":"out","dataType":"float","direction":"output"}],"uniforms":{}}}],
        "edges":[]
    })).unwrap();
    host.set_graph(&graph).unwrap();
    host.mark_dirty("math").unwrap();
    host.set_video_nodes(&["math".to_owned()]).unwrap();
    host.subscribe_output(OutputSubscription {
        subscription_id: "subscription".to_owned(),
        output: OutputKey::new("math", "out"),
        delivery: DeliveryPolicy::Latest,
        transport: OutputTransport::Value,
        max_width: None,
        max_height: None,
    })
    .unwrap();
    assert_eq!(
        host.unsubscribe_output("subscription")
            .unwrap()
            .subscription_id,
        "subscription"
    );
    assert!(!host.drain_events().is_empty());
}
