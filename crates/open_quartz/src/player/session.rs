use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::compiler::ExecutionPlan;
use crate::evaluation::{ExecutionCommand, GraphInstance, GpuFacade};
use crate::evaluation::{EngineEvent, EngineState};
use crate::outputs::{
    AsyncCompletionEnvelope, ContentStamp, FrameStamp, HostBackend, OutputDeliveryBatch,
    OutputKey, OutputPayload, OutputState, OutputSubscription, OutputHub, PlaybackCapabilities,
    PresentationSet, PresentationSubscription, ResourceDescriptor,
};
use crate::project::model::{Graph, SdkError, SdkErrorCode};

use super::{ClockState, CompositionClock};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionFrameInput {
    pub now_ns: u64,
    pub date: [f32; 4],
    pub mouse: [f32; 4],
    pub resolution: [f32; 3],
}

pub struct PlaybackSession {
    graph: GraphInstance,
    clock: CompositionClock,
    hub: OutputHub,
}

impl PlaybackSession {
    pub fn new(capabilities: PlaybackCapabilities) -> Self {
        Self::with_graph(capabilities, GraphInstance::new())
    }

    pub fn new_native(capabilities: PlaybackCapabilities) -> Self {
        Self::with_graph(capabilities, GraphInstance::new_native())
    }

    fn with_graph(capabilities: PlaybackCapabilities, graph: GraphInstance) -> Self {
        Self {
            graph,
            clock: CompositionClock::new(16_666_667),
            hub: OutputHub::new(capabilities),
        }
    }

    pub fn set_graph(&mut self, graph: &Graph) -> Result<u32, SdkError> {
        let revision = self.graph.set_graph(graph.clone())?;
        let contracts = graph.nodes.iter().flat_map(|node| {
            node.data.outputs.iter().map(|port| {
                (OutputKey::new(&node.id, &port.id), port.data_type)
            })
        }).collect();
        self.hub.reconcile(revision, contracts);
        Ok(revision)
    }

