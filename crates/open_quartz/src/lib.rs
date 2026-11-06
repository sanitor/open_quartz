//! OpenQuartz project model, playback, and native rendering.
//! Device capabilities are opt-in; project editing and shader planning are CPU-only.

pub mod project;
pub mod player;
pub mod execution;
pub mod inference;
pub mod media;
pub mod resources;

pub use project::{
    DataType, Edge, FramebufferFormat, Graph, GraphChange, GraphCommand, GraphEdit, GraphLayout,
    InputMode, NodeData, NodeFactoryRequest, NodeId, NodeType, OnnxParamValue, OnnxSource,
    OnnxTask, Port, PortDirection, PortId, PortKey, Position, Project, ProjectFile, ProjectId,
    ProjectNode, ResourceId, SdkError, SdkErrorCode, SubscriptionId, SystemSource,
    TextureFilter, TextureWrap, VideoSourceType, PROJECT_FILE_VERSION,
};
pub use player::{
    Environment, OpenQuartz, Output, OutputPolicy, Player, PlayerBuilder, PlayerState, Subscription,
};
pub use resources::{Resource, ResourceCatalog, ResourceKind, ResourceSource};

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
#[path = "../tests/engine_contract_test.rs"]
mod coverage_engine_contract;
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
