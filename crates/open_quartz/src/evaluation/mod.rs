//! Applied graph identity and deterministic per-frame evaluation.

mod dirty;
pub mod event;
mod evaluator;
mod instance;
mod render_target;
pub mod frame;

pub use event::{EngineEvent, EngineState};
pub use dirty::DirtySet;
pub use evaluator::GraphEvaluator;
pub use instance::GraphInstance;
pub use render_target::GpuFacade;
pub use frame::{ExecutionCommand, FrameInputs, FrameResult};
