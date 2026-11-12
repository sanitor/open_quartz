//! Shared clock-free graph transactions and the native frame pipeline.
pub(crate) mod session;
pub use session::{FrameEvaluationInput, FrameExecutionSession};
#[cfg(all(feature = "native-runtime", not(target_arch = "wasm32")))]
mod pipeline;
#[cfg(all(feature = "native-runtime", not(target_arch = "wasm32")))]
pub use pipeline::{
    ExternalFrameInput, FrameAdmission, FrameCompletion, FrameDemand, FrameOutcome, FramePipeline,
    FrameStageEvent, FrameWindowConfig, PreparedNativeGraph,
};
#[cfg(all(target_os = "macos", feature = "native-video", feature = "native-runtime"))]
pub use pipeline::{ExportConfig, ExportJob, ExportPhase, ExportStageEvent, ExportStatus};
