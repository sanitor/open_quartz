use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::project::model::{DataType, SdkError, SdkErrorCode};

use super::{
    AsyncCompletionEnvelope, ContentStamp, FrameStamp, OutputDeliveryBatch, OutputKey,
    OutputPayload, OutputRegistry, OutputState, OutputSubscription, PlaybackCapabilities,
    PresentationPlanner, PresentationSet, PresentationSubscription,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceDescriptor {
    pub resource_id: String,
    pub kind: String,
}

pub trait HostBackend: Send {
    fn capabilities(&self) -> PlaybackCapabilities;
    fn register_resource(
        &mut self,
        descriptor: &ResourceDescriptor,
        handle: u64,
    ) -> Result<(), SdkError>;
    fn remove_resource(&mut self, resource_id: &str, handle: u64) -> Result<(), SdkError>;
    fn present(&mut self, set: &PresentationSet) -> Result<(), SdkError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RegisteredResource {
    descriptor: ResourceDescriptor,
    handle: u64,
}

pub struct OutputHub {
    resources: BTreeMap<String, RegisteredResource>,
    outputs: OutputRegistry,
    presentation: PresentationPlanner,
    backend: Option<Box<dyn HostBackend>>,
    capabilities: PlaybackCapabilities,
}

impl OutputHub {
    pub fn new(capabilities: PlaybackCapabilities) -> Self {
        Self {
            resources: BTreeMap::new(),
            outputs: OutputRegistry::default(),
            presentation: PresentationPlanner::default(),
            backend: None,
            capabilities,
        }
    }

    pub(crate) fn reconcile(&mut self, revision: u32, contracts: BTreeMap<OutputKey, DataType>) {
        self.outputs.reconcile(revision, contracts);
        self.reconcile_presentations();
    }

    pub(crate) fn validate_completion(
        &self,
        completion: &AsyncCompletionEnvelope,
    ) -> Result<(), SdkError> {
        self.outputs.validate_batch(&self.completion_states(completion)?)
    }

    pub(crate) fn publish_completion(
        &mut self,
        completion: &AsyncCompletionEnvelope,
    ) -> Result<(), SdkError> {
        self.outputs.publish_batch(self.completion_states(completion)?)
    }

    fn completion_states(
        &self,
        completion: &AsyncCompletionEnvelope,
    ) -> Result<Vec<OutputState>, SdkError> {
        if completion.graph_revision != self.outputs.revision() {
            return Err(SdkError::new(
                SdkErrorCode::InvalidResource,
                "Async completion graph revision is stale",
            )
            .for_node(&completion.node_id));
        }
        for (output, payload) in &completion.outputs {
            if output.node_id != completion.node_id {
                return Err(SdkError::new(
                    SdkErrorCode::InvalidResource,
                    "Async completion contains an output from another node",
                )
                .for_node(&completion.node_id));
            }
            self.outputs.validate_contract(output, payload)?;
        }
        Ok(self.output_states(
            completion.graph_revision,
            &completion.input_stamp,
            &completion.content_stamp,
            &completion.outputs,
        ))
    }

    fn output_states(
        &self,
        graph_revision: u32,
        evaluation_stamp: &FrameStamp,
        content_stamp: &ContentStamp,
        outputs: &[(OutputKey, OutputPayload)],
    ) -> Vec<OutputState> {
        let mut generations = BTreeMap::<OutputKey, u64>::new();
        let mut states = Vec::with_capacity(outputs.len());
        for (output, payload) in outputs {
            let previous = generations.get(output).copied().unwrap_or_else(|| {
                self.outputs.state(output).map(|state| state.output_generation).unwrap_or(0)
            });
            let output_generation = previous.saturating_add(1);
            generations.insert(output.clone(), output_generation);
            states.push(OutputState {
                output: output.clone(),
                graph_revision,
                output_generation,
                evaluation_stamp: evaluation_stamp.clone(),
                content_stamp: content_stamp.clone(),
                payload: payload.clone(),
            });
        }
        states
    }

    pub(crate) fn dispose(&mut self) -> Result<(), SdkError> {
        let resource_ids = self.resources.keys().cloned().collect::<Vec<_>>();
        for resource_id in resource_ids {
            self.remove_resource(&resource_id)?;
        }
        self.outputs = OutputRegistry::default();
        self.presentation = PresentationPlanner::default();
        Ok(())
    }

    pub fn attach_backend(&mut self, backend: Box<dyn HostBackend>) {
        self.capabilities = backend.capabilities();
        self.backend = Some(backend);
    }

    pub fn capabilities(&self) -> &PlaybackCapabilities {
        &self.capabilities
    }

    pub fn register_resource(
        &mut self,
        descriptor: ResourceDescriptor,
        handle: u64,
    ) -> Result<(), SdkError> {
        if descriptor.resource_id.is_empty() || descriptor.kind.is_empty() || handle == 0 {
            return Err(SdkError::new(
                SdkErrorCode::InvalidResource,
                "Resource registration requires a non-empty ID, kind, and non-zero handle",
            ));
        }
        if self.resources.contains_key(&descriptor.resource_id) {
            return Err(SdkError::new(
                SdkErrorCode::InvalidResource,
                "Resource ID is already registered",
            )
            .with_details(descriptor.resource_id));
        }
        if let Some(backend) = self.backend.as_mut() {
            backend.register_resource(&descriptor, handle)?;
        }
        self.resources.insert(
            descriptor.resource_id.clone(),
            RegisteredResource { descriptor, handle },
        );
        Ok(())
    }

    pub fn remove_resource(&mut self, resource_id: &str) -> Result<u64, SdkError> {
        let resource = self.resources.get(resource_id).ok_or_else(|| {
            SdkError::new(SdkErrorCode::InvalidResource, "Resource is not registered")
                .with_details(resource_id)
        })?;
        if let Some(backend) = self.backend.as_mut() {
            backend.remove_resource(resource_id, resource.handle)?;
        }
        let handle = resource.handle;
        self.resources.remove(resource_id);
        self.reconcile_presentations();
        Ok(handle)
    }

    pub fn subscribe_output(&mut self, subscription: OutputSubscription) -> Result<(), SdkError> {
        self.outputs.subscribe(subscription)
    }

    pub fn update_output_subscription(
        &mut self,
        subscription: OutputSubscription,
    ) -> Result<(), SdkError> {
        self.outputs.update(subscription)
    }

    pub fn unsubscribe_output(
        &mut self,
        subscription_id: &str,
    ) -> Result<OutputSubscription, SdkError> {
        self.outputs.unsubscribe(subscription_id)
    }

    pub fn publish_output(&mut self, state: OutputState) -> Result<(), SdkError> {
        self.outputs.publish(state)
    }

    pub fn drain_deliveries(&mut self) -> OutputDeliveryBatch {
        self.outputs.drain()
    }

    pub fn subscribe_presentation(
        &mut self,
        subscription: PresentationSubscription,
    ) -> Result<(), SdkError> {
        self.validate_presentation_references(&subscription)?;
        self.presentation
            .subscribe(subscription)
            .map_err(|message| SdkError::new(SdkErrorCode::InvalidResource, message))
    }

    pub fn update_presentation_subscription(
        &mut self,
        subscription: PresentationSubscription,
    ) -> Result<(), SdkError> {
        self.validate_presentation_references(&subscription)?;
        self.presentation
            .update(subscription)
            .map_err(|message| SdkError::new(SdkErrorCode::InvalidResource, message))
    }

    pub fn unsubscribe_presentation(&mut self, subscription_id: &str) -> bool {
        self.presentation.unsubscribe(subscription_id)
    }

    pub fn build_presentation(
        &self,
        frame_stamp: FrameStamp,
        content: &BTreeMap<OutputKey, ContentStamp>,
    ) -> BTreeMap<String, PresentationSet> {
        self.presentation.build(frame_stamp, content)
    }

    pub fn set_every_queue_capacity(&mut self, capacity: usize) {
        self.outputs.set_every_queue_capacity(capacity);
    }

    pub(crate) fn publish_values(
        &mut self,
        graph_revision: u32,
        evaluation_stamp: FrameStamp,
        content_stamp: ContentStamp,
        values: Vec<(OutputKey, OutputPayload)>,
    ) -> Result<(), SdkError> {
        let states = self.output_states(
            graph_revision,
            &evaluation_stamp,
            &content_stamp,
            &values,
        );
        self.outputs.publish_batch(states)
    }

    fn validate_presentation_references(
        &self,
        subscription: &PresentationSubscription,
    ) -> Result<(), SdkError> {
        if !self.outputs.contains_output(&subscription.output) {
            return Err(SdkError::new(
                SdkErrorCode::InvalidResource,
                "Presentation references an unknown output",
            ));
        }
        if !self
            .resources
            .values()
            .any(|resource| resource.handle == subscription.resource_handle)
        {
            return Err(SdkError::new(
                SdkErrorCode::InvalidResource,
                "Presentation references an unknown resource handle",
            ));
        }
        Ok(())
    }

    fn reconcile_presentations(&mut self) {
        let outputs = self
            .outputs
            .output_keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        let handles = self
            .resources
            .values()
            .map(|resource| resource.handle)
            .collect::<BTreeSet<_>>();
        self.presentation.reconcile(&outputs, &handles);
    }

    pub(crate) fn dispatch_presentations(&mut self, frame_stamp: FrameStamp) -> Result<(), SdkError> {
        let Some(backend) = self.backend.as_mut() else {
            return Ok(());
        };
        let sets = self
            .presentation
            .build(frame_stamp, &self.outputs.content_stamps());
        for set in sets.values() {
            backend.present(set)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "hub_coverage_tests.rs"]
mod coverage_tests;
