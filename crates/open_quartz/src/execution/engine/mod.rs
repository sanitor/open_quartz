pub mod executor;
mod facade;
pub mod frame;
pub mod plan;
mod state;

pub use executor::ExecutionEngine;
pub use facade::GpuFacade;
pub use frame::{ExecutionCommand, FrameInputs, FrameResult};
pub use plan::{build_execution_plan, ExecutionPlan, NodeExecutionPlan, TargetSpec};
pub use state::Engine;
