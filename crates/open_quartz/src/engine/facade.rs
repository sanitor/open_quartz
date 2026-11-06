use crate::domain::SdkError;

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
