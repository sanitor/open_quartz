//! Native media sources and their integration with the graph executor.

#[cfg(all(not(target_arch = "wasm32"), feature = "native-video"))]
pub mod native;
