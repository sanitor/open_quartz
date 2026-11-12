mod catalog;
mod frame_budget;
#[cfg(all(not(target_arch = "wasm32"), feature = "native-video"))]
pub(crate) mod source_identity;

pub use frame_budget::{FrameAdmissionGuard, FrameBudget, FrameCredit, FrameWakeRegistration};

pub use catalog::{Resource, ResourceCatalog, ResourceKind, ResourceSource};

#[cfg(all(feature = "model-cache", not(target_arch = "wasm32")))]
pub mod models;
pub mod plan;
pub use plan::effective_inference_policy;

#[cfg(all(test, feature = "model-cache", not(target_arch = "wasm32")))]
mod models_tests;
