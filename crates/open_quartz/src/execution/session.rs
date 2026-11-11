//! Clock-free graph state and typed output transactions shared by playback and GPU execution.
use std::collections::BTreeMap;

use crate::compiler::ExecutionPlan;
use crate::evaluation::{EngineEvent, EngineState};
use crate::evaluation::{ExecutionCommand, GpuFacade, GraphInstance};
use crate::outputs::{
    AsyncCompletionEnvelope, ContentStamp, FrameStamp, HostBackend, OutputDeliveryBatch, OutputHub,
    OutputKey, OutputPayload, OutputState, OutputSubscription, PlaybackCapabilities,
    PresentationSet, PresentationSubscription, ResourceDescriptor,
};
use crate::project::model::{Graph, SdkError, SdkErrorCode};

#[derive(Clone, Debug, PartialEq)]
pub struct FrameEvaluationInput {
    pub stamp: FrameStamp,
    pub previous_timeline_ns: u64,
    pub date: [f32; 4],
    pub mouse: [f32; 4],
    pub resolution: [f32; 3],
}

pub struct FrameExecutionSession {
    pub(crate) graph: GraphInstance,
    hub: OutputHub,
    last_evaluation: Option<FrameStamp>,
}

impl FrameExecutionSession {
    pub fn new(capabilities: PlaybackCapabilities) -> Self {
        Self::with_graph(capabilities, GraphInstance::new())
    }

    pub fn new_native(capabilities: PlaybackCapabilities) -> Self {
        Self::with_graph(capabilities, GraphInstance::new_native())
    }

    fn with_graph(capabilities: PlaybackCapabilities, graph: GraphInstance) -> Self {
        Self {
            graph,
            last_evaluation: None,
            hub: OutputHub::new(capabilities),
        }
    }

    pub fn set_graph(&mut self, graph: &Graph) -> Result<u32, SdkError> {
        let revision = self.graph.set_graph(graph.clone())?;
        let contracts = graph
            .nodes
            .iter()
            .flat_map(|node| {
                node.data
                    .outputs
                    .iter()
                    .map(|port| (OutputKey::new(&node.id, &port.id), port.data_type))
            })
            .collect();
        self.hub.reconcile(revision, contracts);
        self.last_evaluation = None;
        Ok(revision)
    }

    pub fn validate_completion(
        &self,
        completion: &AsyncCompletionEnvelope,
    ) -> Result<(), SdkError> {
        self.validate_completion_identity(completion)?;
        self.hub.validate_completion(completion)
    }

    fn validate_completion_identity(
        &self,
        completion: &AsyncCompletionEnvelope,
    ) -> Result<(), SdkError> {
        if completion.graph_revision != self.revision() {
            return Err(SdkError::new(
                SdkErrorCode::InvalidResource,
                "Async completion graph revision is stale",
            )
            .for_node(&completion.node_id));
        }
        if completion.node_generation != self.node_generation(&completion.node_id)? {
            return Err(SdkError::new(
                SdkErrorCode::InvalidResource,
                "Async completion node generation is stale",
            )
            .for_node(&completion.node_id));
        }
        if self
            .last_evaluation
            .as_ref()
            .is_some_and(|stamp| stamp.epoch != completion.input_stamp.epoch)
        {
            return Err(SdkError::new(
                SdkErrorCode::InvalidResource,
                "Async completion execution epoch is stale",
            )
            .for_node(&completion.node_id));
        }
        Ok(())
    }

    pub fn submit_completion(
        &mut self,
        completion: AsyncCompletionEnvelope,
    ) -> Result<(), SdkError> {
        self.validate_completion_identity(&completion)?;
        self.hub.publish_completion(&completion)?;
        self.graph
            .mark_dependents_dirty(&completion.node_id)
            .expect("validated completion node is prepared");
        self.hub.dispatch_presentations(completion.input_stamp)
    }

    pub fn set_video_nodes(&mut self, node_ids: &[String]) -> Result<(), SdkError> {
        self.graph
            .set_video_nodes(node_ids)
            .map_err(decode_engine_error)
    }

