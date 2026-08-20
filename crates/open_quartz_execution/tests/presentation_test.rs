use open_quartz_execution::runtime::{
    ContentStamp, FrameStamp, OutputKey, PresentationFit, PresentationPlanner,
    PresentationSubscription, PresentationViewport,
};
use std::collections::BTreeMap;

#[test]
fn planner_batches_multiple_renderer_outputs_atomically_by_group_and_z_order() {
    let mut planner = PresentationPlanner::default();
    let viewport = PresentationViewport {
        x: 0.0,
        y: 0.0,
        width: 640.0,
        height: 360.0,
    };
    planner
        .subscribe(PresentationSubscription {
            subscription_id: "renderer-b".to_owned(),
            output: OutputKey::new("shader-b", "image"),
            group_id: "main".to_owned(),
            viewport,
            fit: PresentationFit::Contain,
            z_index: 2,
            resource_handle: 2,
        })
        .unwrap();
    planner
        .subscribe(PresentationSubscription {
            subscription_id: "renderer-a".to_owned(),
            output: OutputKey::new("shader-a", "image"),
            group_id: "main".to_owned(),
            viewport,
            fit: PresentationFit::Cover,
            z_index: 1,
            resource_handle: 1,
        })
        .unwrap();

    let frame = FrameStamp {
        epoch: 4,
        frame: 10,
        timeline_ns: 100,
        deadline_ns: 116,
    };
    let mut content = BTreeMap::new();
    content.insert(
        OutputKey::new("shader-b", "image"),
        ContentStamp {
            epoch: 4,
            timeline_ns: 97,
            media_pts_ns: Some(97),
        },
    );
    let groups = planner.build(frame.clone(), &content);
    let set = &groups["main"];
    assert_eq!(set.frame_stamp, frame);
    assert_eq!(set.items.len(), 2);
    assert_eq!(set.items[0].resource_handle, 1);
    assert_eq!(set.items[1].resource_handle, 2);
    assert_eq!(set.items[1].content_stamp.timeline_ns, 97);
}

#[test]
fn planner_rejects_invalid_subscriptions_and_supports_updates() {
    let mut planner = PresentationPlanner::default();
    let mut subscription = PresentationSubscription {
        subscription_id: "renderer".to_owned(),
        output: OutputKey::new("node", "image"),
        group_id: "main".to_owned(),
        viewport: PresentationViewport {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        },
        fit: PresentationFit::Stretch,
        z_index: 0,
        resource_handle: 1,
    };
    planner.subscribe(subscription.clone()).unwrap();
    assert!(planner.subscribe(subscription.clone()).is_err());
    subscription.z_index = 4;
    planner.update(subscription).unwrap();
    subscription = PresentationSubscription {
        subscription_id: "renderer".to_owned(),
        output: OutputKey::new("node", "image"),
        group_id: "main".to_owned(),
        viewport: PresentationViewport {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 100.0,
        },
        fit: PresentationFit::Stretch,
        z_index: 4,
        resource_handle: 1,
    };
    assert!(planner.update(subscription).is_err());
    assert!(planner.unsubscribe("renderer"));
    assert!(!planner.unsubscribe("renderer"));
}
