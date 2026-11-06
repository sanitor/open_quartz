//! WGSL analysis and graph-to-work-plan compilation. No live device or playback state.

pub mod shader;
pub mod topology;
mod plan;

pub use plan::{build_execution_plan, build_execution_plan_with_options, ExecutionPlan, NodeExecutionPlan, TargetSpec};
