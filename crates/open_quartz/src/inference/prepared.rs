use std::fs;
use std::path::Path;

use super::{NativeOnnxOptions, NativeOnnxProvider, OnnxSession, OnnxSessionInfo};
use super::types::NativeOnnxConfig;

/// Own the session off the GPU/UI threads until its request identity is accepted.
pub struct PreparedOnnxModel {
    session: OnnxSession,
    config: NativeOnnxConfig,
}

impl PreparedOnnxModel {
    pub fn open(
        path: &Path,
        config: NativeOnnxConfig,
        prefer_direct_ml: bool,
    ) -> Result<Self, String> {
        let model = fs::read(path)
            .map_err(|e| format!("Cannot read ONNX model {}: {e}", path.display()))?;
        let provider = if prefer_direct_ml && cfg!(windows) {
            NativeOnnxProvider::DirectMl
        } else {
            NativeOnnxProvider::Cpu
        };
        let session = OnnxSession::from_memory_with_options(
            &model,
            NativeOnnxOptions {
                provider,
                allow_cpu_fallback: true,
            },
        )?;
        Ok(Self { session, config })
    }

    pub fn info(&self) -> &OnnxSessionInfo {
        self.session.info()
    }

    pub fn into_parts(self) -> (OnnxSession, NativeOnnxConfig) {
        (self.session, self.config)
    }

    #[cfg(feature = "native-runtime")]
    pub fn attach(self, runtime: &mut crate::execution::runtime::ExecutionRuntime, node_id: String) -> Result<OnnxSessionInfo, String> {
        runtime.load_onnx(node_id, self.session, self.config)
    }
}

#[cfg(test)]
#[path = "prepared_coverage_tests.rs"]
mod tests;
