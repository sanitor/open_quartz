use std::collections::{BTreeMap, VecDeque};

use crate::project::model::DataType;
use crate::project::model::{SdkError, SdkErrorCode};

use super::{
    DeliveryPolicy, FrameStamp, OutputDelivery, OutputDeliveryBatch, OutputKey, OutputPayload,
    OutputState, OutputSubscription, OutputTransport, SubscriptionInvalidation,
};

const DEFAULT_EVERY_QUEUE_CAPACITY: usize = 60;

pub struct OutputRegistry {
    graph_revision: u32,
    valid_outputs: BTreeMap<OutputKey, DataType>,
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
            valid_outputs: BTreeMap::new(),
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
    pub(crate) fn revision(&self) -> u32 {
        self.graph_revision
    }

    pub(crate) fn contains_output(&self, output: &OutputKey) -> bool {
        self.valid_outputs.contains_key(output)
    }

    pub(crate) fn output_keys(&self) -> impl Iterator<Item = &OutputKey> {
        self.valid_outputs.keys()
    }

    pub fn reconcile(&mut self, graph_revision: u32, outputs: BTreeMap<OutputKey, DataType>) {
        let invalidated = self
            .subscriptions
            .iter()
            .filter_map(|(id, subscription)| {
                let reason = match (
                    self.valid_outputs.get(&subscription.output),
                    outputs.get(&subscription.output),
                ) {
                    (_, None) => "output-removed",
                    (Some(previous), Some(next)) if previous != next => "output-contract-changed",
                    _ => return None,
                };
                Some((id.clone(), subscription.output.clone(), reason.to_owned()))
            })
            .collect::<Vec<_>>();
        self.graph_revision = graph_revision;
        self.valid_outputs = outputs;
        self.states.clear();
        self.pending_latest.clear();
        self.pending_every.clear();
        self.last_enqueued_generation.clear();
        self.latest_frame_stamp = None;
        for (subscription_id, output, reason) in invalidated {
            self.subscriptions.remove(&subscription_id);
            self.invalidations.push(SubscriptionInvalidation {
                subscription_id,
                output,
                reason,
            });
        }
    }

    pub fn set_every_queue_capacity(&mut self, capacity: usize) {
        self.every_queue_capacity = capacity.max(1);
    }

