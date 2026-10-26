#[cfg(all(not(target_arch = "wasm32"), target_os = "windows"))]
use ort::execution_providers::{DirectMLExecutionProvider, ExecutionProvider};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NativeOnnxProvider {
    #[default]
    Cpu,
    DirectMl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeOnnxOptions {
    pub provider: NativeOnnxProvider,
    pub allow_cpu_fallback: bool,
}

impl Default for NativeOnnxOptions {
    fn default() -> Self {
        Self {
            provider: NativeOnnxProvider::Cpu,
            allow_cpu_fallback: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeOnnxCapabilities {
    pub cpu: bool,
    pub direct_ml: bool,
    pub shared_wgpu_device: bool,
}

use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OnnxSessionInfo {
    pub input_names: Vec<String>,
    pub output_names: Vec<String>,
    pub backend: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TensorOutput {
    pub data: Vec<f32>,
    pub shape: Vec<i64>,
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) trait TensorRunner {
    fn run_f32(&mut self, input: Vec<f32>, shape: Vec<i64>) -> Result<TensorOutput, String>;
}

#[cfg(not(target_arch = "wasm32"))]
pub struct OnnxSession {
    session: ort::session::Session,
    info: OnnxSessionInfo,
    #[cfg(test)]
    fault: u8,
}

#[cfg(not(target_arch = "wasm32"))]
pub fn native_onnx_capabilities() -> Result<NativeOnnxCapabilities, String> {
    #[cfg(target_os = "windows")]
    let direct_ml = DirectMLExecutionProvider::default()
        .is_available()
        .map_err(|error| format!("Cannot query DirectML availability: {error}"))?;
    #[cfg(not(target_os = "windows"))]
    let direct_ml = false;
    Ok(NativeOnnxCapabilities {
        cpu: true,
        direct_ml,
        shared_wgpu_device: false,
    })
}

#[cfg(not(target_arch = "wasm32"))]
impl OnnxSession {
    pub fn from_memory(model: &[u8]) -> Result<Self, String> {
        Self::from_memory_with_options(model, NativeOnnxOptions::default())
    }

    pub fn from_memory_with_options(
        model: &[u8],
        options: NativeOnnxOptions,
    ) -> Result<Self, String> {
        #[cfg(not(target_os = "windows"))]
        if options.provider == NativeOnnxProvider::DirectMl {
            return Err("DirectML is only available on Windows".to_owned());
        }
        #[allow(unused_mut)]
        #[cfg(test)]
        let builder_result = if model == b"__builder_error__" {
            Err(ort::Error::new("injected builder error"))
        } else {
            ort::session::Session::builder()
        };
        #[cfg(not(test))]
        let builder_result = ort::session::Session::builder();
        #[allow(unused_mut)]
        let mut builder = builder_result
            .map_err(|error| format!("Cannot create ONNX session builder: {error}"))?;
        #[cfg(not(target_os = "windows"))]
        let backend = "cpu";
        #[cfg(target_os = "windows")]
        let backend = match options.provider {
            NativeOnnxProvider::Cpu => "cpu",
            #[cfg(target_os = "windows")]
            NativeOnnxProvider::DirectMl => {
                let provider = DirectMLExecutionProvider::default().build();
                builder = builder
                    .with_execution_providers([if options.allow_cpu_fallback {
                        provider.fail_silently()
                    } else {
                        provider.error_on_failure()
                    }])
                    .and_then(|builder| builder.with_parallel_execution(false))
                    .and_then(|builder| builder.with_memory_pattern(false))
                    .map_err(|error| format!("Cannot configure DirectML session: {error}"))?;
                if !options.allow_cpu_fallback {
                    builder = builder
                        .with_config_entry("session.disable_cpu_ep_fallback", "1")
                        .map_err(|error| format!("Cannot disable ONNX CPU fallback: {error}"))?;
                }
                if options.allow_cpu_fallback {
                    "directml+cpu"
                } else {
                    "directml"
                }
            }
        };
        let session = builder
            .commit_from_memory(model)
            .map_err(|error| format!("Cannot load ONNX model: {error}"))?;
        let info = OnnxSessionInfo {
            input_names: session
                .inputs
                .iter()
                .map(|input| input.name.clone())
                .collect(),
            output_names: session
                .outputs
                .iter()
                .map(|output| output.name.clone())
                .collect(),
            backend: backend.to_owned(),
        };
        Ok(Self {
            session,
            info,
            #[cfg(test)]
            fault: 0,
        })
    }

    pub fn info(&self) -> &OnnxSessionInfo {
        &self.info
    }

    pub fn run_f32(&mut self, input: Vec<f32>, shape: Vec<i64>) -> Result<TensorOutput, String> {
        let input_name = self
            .info
            .input_names
            .first()
            .ok_or_else(|| "ONNX model has no inputs".to_owned())?
            .clone();
        let tensor = ort::value::Tensor::from_array((shape.clone(), input))
            .map_err(|error| format!("Cannot create ONNX tensor: {error}"))?;
        #[cfg(test)]
        let inputs_result = if self.fault == 1 {
            Err(ort::Error::new("injected input error"))
        } else {
            ort::inputs![input_name.as_str() => tensor]
        };
        #[cfg(not(test))]
        let inputs_result = ort::inputs![input_name.as_str() => tensor];
        let inputs =
            inputs_result.map_err(|error| format!("Cannot prepare ONNX inputs: {error}"))?;
        let outputs = self
            .session
            .run(inputs)
            .map_err(|error| format!("ONNX inference failed: {error}"))?;
        #[cfg(test)]
        let output_result = if self.fault == 2 {
            Err(ort::Error::new("injected output type error"))
        } else {
            outputs[0].try_extract_tensor::<f32>()
        };
        #[cfg(not(test))]
        let output_result = outputs[0].try_extract_tensor::<f32>();
        let output =
            output_result.map_err(|error| format!("ONNX output is not float32: {error}"))?;
        Ok(TensorOutput {
            data: output.iter().copied().collect(),
            shape: output
                .shape()
                .iter()
                .map(|&dimension| dimension as i64)
                .collect(),
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl TensorRunner for OnnxSession {
    fn run_f32(&mut self, input: Vec<f32>, shape: Vec<i64>) -> Result<TensorOutput, String> {
        OnnxSession::run_f32(self, input, shape)
    }
}

#[cfg(test)]
#[path = "session_coverage_tests.rs"]
mod coverage_tests;