    pub fn drain_events(&mut self) -> Vec<EngineEvent> {
        self.graph.drain_events()
    }

    pub fn execute_gpu(
        &self,
        facade: &mut dyn GpuFacade,
        commands: &[ExecutionCommand],
    ) -> Result<(), SdkError> {
        let plan = self.execution_plan().ok_or_else(|| {
            SdkError::new(
                SdkErrorCode::NotPrepared,
                "Playback session must receive a graph before GPU execution",
            )
        })?;
        facade.execute(plan, commands)
    }

    pub fn dispose(&mut self) -> Result<(), SdkError> {
        self.hub.dispose()?;
        self.graph.dispose();
        self.last_evaluation = None;
        Ok(())
    }

    pub fn drain_work(&mut self) -> Result<String, SdkError> {
        Ok(serde_json::to_string(&self.graph.drain_commands())
            .expect("ExecutionCommand serialization is infallible"))
    }

    pub fn stop(&mut self) -> Result<(), SdkError> {
        self.graph.stop().map_err(decode_engine_error)?;
        self.graph.reset_temporal_state()?;
        self.last_evaluation = None;
        Ok(())
    }

    pub fn revision(&self) -> u32 {
        self.graph.revision()
    }

    pub fn node_generation(&self, node_id: &str) -> Result<u32, SdkError> {
        self.graph
            .node_generation(node_id)
            .map_err(decode_engine_error)
    }

    pub fn mark_dirty(&mut self, node_id: &str) -> Result<(), SdkError> {
        self.graph.mark_dirty(node_id).map_err(decode_engine_error)
    }

    pub fn execution_plan(&self) -> Option<&ExecutionPlan> {
        self.graph.execution_plan()
    }

    pub fn drain_commands(&mut self) -> Vec<ExecutionCommand> {
        self.graph.drain_commands()
    }

    pub fn state(&self) -> EngineState {
        self.graph.state_value()
    }

    pub fn attach_backend(&mut self, backend: Box<dyn HostBackend>) {
        self.hub.attach_backend(backend);
    }

    pub fn capabilities(&self) -> &PlaybackCapabilities {
        self.hub.capabilities()
    }

    pub fn register_resource(
        &mut self,
        descriptor: ResourceDescriptor,
        handle: u64,
    ) -> Result<(), SdkError> {
        self.hub.register_resource(descriptor, handle)
    }

    pub fn remove_resource(&mut self, resource_id: &str) -> Result<u64, SdkError> {
        self.hub.remove_resource(resource_id)
    }

    pub fn subscribe_output(&mut self, subscription: OutputSubscription) -> Result<(), SdkError> {
        self.hub.subscribe_output(subscription)
    }

    pub fn update_output_subscription(
        &mut self,
        subscription: OutputSubscription,
    ) -> Result<(), SdkError> {
        self.hub.update_output_subscription(subscription)
    }

    pub fn unsubscribe_output(
        &mut self,
        subscription_id: &str,
    ) -> Result<OutputSubscription, SdkError> {
        self.hub.unsubscribe_output(subscription_id)
    }

    pub fn publish_output(&mut self, state: OutputState) -> Result<(), SdkError> {
        self.hub.publish_output(state)
    }

    pub fn drain_deliveries(&mut self) -> OutputDeliveryBatch {
        self.hub.drain_deliveries()
    }

    #[cfg(all(feature = "native-runtime", not(target_arch = "wasm32")))]
    pub(crate) fn take_latest_delivery(&mut self, subscription_id: &str) -> bool {
        self.hub.take_latest_delivery(subscription_id)
    }

    pub fn subscribe_presentation(
        &mut self,
        subscription: PresentationSubscription,
    ) -> Result<(), SdkError> {
        self.hub.subscribe_presentation(subscription)
    }

    pub fn update_presentation_subscription(
        &mut self,
        subscription: PresentationSubscription,
    ) -> Result<(), SdkError> {
        self.hub.update_presentation_subscription(subscription)
    }

    pub fn unsubscribe_presentation(&mut self, subscription_id: &str) -> bool {
        self.hub.unsubscribe_presentation(subscription_id)
    }

