pub mod catalog;
pub mod engine;
pub mod event;
pub mod gpu;
pub mod graph;
pub mod host;
mod logging;
pub mod media;
#[cfg(not(target_arch = "wasm32"))]
pub mod native_video;
pub mod onnx;
pub mod runtime;
#[cfg(target_arch = "wasm32")]
pub mod wasm_environment;
pub mod wgsl;

pub mod error {
    pub use open_quartz_schema::{SdkError, SdkErrorCode};
}

pub mod types {
    pub use open_quartz_schema::*;
}

#[cfg(test)]
extern crate self as open_quartz_execution;

#[cfg(test)]
#[path = "../tests/catalog_contract_test.rs"]
mod coverage_catalog_contract;
#[cfg(test)]
#[path = "../tests/compiler_test.rs"]
mod coverage_compiler;
#[cfg(all(test, windows))]
#[path = "../tests/dxgi_shared_texture_test.rs"]
mod coverage_dxgi_shared_texture;
#[cfg(test)]
#[path = "../tests/executor_test.rs"]
mod coverage_executor;
#[cfg(test)]
#[path = "../tests/executor_ffi_test.rs"]
mod coverage_executor_ffi;
#[cfg(test)]
#[path = "../tests/gpu_test.rs"]
mod coverage_gpu;
#[cfg(test)]
#[path = "../tests/gpu_executor_test.rs"]
mod coverage_gpu_executor;
#[cfg(test)]
#[path = "../tests/gpu_shader_test.rs"]
mod coverage_gpu_shader;
#[cfg(test)]
#[path = "../tests/graph_test.rs"]
mod coverage_graph;
#[cfg(test)]
#[path = "../tests/media_gpu_test.rs"]
mod coverage_media_gpu;
#[cfg(test)]
#[path = "../tests/onnx_test.rs"]
mod coverage_onnx;
#[cfg(test)]
#[path = "../tests/output_registry_test.rs"]
mod coverage_output_registry;
#[cfg(test)]
#[path = "../tests/parser_test.rs"]
mod coverage_parser;
#[cfg(test)]
#[path = "../tests/presentation_test.rs"]
mod coverage_presentation;
#[cfg(test)]
#[path = "../tests/presenter_test.rs"]
mod coverage_presenter;
#[cfg(test)]
#[path = "../tests/runtime_contract_test.rs"]
mod coverage_runtime_contract;
