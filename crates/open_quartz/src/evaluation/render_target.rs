use crate::project::model::SdkError;

use crate::compiler::ExecutionPlan;
use super::ExecutionCommand;

/// GPU execution port consumed by the shared GraphInstance.
/// Implementations own platform GPU objects; the GraphInstance only supplies a plan and typed work.
pub trait GpuFacade {
    fn execute(
        &mut self,
        plan: &ExecutionPlan,
        commands: &[ExecutionCommand],
    ) -> Result<(), SdkError>;
}
