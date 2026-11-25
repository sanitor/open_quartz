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
#[cfg(all(feature = "native-runtime", any(all(target_os = "macos", feature = "native-video"), all(windows, feature = "native-video-precise-d3d12"))))]
pub use pipeline::{ExportConfig, ExportJob, ExportPhase, ExportSequence, ExportStageEvent, ExportStatus};