    pub fn build_presentation(
        &self,
        frame_stamp: FrameStamp,
        content: &BTreeMap<OutputKey, ContentStamp>,
    ) -> BTreeMap<String, PresentationSet> {
        self.hub.build_presentation(frame_stamp, content)
    }

    pub fn set_every_queue_capacity(&mut self, capacity: usize) {
        self.hub.set_every_queue_capacity(capacity);
    }

    pub fn evaluate(&mut self, input: &FrameEvaluationInput) -> Result<(), SdkError> {
        self.evaluate_with_dirty(input, &[])
    }

    pub(crate) fn evaluate_with_dirty(
        &mut self,
        input: &FrameEvaluationInput,
        dirty: &[&str],
    ) -> Result<(), SdkError> {
        if !matches!(self.state(), EngineState::Ready | EngineState::Running) {
            return Err(SdkError::new(
                SdkErrorCode::InvalidState,
                "Frame execution requires a ready or running graph",
            ));
        }
        if input.stamp.frame == 0
            || input.previous_timeline_ns > input.stamp.timeline_ns
            || input.resolution[..2]
                .iter()
                .any(|v| !v.is_finite() || *v <= 0.0)
            || input
                .date
                .iter()
                .chain(input.mouse.iter())
                .chain(input.resolution.iter())
                .any(|v| !v.is_finite())
        {
            return Err(SdkError::new(
                SdkErrorCode::InvalidResource,
                "Invalid explicit frame inputs",
            ));
        }
        if self
            .last_evaluation
            .as_ref()
            .is_some_and(|stamp| input.stamp.epoch < stamp.epoch)
        {
            return Err(SdkError::new(
                SdkErrorCode::InvalidResource,
                "Explicit frame epoch is stale",
            ));
        }
        let mut candidate_graph = self.graph.clone();
        if self
            .last_evaluation
            .as_ref()
            .is_some_and(|stamp| input.stamp.epoch != stamp.epoch)
        {
            candidate_graph.reset_temporal_state()?;
        }
        for node in dirty {
            candidate_graph
                .mark_dirty(node)
                .map_err(decode_engine_error)?;
        }
        candidate_graph
            .run_frame(
                input.stamp.timeline_ns as f64 / 1_000_000_000.0,
                (input.stamp.timeline_ns - input.previous_timeline_ns) as f64 / 1_000_000_000.0,
                input.stamp.frame,
                &input.date,
                &input.mouse,
                &input.resolution,
            )
            .map_err(decode_engine_error)?;
        let values = candidate_graph
            .pending_commands()
            .iter()
            .filter_map(|command| {
                Some((
                    OutputKey::new(&command.node_id, command.output_port_id.as_ref()?),
                    OutputPayload::Float(command.scalar_output?),
                ))
            })
            .collect();
        self.hub.publish_values(
            candidate_graph.revision(),
            input.stamp.clone(),
            ContentStamp {
                epoch: input.stamp.epoch,
                timeline_ns: input.stamp.timeline_ns,
                media_pts_ns: None,
            },
            values,
        )?;
        self.graph = candidate_graph;
        self.last_evaluation = Some(input.stamp.clone());
        crate::player::logging::sdk_log(
            "debug",
            "session-work",
            &format!(
                "{{\"frame\":{},\"commands\":{}}}",
                input.stamp.frame,
                self.graph.pending_command_count()
            ),
        );
        self.hub.dispatch_presentations(input.stamp.clone())
    }

    pub fn last_evaluation(&self) -> Option<&FrameStamp> {
        self.last_evaluation.as_ref()
    }

    pub fn pause(&mut self) -> Result<(), SdkError> {
        self.graph.pause().map_err(decode_engine_error)
    }
    pub fn resume(&mut self) -> Result<(), SdkError> {
        self.graph.resume().map_err(decode_engine_error)
    }
}

pub(crate) fn decode_engine_error(error: String) -> SdkError {
    serde_json::from_str(&error).unwrap_or_else(|_| {
        SdkError::new(SdkErrorCode::InvalidState, "Engine operation failed").with_details(error)
    })
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
