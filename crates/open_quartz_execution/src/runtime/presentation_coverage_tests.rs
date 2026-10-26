use super::*;

fn subscription(id: &str) -> PresentationSubscription {
    PresentationSubscription {
        subscription_id: id.to_owned(),
        output: OutputKey::new("node", "port"),
        group_id: "group".to_owned(),
        viewport: Viewport {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
        fit: PresentationFit::Contain,
        z_index: 0,
        resource_handle: 7,
    }
}

#[test]
fn planner_validation_and_reconcile_keep_and_drop_paths() {
    let mut invalid = subscription("");
    assert!(validate_subscription(&invalid).is_err());
    assert!(PresentationPlanner::default()
        .subscribe(invalid.clone())
        .is_err());
    invalid = subscription("id");
    invalid.group_id.clear();
    assert!(validate_subscription(&invalid).is_err());
    invalid = subscription("id");
    invalid.viewport.width = 0.0;
    assert!(validate_subscription(&invalid).is_err());
    invalid = subscription("id");
    invalid.viewport.height = f32::NAN;
    assert!(validate_subscription(&invalid).is_err());
    invalid = subscription("id");
    invalid.resource_handle = 0;
    assert!(validate_subscription(&invalid).is_err());

    let mut planner = PresentationPlanner::default();
    planner.subscribe(subscription("keep")).unwrap();
    planner.reconcile(
        &BTreeSet::from([OutputKey::new("node", "port")]),
        &BTreeSet::from([7]),
    );
    assert!(planner.unsubscribe("keep"));
    planner.subscribe(subscription("drop")).unwrap();
    planner.reconcile(&BTreeSet::new(), &BTreeSet::from([7]));
    assert!(!planner.unsubscribe("drop"));
}
