use crate::engine::{ExecutionCommand, ExecutionPlan, GpuFacade};
use open_quartz_schema::SdkError;
use crate::event::{EngineEvent, EngineState};
use crate::runtime::{
    AsyncCompletionEnvelope, ClockState, HostBackend, OutputDeliveryBatch, OutputState,
    OutputSubscription, ResourceDescriptor, Runtime, RuntimeCapabilities, RuntimeFrameInput,
};
use open_quartz_schema::Graph;

/// Advanced host-integration driver behind the public [`crate::Player`] facade.
///
/// Tauri, screen-saver, WASM, and future JNI hosts use this type to supply
/// platform execution capabilities. Application code should use `Player`.
pub struct PlayerHost {
    runtime: Runtime,
}

impl PlayerHost {
    pub fn new(capabilities: RuntimeCapabilities) -> Self {
        Self {
            runtime: Runtime::new(capabilities),
        }
    }

    pub fn new_native(capabilities: RuntimeCapabilities) -> Self {
        Self {
            runtime: Runtime::new_native(capabilities),
        }
    }
    pub fn attach_backend(&mut self, backend: Box<dyn HostBackend>) {
        self.runtime.attach_backend(backend);
    }

    pub fn capabilities(&self) -> &RuntimeCapabilities {
        self.runtime.capabilities()
    }

    pub fn register_resource(
        &mut self,
        descriptor: ResourceDescriptor,
        handle: u64,
    ) -> Result<(), SdkError> {
        self.runtime.register_resource(descriptor, handle)
    }

    pub fn remove_resource(&mut self, resource_id: &str) -> Result<u64, SdkError> {
        self.runtime.remove_resource(resource_id)
    }

    pub fn set_graph(&mut self, graph: &Graph) -> Result<u32, SdkError> {
        self.runtime.set_graph(graph)
    }

    pub fn play(&mut self, now_ns: u64) -> Result<(), SdkError> {
        self.runtime.play(now_ns)
    }

    pub fn pause(&mut self, now_ns: u64) -> Result<(), SdkError> {
        self.runtime.pause(now_ns)
    }

    pub fn resume(&mut self, now_ns: u64) -> Result<(), SdkError> {
        self.runtime.resume(now_ns)
    }

    pub fn stop(&mut self) -> Result<(), SdkError> {
        self.runtime.stop()
    }

    pub fn advance(&mut self, input: &RuntimeFrameInput) -> Result<ClockState, SdkError> {
        self.runtime.advance(input)
    }

    pub fn state(&self) -> EngineState {
        self.runtime.state()
    }

    pub fn revision(&self) -> u32 {
        self.runtime.revision()
    }

    pub fn node_generation(&self, node_id: &str) -> Result<u32, SdkError> {
        self.runtime.node_generation(node_id)
    }

    pub fn mark_dirty(&mut self, node_id: &str) -> Result<(), SdkError> {
        self.runtime.mark_dirty(node_id)
    }

    pub fn set_video_nodes(&mut self, node_ids: &[String]) -> Result<(), SdkError> {
        self.runtime.set_video_nodes(node_ids)
    }

    pub fn execution_plan(&self) -> Option<&ExecutionPlan> {
        self.runtime.execution_plan()
    }

    pub fn drain_commands(&mut self) -> Vec<ExecutionCommand> {
        self.runtime.drain_commands()
    }
    pub fn drain_work(&mut self) -> Result<String, SdkError> {
        self.runtime.drain_work()
    }

    pub fn publish_output(&mut self, state: OutputState) -> Result<(), SdkError> {
        self.runtime.publish_output(state)
    }

    pub fn set_every_queue_capacity(&mut self, capacity: usize) {
        self.runtime.set_every_queue_capacity(capacity);
    }

    pub fn execute_gpu(
        &self,
        facade: &mut dyn GpuFacade,
        commands: &[ExecutionCommand],
    ) -> Result<(), SdkError> {
        self.runtime.execute_gpu(facade, commands)
    }

    pub fn subscribe_output(&mut self, subscription: OutputSubscription) -> Result<(), SdkError> {
        self.runtime.subscribe_output(subscription)
    }

    pub fn unsubscribe_output(
        &mut self,
        subscription_id: &str,
    ) -> Result<OutputSubscription, SdkError> {
        self.runtime.unsubscribe_output(subscription_id)
    }

    pub fn submit_completion(
        &mut self,
        completion: AsyncCompletionEnvelope,
    ) -> Result<(), SdkError> {
        self.runtime.submit_completion(completion)
    }

    pub fn drain_deliveries(&mut self) -> OutputDeliveryBatch {
        self.runtime.drain_deliveries()
    }

    pub fn drain_events(&mut self) -> Vec<EngineEvent> {
        self.runtime.drain_events()
    }

    pub fn dispose(&mut self) -> Result<(), SdkError> {
        self.runtime.dispose()
    }
}
