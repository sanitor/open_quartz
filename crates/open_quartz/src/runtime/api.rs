use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::ffi::{Engine, EngineEvent, EngineState, SdkError, SdkErrorCode};
use crate::types::Graph;

use super::{
    ClockState, CompositionClock, ContentStamp, FrameStamp, OutputDeliveryBatch, OutputKey,
    OutputPayload, OutputRegistry, OutputState, OutputSubscription, PresentationPlanner,
    PresentationSet, PresentationSubscription, RuntimeCapabilities,
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
    pub time: f64,
    pub delta: f64,
    pub frame: u64,
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
    output_ports: BTreeMap<String, Vec<OutputKey>>,
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
            output_ports: BTreeMap::new(),
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
    pub fn start_at(&mut self, now_ns: u64) {
        self.clock.start(now_ns);
    }

    pub fn pause_at(&mut self, now_ns: u64) -> Result<(), SdkError> {
        self.clock.pause(now_ns)
    }

    pub fn resume_at(&mut self, now_ns: u64) -> Result<(), SdkError> {
        self.clock.resume(now_ns)
    }

    pub fn stop_clock(&mut self) {
        self.clock.stop();
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
        self.output_ports = graph
            .nodes
            .iter()
            .map(|node| {
                let outputs = node
                    .data
                    .outputs
                    .iter()
                    .map(|port| OutputKey::new(&node.id, &port.id))
                    .collect::<Vec<_>>();
                (node.id.clone(), outputs)
            })
            .collect();
        let valid_outputs = self
            .output_ports
            .values()
            .flatten()
            .cloned()
            .collect::<BTreeSet<_>>();
        self.outputs.reconcile(revision, valid_outputs);
        Ok(revision)
    }

    pub fn advance(&mut self, input: &RuntimeFrameInput) -> Result<(), SdkError> {
        self.engine
            .run_frame(
                input.time,
                input.delta,
                input.frame,
                &input.date,
                &input.mouse,
                &input.resolution,
            )
            .map_err(decode_engine_error)?;
        let commands = self.engine.pending_commands().to_vec();
        for command in commands {
            let Some(value) = command.scalar_output else {
                continue;
            };
            let Some(output) = self
                .output_ports
                .get(&command.node_id)
                .and_then(|ports| ports.first())
                .cloned()
            else {
                continue;
            };
            let generation = self
                .outputs
                .state(&output)
                .map(|state| state.output_generation.saturating_add(1))
                .unwrap_or(1);
            let timeline_ns = seconds_to_ns(input.time);
            let clock_state = self.clock.state();
            let stamp = FrameStamp {
                epoch: clock_state.epoch,
                frame: input.frame,
                timeline_ns,
                deadline_ns: clock_state.next_deadline_ns,
            };
            self.outputs.publish(OutputState {
                output,
                graph_revision: self.revision(),
                output_generation: generation,
                evaluation_stamp: stamp.clone(),
                content_stamp: ContentStamp {
                    epoch: stamp.epoch,
                    timeline_ns: stamp.timeline_ns,
                    media_pts_ns: None,
                },
                payload: OutputPayload::Float(value),
            })?;
        }
        Ok(())
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
        let resource = self.resources.remove(resource_id).ok_or_else(|| {
            SdkError::new(SdkErrorCode::InvalidResource, "Resource is not registered")
                .with_details(resource_id)
        })?;
        if let Some(backend) = self.backend.as_mut() {
            backend.remove_resource(resource_id, resource.handle)?;
        }
        Ok(resource.handle)
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
        self.presentation
            .subscribe(subscription)
            .map_err(|message| SdkError::new(SdkErrorCode::InvalidResource, message))
    }

    pub fn update_presentation_subscription(
        &mut self,
        subscription: PresentationSubscription,
    ) -> Result<(), SdkError> {
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

    pub fn pause(&mut self) -> Result<(), SdkError> {
        self.engine.pause().map_err(decode_engine_error)
    }

    pub fn resume(&mut self) -> Result<(), SdkError> {
        self.engine.resume().map_err(decode_engine_error)
    }

    pub fn stop(&mut self) -> Result<(), SdkError> {
        self.engine.stop().map_err(decode_engine_error)
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

    pub fn dispose(&mut self) {
        self.engine.dispose();
        self.resources.clear();
        self.output_ports.clear();
        self.outputs = OutputRegistry::default();
        self.presentation = PresentationPlanner::default();
    }
}

fn decode_engine_error(error: String) -> SdkError {
    serde_json::from_str(&error).unwrap_or_else(|_| {
        SdkError::new(SdkErrorCode::InvalidState, "Engine operation failed").with_details(error)
    })
}

fn seconds_to_ns(seconds: f64) -> u64 {
    if !seconds.is_finite() || seconds <= 0.0 {
        return 0;
    }
    (seconds * 1_000_000_000.0).round().min(u64::MAX as f64) as u64
}
