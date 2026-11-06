mod catalog;

pub use catalog::{Resource, ResourceCatalog, ResourceKind, ResourceSource};

#[cfg(all(feature = "model-cache", not(target_arch = "wasm32")))]
pub mod models;
pub mod plan;

#[cfg(all(test, feature = "model-cache", not(target_arch = "wasm32")))]
mod models_tests;
