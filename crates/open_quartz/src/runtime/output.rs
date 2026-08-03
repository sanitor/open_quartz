use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::ffi::{SdkError, SdkErrorCode};

use super::{
    DeliveryPolicy, FrameStamp, OutputDelivery, OutputDeliveryBatch, OutputKey, OutputState,
    OutputSubscription, SubscriptionInvalidation,
};

const DEFAULT_EVERY_QUEUE_CAPACITY: usize = 60;

pub struct OutputRegistry {
    graph_revision: u32,
    valid_outputs: BTreeSet<OutputKey>,
    states: BTreeMap<OutputKey, OutputState>,
    subscriptions: BTreeMap<String, OutputSubscription>,
    pending_latest: BTreeMap<String, OutputDelivery>,
    pending_every: VecDeque<OutputDelivery>,
    last_enqueued_generation: BTreeMap<String, u64>,
    invalidations: Vec<SubscriptionInvalidation>,
    latest_frame_stamp: Option<FrameStamp>,
    every_queue_capacity: usize,
}

impl Default for OutputRegistry {
    fn default() -> Self {
        Self {
            graph_revision: 0,
            valid_outputs: BTreeSet::new(),
            states: BTreeMap::new(),
            subscriptions: BTreeMap::new(),
            pending_latest: BTreeMap::new(),
            pending_every: VecDeque::new(),
            last_enqueued_generation: BTreeMap::new(),
            invalidations: Vec::new(),
            latest_frame_stamp: None,
            every_queue_capacity: DEFAULT_EVERY_QUEUE_CAPACITY,
        }
    }
}

impl OutputRegistry {
    pub fn reconcile(&mut self, graph_revision: u32, outputs: BTreeSet<OutputKey>) {
        let removed = self
            .subscriptions
            .iter()
            .filter(|(_, subscription)| !outputs.contains(&subscription.output))
            .map(|(id, subscription)| (id.clone(), subscription.output.clone()))
            .collect::<Vec<_>>();
        self.graph_revision = graph_revision;
        self.valid_outputs = outputs;
        self.states.clear();
        self.pending_latest.clear();
        self.pending_every.clear();
        self.last_enqueued_generation.clear();
        self.latest_frame_stamp = None;
        for (subscription_id, output) in removed {
            self.subscriptions.remove(&subscription_id);
            self.invalidations.push(SubscriptionInvalidation {
                subscription_id,
                output,
                reason: "output-removed".to_owned(),
            });
        }
    }

    pub fn set_every_queue_capacity(&mut self, capacity: usize) {
        self.every_queue_capacity = capacity.max(1);
    }

    pub fn subscribe(&mut self, subscription: OutputSubscription) -> Result<(), SdkError> {
        validate_subscription(&subscription)?;
        if !self.valid_outputs.contains(&subscription.output) {
            return Err(SdkError::new(
                SdkErrorCode::InvalidResource,
                "Output subscription references an unknown port",
            )
            .for_node(&subscription.output.node_id)
            .with_details(&subscription.output.port_id));
        }
        if self
            .subscriptions
            .contains_key(&subscription.subscription_id)
        {
            return Err(SdkError::new(
                SdkErrorCode::InvalidResource,
                "Output subscription ID is already registered",
            )
            .with_details(subscription.subscription_id));
        }
        self.subscriptions
            .insert(subscription.subscription_id.clone(), subscription);
        Ok(())
    }

    pub fn update(&mut self, subscription: OutputSubscription) -> Result<(), SdkError> {
        if !self
            .subscriptions
            .contains_key(&subscription.subscription_id)
        {
            return Err(SdkError::new(
                SdkErrorCode::InvalidResource,
                "Output subscription is not registered",
            )
            .with_details(subscription.subscription_id));
        }
        validate_subscription(&subscription)?;
        if !self.valid_outputs.contains(&subscription.output) {
            return Err(SdkError::new(
                SdkErrorCode::InvalidResource,
                "Output subscription references an unknown port",
            )
            .for_node(&subscription.output.node_id)
            .with_details(&subscription.output.port_id));
        }
        let id = subscription.subscription_id.clone();
        self.subscriptions.insert(id.clone(), subscription);
        self.pending_latest.remove(&id);
        self.pending_every
            .retain(|delivery| delivery.subscription_id != id);
        self.last_enqueued_generation.remove(&id);
        Ok(())
    }