    pub fn subscribe(&mut self, subscription: OutputSubscription) -> Result<(), SdkError> {
        validate_subscription(&subscription)?;
        self.validate_transport(&subscription)?;
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
        self.validate_transport(&subscription)?;
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

    pub fn validate_contract(
        &self,
        output: &OutputKey,
        payload: &OutputPayload,
    ) -> Result<(), SdkError> {
        let Some(data_type) = self.valid_outputs.get(output).copied() else {
            return Err(SdkError::new(
                SdkErrorCode::InvalidResource,
                "Output state references an unknown port",
            )
            .for_node(&output.node_id)
            .with_details(&output.port_id));
        };
        if !payload_matches(data_type, payload) {
            return Err(SdkError::new(
                SdkErrorCode::InvalidResource,
                "Output payload does not match the port contract",
            )
            .for_node(&output.node_id)
            .with_details(&output.port_id));
        }
        Ok(())
    }

    pub fn publish(&mut self, state: OutputState) -> Result<(), SdkError> {
        self.publish_batch(vec![state])
    }

    /// Validate every state and delivery reservation without changing registry state.
    pub(crate) fn validate_batch(&self, states: &[OutputState]) -> Result<(), SdkError> {
        let mut generations = BTreeMap::<&OutputKey, u64>::new();
        let mut queued = BTreeMap::<&str, usize>::new();
        for delivery in &self.pending_every {
            *queued.entry(&delivery.subscription_id).or_default() += 1;
        }
        for state in states {
            if state.graph_revision != self.graph_revision {
                return Err(SdkError::new(
                    SdkErrorCode::InvalidResource,
                    "Output state graph revision is stale",
                )
                .for_node(&state.output.node_id));
            }
            self.validate_contract(&state.output, &state.payload)?;
            let previous = generations.get(&state.output).copied().or_else(|| {
                self.states.get(&state.output).map(|previous| previous.output_generation)
            });
            if previous.is_some_and(|previous| state.output_generation <= previous) {
                return Err(SdkError::new(
                    SdkErrorCode::InvalidResource,
                    "Output generation must increase",
                )
                .for_node(&state.output.node_id));
            }
            generations.insert(&state.output, state.output_generation);
            for subscription in self.subscriptions.values().filter(|subscription| {
                subscription.output == state.output && subscription.delivery == DeliveryPolicy::Every
            }) {
                let count = queued.entry(&subscription.subscription_id).or_default();
                if *count >= self.every_queue_capacity {
                    return Err(SdkError::new(
                        SdkErrorCode::InvalidState,
                        "Output subscription backpressure",
                    )
                    .with_details(subscription.subscription_id.clone()));
                }
                *count += 1;
            }
        }
        Ok(())
    }

    pub(crate) fn publish_batch(&mut self, states: Vec<OutputState>) -> Result<(), SdkError> {
        self.validate_batch(&states)?;
        // Validation and queue reservations are complete before this commit phase.
        for state in states {
            self.publish_validated(state);
        }
        Ok(())
    }

    fn publish_validated(&mut self, state: OutputState) {
        let matching = self.subscriptions.values()
            .filter(|subscription| subscription.output == state.output)
            .cloned()
            .collect::<Vec<_>>();
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
    }

    pub fn state(&self, output: &OutputKey) -> Option<&OutputState> {
        self.states.get(output)
    }

    pub fn content_stamps(&self) -> BTreeMap<OutputKey, super::ContentStamp> {
        self.states
            .iter()
            .map(|(output, state)| (output.clone(), state.content_stamp.clone()))
            .collect()
    }

    fn validate_transport(&self, subscription: &OutputSubscription) -> Result<(), SdkError> {
        let Some(data_type) = self.valid_outputs.get(&subscription.output).copied() else {
            return Err(SdkError::new(
                SdkErrorCode::InvalidResource,
                "Output subscription references an unknown port",
            )
            .for_node(&subscription.output.node_id)
            .with_details(&subscription.output.port_id));
        };
        let texture = matches!(data_type, DataType::Sampler2d | DataType::SamplerCube);
        let compatible = match subscription.transport {
            OutputTransport::Value => !texture,
            OutputTransport::Preview
            | OutputTransport::Capture
            | OutputTransport::NativePresent => texture,
        };
        if compatible {
            Ok(())
        } else {
            Err(SdkError::new(
                SdkErrorCode::InvalidResource,
                "Output transport does not match the port contract",
            )
            .for_node(&subscription.output.node_id)
            .with_details(&subscription.output.port_id))
        }
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

fn payload_matches(data_type: DataType, payload: &OutputPayload) -> bool {
    match data_type {
        DataType::Bool => matches!(payload, OutputPayload::Bool(_)),
        DataType::Int => matches!(payload, OutputPayload::Int(_)),
        DataType::Uint => matches!(payload, OutputPayload::Uint(_)),
        DataType::Float => matches!(payload, OutputPayload::Float(_)),
        DataType::Vec2
        | DataType::Vec3
        | DataType::Vec4
        | DataType::Ivec2
        | DataType::Ivec3
        | DataType::Ivec4
        | DataType::Uvec2
        | DataType::Uvec3
        | DataType::Uvec4
        | DataType::Bvec2
        | DataType::Bvec3
        | DataType::Bvec4
        | DataType::Mat2
        | DataType::Mat3
        | DataType::Mat4 => matches!(payload, OutputPayload::FloatArray(_)),
        DataType::Sampler2d | DataType::SamplerCube => {
            matches!(payload, OutputPayload::Resource { .. })
        }
        // Resident detections use a device buffer; host-side ROI values remain JSON.
        DataType::Roi => matches!(payload, OutputPayload::Resource { .. } | OutputPayload::Json(_)),
        DataType::Mesh | DataType::Json => {
            matches!(payload, OutputPayload::Json(_))
        }
        DataType::Auto => true,
    }
}

#[cfg(test)]
#[path = "registry_coverage_tests.rs"]
mod coverage_tests;
