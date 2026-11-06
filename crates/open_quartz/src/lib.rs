//! OpenQuartz project model, execution runtime, and language bindings.
//! Device capabilities are opt-in; project editing and shader planning are CPU-only.

mod domain;
mod sdk;
mod logging;

pub mod catalog;
pub mod engine;
pub mod error;
pub mod event;
pub mod ffi;
pub mod graph;
pub mod onnx;
pub mod resources;
pub mod runtime;
pub mod wgsl;
#[cfg(feature = "gpu")]
pub mod gpu;
#[cfg(all(not(target_arch = "wasm32"), feature = "native-video"))]
pub mod native_video;
#[cfg(all(target_arch = "wasm32", feature = "browser"))]
pub mod wasm_environment;

pub use domain::{
    DataType, Edge, FramebufferFormat, Graph, GraphChange, GraphCommand, GraphEdit, InputMode,
    NodeData, NodeFactoryRequest, NodeId, NodeType, OnnxParamValue, OnnxSource, OnnxTask, Port,
    PortDirection, PortId, PortKey, Position, ProjectFile, ProjectId, ProjectNode, ResourceId,
    SdkError, SdkErrorCode, SubscriptionId, SystemSource, TextureFilter, TextureWrap,
    VideoSourceType, PROJECT_FILE_VERSION,
};
pub use sdk::{
    Environment, GraphLayout, OpenQuartz, Output, OutputPolicy, Player, PlayerBuilder, PlayerState,
    Project, Resource, ResourceCatalog, ResourceKind, ResourceSource, Subscription,
};

#[cfg(test)]
extern crate self as open_quartz;

#[cfg(test)]
#[path = "../tests/catalog_contract_test.rs"]
mod coverage_catalog_contract;
#[cfg(test)]
#[path = "../tests/compiler_test.rs"]
mod coverage_compiler;
#[cfg(all(test, windows, feature = "native-video-d3d12"))]
#[path = "../tests/dxgi_shared_texture_test.rs"]
mod coverage_dxgi_shared_texture;
#[cfg(test)]
#[path = "../tests/executor_test.rs"]
mod coverage_executor;
#[cfg(test)]
#[path = "../tests/executor_ffi_test.rs"]
mod coverage_executor_ffi;
#[cfg(all(test, feature = "gpu"))]
#[path = "../tests/gpu_test.rs"]
mod coverage_gpu;
#[cfg(all(test, feature = "gpu", not(target_arch = "wasm32")))]
#[path = "../tests/gpu_executor_test.rs"]
mod coverage_gpu_executor;
#[cfg(all(test, feature = "gpu"))]
#[path = "../tests/gpu_shader_test.rs"]
mod coverage_gpu_shader;
#[cfg(test)]
#[path = "../tests/graph_test.rs"]
mod coverage_graph;
#[cfg(test)]
#[path = "../tests/onnx_test.rs"]
mod coverage_onnx;
#[cfg(test)]
#[path = "../tests/host_resource_intent_test.rs"]
mod coverage_host_resource_intent;
#[cfg(test)]
#[path = "../tests/output_registry_test.rs"]
mod coverage_output_registry;
#[cfg(test)]
#[path = "../tests/parser_test.rs"]
mod coverage_parser;
#[cfg(test)]
#[path = "../tests/presentation_test.rs"]
mod coverage_presentation;
#[cfg(all(test, feature = "gpu", not(target_arch = "wasm32")))]
#[path = "../tests/presenter_test.rs"]
mod coverage_presenter;
#[cfg(test)]
#[path = "../tests/runtime_contract_test.rs"]
mod coverage_runtime_contract;
