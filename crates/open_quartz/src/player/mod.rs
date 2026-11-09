mod entry;
mod environment;
mod output;
mod playback;
mod subscription;
mod clock;
mod session;
pub(crate) mod logging;
#[cfg(all(feature = "native-runtime", not(target_arch = "wasm32")))]
mod native;

pub use entry::OpenQuartz;
pub use environment::Environment;
pub use output::{Output, OutputPolicy};
pub use playback::{Player, PlayerBuilder, PlayerState};
pub use subscription::Subscription;
pub use clock::{ClockState, CompositionClock};
pub use session::{PlaybackSession, SessionFrameInput};
pub use crate::outputs::PlaybackCapabilities;
#[cfg(all(feature = "native-runtime", not(target_arch = "wasm32")))]
pub use native::{InferenceConfig, InferenceMetrics, InferenceOutputEvent, NativePlayer, NativePlayerInfo, NativeFrameRendered, PreparedNativeGraph};

#[cfg(test)]
#[path = "tests/support.rs"]
mod test_support;
