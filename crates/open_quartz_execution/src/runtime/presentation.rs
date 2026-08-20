use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{
    ContentStamp, FrameStamp, OutputKey, PresentationFit, PresentationItem, PresentationSet,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationSubscription {
    pub subscription_id: String,
    pub output: OutputKey,
    pub group_id: String,
    pub viewport: Viewport,
    pub fit: PresentationFit,
    pub z_index: i32,
    pub resource_handle: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Default)]
pub struct PresentationPlanner {
    subscriptions: BTreeMap<String, PresentationSubscription>,
}

impl PresentationPlanner {
    pub fn subscribe(&mut self, subscription: PresentationSubscription) -> Result<(), String> {
        validate_subscription(&subscription)?;
        if self
            .subscriptions
            .contains_key(&subscription.subscription_id)
        {
            return Err("Presentation subscription ID is already registered".to_owned());
        }
        self.subscriptions
            .insert(subscription.subscription_id.clone(), subscription);
        Ok(())
    }

    pub fn update(&mut self, subscription: PresentationSubscription) -> Result<(), String> {
        if !self
            .subscriptions
            .contains_key(&subscription.subscription_id)
        {
            return Err("Presentation subscription is not registered".to_owned());
        }
        validate_subscription(&subscription)?;
        self.subscriptions
            .insert(subscription.subscription_id.clone(), subscription);
        Ok(())
    }

    pub fn unsubscribe(&mut self, subscription_id: &str) -> bool {
        self.subscriptions.remove(subscription_id).is_some()
    }

    pub fn reconcile(&mut self, outputs: &BTreeSet<OutputKey>, resource_handles: &BTreeSet<u64>) {
        self.subscriptions.retain(|_, subscription| {
            outputs.contains(&subscription.output)
                && resource_handles.contains(&subscription.resource_handle)
        });
    }

    pub fn build(
        &self,
        frame_stamp: FrameStamp,
        content: &BTreeMap<OutputKey, ContentStamp>,
    ) -> BTreeMap<String, PresentationSet> {
        let mut groups: BTreeMap<String, PresentationSet> = BTreeMap::new();
        for subscription in self.subscriptions.values() {
            let content_stamp =
                content
                    .get(&subscription.output)
                    .cloned()
                    .unwrap_or(ContentStamp {
                        epoch: frame_stamp.epoch,
                        timeline_ns: frame_stamp.timeline_ns,
                        media_pts_ns: None,
                    });
            groups
                .entry(subscription.group_id.clone())
                .or_insert_with(|| PresentationSet {
                    group_id: subscription.group_id.clone(),
                    frame_stamp: frame_stamp.clone(),
                    items: Vec::new(),
                })
                .items
                .push(PresentationItem {
                    output: subscription.output.clone(),
                    resource_handle: subscription.resource_handle,
                    viewport: super::contract::Viewport {
                        x: subscription.viewport.x,
                        y: subscription.viewport.y,
                        width: subscription.viewport.width,
                        height: subscription.viewport.height,
                    },
                    fit: subscription.fit,
                    z_index: subscription.z_index,
                    evaluation_stamp: frame_stamp.clone(),
                    content_stamp,
                });
        }
        for set in groups.values_mut() {
            set.items.sort_by_key(|item| item.z_index);
        }
        groups
    }
}

fn validate_subscription(subscription: &PresentationSubscription) -> Result<(), String> {
    if subscription.subscription_id.is_empty()
        || subscription.group_id.is_empty()
        || subscription.output.node_id.is_empty()
        || subscription.output.port_id.is_empty()
        || subscription.resource_handle == 0
        || !subscription.viewport.x.is_finite()
        || !subscription.viewport.y.is_finite()
        || !subscription.viewport.width.is_finite()
        || !subscription.viewport.height.is_finite()
        || subscription.viewport.width <= 0.0
        || subscription.viewport.height <= 0.0
    {
        Err("Invalid presentation subscription".to_owned())
    } else {
        Ok(())
    }
}