    pub fn unsubscribe(&mut self, subscription_id: &str) -> Result<OutputSubscription, SdkError> {
        let subscription = self.subscriptions.remove(subscription_id).ok_or_else(|| {
            SdkError::new(
                SdkErrorCode::InvalidResource,
                "Output subscription is not registered",
            )
            .with_details(subscription_id)
        })?;
        self.pending_latest.remove(subscription_id);
        self.pending_every
            .retain(|delivery| delivery.subscription_id != subscription_id);
        self.last_enqueued_generation.remove(subscription_id);
        Ok(subscription)
    }

    pub fn publish(&mut self, state: OutputState) -> Result<(), SdkError> {
        if state.graph_revision != self.graph_revision {
            return Err(SdkError::new(
                SdkErrorCode::InvalidResource,
                "Output state graph revision is stale",
            )
            .for_node(&state.output.node_id));
        }
        if !self.valid_outputs.contains(&state.output) {
            return Err(SdkError::new(
                SdkErrorCode::InvalidResource,
                "Output state references an unknown port",
            )
            .for_node(&state.output.node_id)
            .with_details(&state.output.port_id));
        }
        if self
            .states
            .get(&state.output)
            .is_some_and(|previous| state.output_generation <= previous.output_generation)
        {
            return Err(SdkError::new(
                SdkErrorCode::InvalidResource,
                "Output generation must increase",
            )
            .for_node(&state.output.node_id));
        }

        let matching = self
            .subscriptions
            .values()
            .filter(|subscription| subscription.output == state.output)
            .cloned()
            .collect::<Vec<_>>();
        for subscription in matching
            .iter()
            .filter(|subscription| subscription.delivery == DeliveryPolicy::Every)
        {
            let queued = self
                .pending_every
                .iter()
                .filter(|delivery| delivery.subscription_id == subscription.subscription_id)
                .count();
            if queued >= self.every_queue_capacity {
                return Err(SdkError::new(
                    SdkErrorCode::InvalidState,
                    "Output subscription backpressure",
                )
                .with_details(subscription.subscription_id.clone()));
            }
        }

        self.states.insert(state.output.clone(), state.clone());
        self.latest_frame_stamp = Some(state.evaluation_stamp.clone());
        for subscription in matching {
            let delivery = OutputDelivery {
                subscription_id: subscription.subscription_id.clone(),
                state: state.clone(),
            };
            match subscription.delivery {
                DeliveryPolicy::Every => self.pending_every.push_back(delivery),
                DeliveryPolicy::Latest => {
                    self.pending_latest
                        .insert(subscription.subscription_id.clone(), delivery);
                }
                DeliveryPolicy::OnChange => {
                    let last = self
                        .last_enqueued_generation
                        .get(&subscription.subscription_id)
                        .copied()
                        .unwrap_or(0);
                    if state.output_generation > last {
                        self.pending_latest
                            .insert(subscription.subscription_id.clone(), delivery);
                        self.last_enqueued_generation
                            .insert(subscription.subscription_id, state.output_generation);
                    }
                }
            }
        }
        Ok(())
    }

    pub fn state(&self, output: &OutputKey) -> Option<&OutputState> {
        self.states.get(output)
    }

    pub fn drain(&mut self) -> OutputDeliveryBatch {
        let mut deliveries = self.pending_every.drain(..).collect::<Vec<_>>();
        deliveries.extend(std::mem::take(&mut self.pending_latest).into_values());
        deliveries.sort_by(|left, right| {
            left.state
                .evaluation_stamp
                .frame
                .cmp(&right.state.evaluation_stamp.frame)
                .then_with(|| left.subscription_id.cmp(&right.subscription_id))
        });
        OutputDeliveryBatch {
            frame_stamp: self.latest_frame_stamp.take(),
            deliveries,
            invalidations: std::mem::take(&mut self.invalidations),
        }
    }
}

fn validate_subscription(subscription: &OutputSubscription) -> Result<(), SdkError> {
    if subscription.subscription_id.is_empty()
        || subscription.output.node_id.is_empty()
        || subscription.output.port_id.is_empty()
    {
        return Err(SdkError::new(
            SdkErrorCode::InvalidResource,
            "Output subscription requires non-empty IDs",
        ));
    }
    if matches!(subscription.max_width, Some(0)) || matches!(subscription.max_height, Some(0)) {
        return Err(SdkError::new(
            SdkErrorCode::InvalidResource,
            "Output subscription dimensions must be positive",
        ));
    }
    Ok(())
}
