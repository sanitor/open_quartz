use crate::error::SdkError;

use super::{ExecutionCommand, ExecutionPlan};

/// GPU execution port consumed by the shared Engine.
/// Implementations own platform GPU objects; the Engine only supplies a plan and typed work.
pub trait GpuFacade {
    fn execute(
        &mut self,
        plan: &ExecutionPlan,
        commands: &[ExecutionCommand],
    ) -> Result<(), SdkError>;
}

/// Media lifecycle port. Platform decoders remain behind this contract.
pub trait MediaFacade: Send {
    fn pause(&mut self) -> Result<(), SdkError>;
    fn resume(&mut self) -> Result<(), SdkError>;
}

/// Inference lifecycle port. Requests and completions use Runtime-owned stamps.
pub trait InferenceFacade: Send {
    fn cancel_node(&mut self, node_id: &str, generation: u32) -> Result<(), SdkError>;
}

/// Presentation lifecycle port. Concrete window and stream objects remain platform-owned.
pub trait PresentationFacade: Send {
    fn remove_output(&mut self, node_id: &str) -> Result<(), SdkError>;
}