    /// Checks identity, output contracts, and current delivery capacity without changing state.
    /// Submission checks the same policies again against the state at publication.
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
            ).for_node(&completion.node_id));
        }
        if completion.node_generation != self.node_generation(&completion.node_id)? {
            return Err(SdkError::new(
                SdkErrorCode::InvalidResource,
                "Async completion node generation is stale",
            ).for_node(&completion.node_id));
        }
        Ok(())
    }

    pub fn submit_completion(
        &mut self,
        completion: AsyncCompletionEnvelope,
    ) -> Result<(), SdkError> {
        self.validate_completion_identity(&completion)?;
        self.hub.publish_completion(&completion)?;
        self.graph.mark_dependents_dirty(&completion.node_id)
            .expect("validated completion node is prepared");
        self.hub.dispatch_presentations(completion.input_stamp)
    }

    pub fn set_video_nodes(&mut self, node_ids: &[String]) -> Result<(), SdkError> {
        self.graph.set_video_nodes(node_ids).map_err(decode_engine_error)
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
        self.clock.stop();
        Ok(())
    }

    pub fn play(&mut self, now_ns: u64) -> Result<(), SdkError> {
        if self.state() != EngineState::Ready {
            return Err(SdkError::new(
                SdkErrorCode::InvalidState,
                "PlaybackSession can only play from the ready state",
            ));
        }
        self.clock.start(now_ns);
        Ok(())
    }

    pub fn pause(&mut self, now_ns: u64) -> Result<(), SdkError> {
        if self.state() != EngineState::Running {
            return Err(SdkError::new(
                SdkErrorCode::InvalidState,
                "PlaybackSession can only pause while running",
            ));
        }
        self.clock
            .pause(now_ns)
            .expect("running session owns a running clock");
        self.graph.pause().map_err(decode_engine_error)
    }

    pub fn resume(&mut self, now_ns: u64) -> Result<(), SdkError> {
        if self.state() != EngineState::Paused {
            return Err(SdkError::new(
                SdkErrorCode::InvalidState,
                "PlaybackSession can only resume while paused",
            ));
        }
        self.clock
            .resume(now_ns)
            .expect("paused session owns a paused clock");
        self.graph.resume().map_err(decode_engine_error)
    }

    pub fn advance(&mut self, input: &SessionFrameInput) -> Result<ClockState, SdkError> {
        crate::player::logging::sdk_log(
            "debug",
            "session-advance",
            &format!(
                "{{\"revision\":{},\"nowNs\":{}}}",
                self.revision(),
                input.now_ns
            ),
        );
        if !matches!(self.state(), EngineState::Ready | EngineState::Running) {
            return Err(SdkError::new(
                SdkErrorCode::InvalidState,
                "PlaybackSession cannot advance in its current state",
            ));
        }
        // Rejected output batches must leave the clock and dirty graph work retryable.
        let mut candidate_clock = self.clock.clone();
        let clock = candidate_clock.tick(input.now_ns)?;
        let mut candidate_graph = self.graph.clone();
        let time = clock.timeline_ns as f64 / 1_000_000_000.0;
        let delta =
            clock.timeline_ns.saturating_sub(clock.previous_timeline_ns) as f64 / 1_000_000_000.0;
        candidate_graph
            .run_frame(
                time,
                delta,
                clock.frame,
                &input.date,
                &input.mouse,
                &input.resolution,
            )
            .expect("typed SessionFrameInput and prepared session are always valid");
        let stamp = FrameStamp {
            epoch: clock.epoch,
            frame: clock.frame,
            timeline_ns: clock.timeline_ns,
            deadline_ns: clock.next_deadline_ns,
        };
        let values = candidate_graph.pending_commands().iter().filter_map(|command| {
            let value = command.scalar_output?;
            let port_id = command.output_port_id.as_ref()?;
            Some((OutputKey::new(&command.node_id, port_id), OutputPayload::Float(value)))
        }).collect();
        self.hub.publish_values(
            candidate_graph.revision(),
            stamp.clone(),
            ContentStamp {
                epoch: stamp.epoch,
                timeline_ns: stamp.timeline_ns,
                media_pts_ns: None,
            },
            values,
        )?;
        self.graph = candidate_graph;
        self.clock = candidate_clock;
        self.hub.dispatch_presentations(stamp)?;
        crate::player::logging::sdk_log(
            "debug",
            "session-work",
            &format!(
                "{{\"frame\":{},\"commands\":{}}}",
                clock.frame,
                self.graph.pending_command_count()
            ),
        );
        Ok(clock)
    }

    pub fn drain_work(&mut self) -> Result<String, SdkError> {
        Ok(serde_json::to_string(&self.graph.drain_commands())
            .expect("ExecutionCommand serialization is infallible"))
    }

    pub fn stop(&mut self) -> Result<(), SdkError> {
        self.graph.stop().map_err(decode_engine_error)?;
        self.clock.stop();
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

    pub fn register_resource(&mut self, descriptor: ResourceDescriptor, handle: u64) -> Result<(), SdkError> {
        self.hub.register_resource(descriptor, handle)
    }

    pub fn remove_resource(&mut self, resource_id: &str) -> Result<u64, SdkError> {
        self.hub.remove_resource(resource_id)
    }

    pub fn subscribe_output(&mut self, subscription: OutputSubscription) -> Result<(), SdkError> {
        self.hub.subscribe_output(subscription)
    }

    pub fn update_output_subscription(&mut self, subscription: OutputSubscription) -> Result<(), SdkError> {
        self.hub.update_output_subscription(subscription)
    }

    pub fn unsubscribe_output(&mut self, subscription_id: &str) -> Result<OutputSubscription, SdkError> {
        self.hub.unsubscribe_output(subscription_id)
    }

    pub fn publish_output(&mut self, state: OutputState) -> Result<(), SdkError> {
        self.hub.publish_output(state)
    }

    pub fn drain_deliveries(&mut self) -> OutputDeliveryBatch {
        self.hub.drain_deliveries()
    }

    pub fn subscribe_presentation(&mut self, subscription: PresentationSubscription) -> Result<(), SdkError> {
        self.hub.subscribe_presentation(subscription)
    }

    pub fn update_presentation_subscription(&mut self, subscription: PresentationSubscription) -> Result<(), SdkError> {
        self.hub.update_presentation_subscription(subscription)
    }

    pub fn unsubscribe_presentation(&mut self, subscription_id: &str) -> bool {
        self.hub.unsubscribe_presentation(subscription_id)
    }

    pub fn build_presentation(&self, frame_stamp: FrameStamp, content: &BTreeMap<OutputKey, ContentStamp>) -> BTreeMap<String, PresentationSet> {
        self.hub.build_presentation(frame_stamp, content)
    }

    pub fn set_every_queue_capacity(&mut self, capacity: usize) {
        self.hub.set_every_queue_capacity(capacity);
    }

}

fn decode_engine_error(error: String) -> SdkError {
    serde_json::from_str(&error).unwrap_or_else(|_| {
        SdkError::new(SdkErrorCode::InvalidState, "Engine operation failed").with_details(error)
    })
}

#[cfg(test)]
#[path = "session_coverage_tests.rs"]
mod coverage_tests;
