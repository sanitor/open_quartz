#[cfg(not(target_arch = "wasm32"))]
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
pub struct OnnxSession {
    session: ort::session::Session,
    info: OnnxSessionInfo,
}

#[cfg(not(target_arch = "wasm32"))]
pub fn native_onnx_capabilities() -> Result<NativeOnnxCapabilities, String> {
    let direct_ml = if cfg!(target_os = "windows") {
        DirectMLExecutionProvider::default()
            .is_available()
            .map_err(|error| format!("Cannot query DirectML availability: {error}"))?
    } else {
        false
    };
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
        let mut builder = ort::session::Session::builder()
            .map_err(|error| format!("Cannot create ONNX session builder: {error}"))?;
        let backend = match options.provider {
            NativeOnnxProvider::Cpu => "cpu",
            NativeOnnxProvider::DirectMl => {
                if !cfg!(target_os = "windows") {
                    return Err("DirectML is only available on Windows".to_owned());
                }
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
        Ok(Self { session, info })
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
        let inputs = ort::inputs![input_name.as_str() => tensor]
            .map_err(|error| format!("Cannot prepare ONNX inputs: {error}"))?;
        let outputs = self
            .session
            .run(inputs)
            .map_err(|error| format!("ONNX inference failed: {error}"))?;
        let output = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|error| format!("ONNX output is not float32: {error}"))?;
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
