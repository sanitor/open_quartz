use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ffi::{Engine, EngineEvent, EngineState, SdkError, SdkErrorCode};
use crate::types::Graph;

use super::{PresentationSet, RuntimeCapabilities};

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
    presentations: BTreeMap<String, PresentationSet>,
    capabilities: RuntimeCapabilities,
}

impl Runtime {
    pub fn new(capabilities: RuntimeCapabilities) -> Self {
        Self {
            engine: Engine::new(),
            resources: BTreeMap::new(),
            presentations: BTreeMap::new(),
            capabilities,
        }
    }

    pub fn capabilities(&self) -> &RuntimeCapabilities {
        &self.capabilities
    }

    pub fn set_graph(&mut self, graph: &Graph) -> Result<u32, SdkError> {
        let json = serde_json::to_string(graph).map_err(|error| {
            SdkError::new(SdkErrorCode::InvalidGraph, "Cannot serialize graph")
                .with_details(error.to_string())
        })?;
        self.engine
            .set_graph_json(&json)
            .map_err(decode_engine_error)
    }

    pub fn mark_dirty(&mut self, node_id: &str) -> Result<(), SdkError> {
        self.engine.mark_dirty(node_id).map_err(decode_engine_error)
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
            .map_err(decode_engine_error)
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
        self.resources.insert(
            descriptor.resource_id.clone(),
            RegisteredResource { descriptor, handle },
        );
        Ok(())
    }

    pub fn remove_resource(&mut self, resource_id: &str) -> Result<u64, SdkError> {
        self.resources
            .remove(resource_id)
            .map(|resource| resource.handle)
            .ok_or_else(|| {
                SdkError::new(SdkErrorCode::InvalidResource, "Resource is not registered")
                    .with_details(resource_id)
            })
    }

    pub fn update_presentation(&mut self, set: PresentationSet) -> Result<(), SdkError> {
        if set.group_id.is_empty() {
            return Err(SdkError::new(
                SdkErrorCode::InvalidResource,
                "Presentation group ID cannot be empty",
            ));
        }
        self.presentations.insert(set.group_id.clone(), set);
        Ok(())
    }

    pub fn presentation(&self, group_id: &str) -> Option<&PresentationSet> {
        self.presentations.get(group_id)
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
        self.presentations.clear();
    }
}

fn decode_engine_error(error: String) -> SdkError {
    serde_json::from_str(&error).unwrap_or_else(|_| {
        SdkError::new(SdkErrorCode::InvalidState, "Engine operation failed").with_details(error)
    })
}
