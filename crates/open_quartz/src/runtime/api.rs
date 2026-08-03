use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::ffi::{Engine, EngineEvent, EngineState, SdkError, SdkErrorCode};
use crate::types::{DataType, Graph};

use super::{
    AsyncCompletionEnvelope, ClockState, CompositionClock, ContentStamp, FrameStamp,
    OutputDeliveryBatch, OutputKey, OutputPayload, OutputRegistry, OutputState, OutputSubscription,
    PresentationPlanner, PresentationSet, PresentationSubscription, RuntimeCapabilities,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceDescriptor {
    pub resource_id: String,
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeFrameInput {
    pub now_ns: u64,
    pub date: [f32; 4],
    pub mouse: [f32; 4],
    pub resolution: [f32; 3],
}

pub trait FramePacer {
    fn pacing_timestamp_ns(&mut self) -> Result<u64, SdkError>;
}

pub trait HostBackend {
    fn capabilities(&self) -> RuntimeCapabilities;
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

pub struct Runtime {
    engine: Engine,
    resources: BTreeMap<String, RegisteredResource>,
    output_contracts: BTreeMap<OutputKey, DataType>,
    outputs: OutputRegistry,
    clock: CompositionClock,
    presentation: PresentationPlanner,
    backend: Option<Box<dyn HostBackend>>,
    capabilities: RuntimeCapabilities,
}

impl Runtime {
    pub fn new(capabilities: RuntimeCapabilities) -> Self {
        Self {
            engine: Engine::new(),
            resources: BTreeMap::new(),
            output_contracts: BTreeMap::new(),
            outputs: OutputRegistry::default(),
            clock: CompositionClock::new(16_666_667),
            presentation: PresentationPlanner::default(),
            backend: None,
            capabilities,
        }
    }

    pub fn attach_backend(&mut self, backend: Box<dyn HostBackend>) {
        self.capabilities = backend.capabilities();
        self.backend = Some(backend);
    }
    pub fn play(&mut self, now_ns: u64) -> Result<(), SdkError> {
        if self.state() != EngineState::Ready {
            return Err(SdkError::new(
                SdkErrorCode::InvalidState,
                "Runtime can only play from the ready state",
            ));
        }
        self.clock.start(now_ns);
        Ok(())
    }

    pub fn pause(&mut self, now_ns: u64) -> Result<(), SdkError> {
        if self.state() != EngineState::Running {
            return Err(SdkError::new(
                SdkErrorCode::InvalidState,
                "Runtime can only pause while running",
            ));
        }
        self.clock.pause(now_ns)?;
        self.engine.pause().map_err(decode_engine_error)
    }

    pub fn resume(&mut self, now_ns: u64) -> Result<(), SdkError> {
        if self.state() != EngineState::Paused {
            return Err(SdkError::new(
                SdkErrorCode::InvalidState,
                "Runtime can only resume while paused",
            ));
        }
        self.clock.resume(now_ns)?;
        self.engine.resume().map_err(decode_engine_error)
    }

    pub fn capabilities(&self) -> &RuntimeCapabilities {
        &self.capabilities
    }

    pub fn set_graph(&mut self, graph: &Graph) -> Result<u32, SdkError> {
        let json = serde_json::to_string(graph).map_err(|error| {
            SdkError::new(SdkErrorCode::InvalidGraph, "Cannot serialize graph")
                .with_details(error.to_string())
        })?;
        let revision = self
            .engine
            .set_graph_json(&json)
            .map_err(decode_engine_error)?;
        self.output_contracts = graph
            .nodes
            .iter()
            .flat_map(|node| {
                node.data
                    .outputs
                    .iter()
                    .map(|port| (OutputKey::new(&node.id, &port.id), port.data_type))
            })
            .collect();
        self.outputs
            .reconcile(revision, self.output_contracts.clone());
        self.reconcile_presentations();
        Ok(revision)
    }

    pub fn advance(&mut self, input: &RuntimeFrameInput) -> Result<ClockState, SdkError> {
        if !matches!(self.state(), EngineState::Ready | EngineState::Running) {
            return Err(SdkError::new(
                SdkErrorCode::InvalidState,
                "Runtime cannot advance in its current state",
            ));
        }
        let clock = self.clock.tick(input.now_ns)?;
        let time = clock.timeline_ns as f64 / 1_000_000_000.0;
        let delta =
            clock.timeline_ns.saturating_sub(clock.previous_timeline_ns) as f64 / 1_000_000_000.0;
        self.engine
            .run_frame(
                time,
                delta,
                clock.frame,
                &input.date,
                &input.mouse,
                &input.resolution,
            )
            .map_err(decode_engine_error)?;
        let stamp = FrameStamp {
            epoch: clock.epoch,
            frame: clock.frame,
            timeline_ns: clock.timeline_ns,
            deadline_ns: clock.next_deadline_ns,
        };
        for command in self.engine.pending_commands().to_vec() {
            let (Some(value), Some(port_id)) = (command.scalar_output, command.output_port_id)
            else {
                continue;
            };
            let output = OutputKey::new(command.node_id, port_id);
            self.publish_state(
                output,
                stamp.clone(),
                ContentStamp {
                    epoch: stamp.epoch,
                    timeline_ns: stamp.timeline_ns,
                    media_pts_ns: None,
                },
                OutputPayload::Float(value),
            )?;
        }
        self.dispatch_presentations(stamp)?;
        Ok(clock)
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
    pub fn drain_work(&mut self) -> Result<String, SdkError> {
        serde_json::to_string(&self.engine.drain_commands()).map_err(|error| {
            SdkError::new(
                SdkErrorCode::InvalidResource,
                "Cannot serialize runtime work batch",
            )
            .with_details(error.to_string())
        })
    }
    pub fn submit_completion(
        &mut self,
        completion: AsyncCompletionEnvelope,
    ) -> Result<(), SdkError> {
        if completion.graph_revision != self.revision() {
            return Err(SdkError::new(
                SdkErrorCode::InvalidResource,
                "Async completion graph revision is stale",
            )
            .for_node(completion.node_id));
        }
        if completion.node_generation != self.node_generation(&completion.node_id)? {
            return Err(SdkError::new(
                SdkErrorCode::InvalidResource,
                "Async completion node generation is stale",
            )
            .for_node(completion.node_id));
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
        for (output, payload) in completion.outputs {
            self.publish_state(
                output,
                completion.input_stamp.clone(),
                completion.content_stamp.clone(),
                payload,
            )?;
        }
        self.dispatch_presentations(completion.input_stamp)
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

    pub fn stop(&mut self) -> Result<(), SdkError> {
        self.engine.stop().map_err(decode_engine_error)?;
        self.clock.stop();
        Ok(())
    }

    pub fn revision(&self) -> u32 {
        self.engine.revision()
    }

    pub fn node_generation(&self, node_id: &str) -> Result<u32, SdkError> {
        self.engine
            .node_generation(node_id)
            .map_err(decode_engine_error)
    }

    pub fn state(&self) -> EngineState {
        match self.engine.engine_state().as_str() {
            "empty" => EngineState::Empty,
            "ready" => EngineState::Ready,
            "running" => EngineState::Running,
            "paused" => EngineState::Paused,
            "stopped" => EngineState::Stopped,
            "disposed" => EngineState::Disposed,
            _ => unreachable!("Engine only returns known state names"),
        }
    }

    pub fn drain_events(&mut self) -> Vec<EngineEvent> {
        serde_json::from_str(&self.engine.drain_events_json())
            .expect("Engine events always use the canonical schema")
    }

    pub fn dispose(&mut self) -> Result<(), SdkError> {
        let resource_ids = self.resources.keys().cloned().collect::<Vec<_>>();
        for resource_id in resource_ids {
            self.remove_resource(&resource_id)?;
        }
        self.engine.dispose();
        self.output_contracts.clear();
        self.outputs = OutputRegistry::default();
        self.presentation = PresentationPlanner::default();
        self.clock.stop();
        Ok(())
    }

    fn publish_state(
        &mut self,
        output: OutputKey,
        evaluation_stamp: FrameStamp,
        content_stamp: ContentStamp,
        payload: OutputPayload,
    ) -> Result<(), SdkError> {
        let output_generation = self
            .outputs
            .state(&output)
            .map(|state| state.output_generation.saturating_add(1))
            .unwrap_or(1);
        self.outputs.publish(OutputState {
            output,
            graph_revision: self.revision(),
            output_generation,
            evaluation_stamp,
            content_stamp,
            payload,
        })
    }

    fn validate_presentation_references(
        &self,
        subscription: &PresentationSubscription,
    ) -> Result<(), SdkError> {
        if !self.output_contracts.contains_key(&subscription.output) {
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
            .output_contracts
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        let handles = self
            .resources
            .values()
            .map(|resource| resource.handle)
            .collect::<BTreeSet<_>>();
        self.presentation.reconcile(&outputs, &handles);
    }

    fn dispatch_presentations(&mut self, frame_stamp: FrameStamp) -> Result<(), SdkError> {
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

fn decode_engine_error(error: String) -> SdkError {
    serde_json::from_str(&error).unwrap_or_else(|_| {
        SdkError::new(SdkErrorCode::InvalidState, "Engine operation failed").with_details(error)
    })
}
