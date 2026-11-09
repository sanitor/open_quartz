mod catalog;

pub use catalog::{Resource, ResourceCatalog, ResourceKind, ResourceSource};

#[cfg(all(feature = "model-cache", not(target_arch = "wasm32")))]
pub mod models;
pub mod plan;
pub use plan::effective_inference_policy;

#[cfg(all(test, feature = "model-cache", not(target_arch = "wasm32")))]
mod models_tests;
