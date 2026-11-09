use super::InferenceTask;
use oq_inference::TensorOutput;
/// Decoder semantics are selected by the task adapter, never by tensor length.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ImageTaskAdapter {
    RgbImage,
    LuminanceSuperResolution3x,
    AlphaMask,
    DepthMap,
    CocoYolo8400,
    Cityscapes19,
}

/// These describe the existing application transforms, not ONNX metadata claims.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ImageTaskPreprocessing {
    RgbZeroToOne,
    Bt601LuminanceZeroToOne,
    BgrImageNetMeanStd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ImageTaskResize {
    Source,
    Square(u32),
    Letterbox(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ImageDimensionConstraint {
    Fixed(u64),
    Positive,
    InputHeightTimes(u32),
    InputWidthTimes(u32),
}

/// The exact named FP32 input and selected output plus task adapter constraints.
/// Full model outputs remain available in `InferenceSessionInfo`; only explicit
/// catalog bindings may select one output from a multi-output model.
/// Symbolic/dynamic dimensions remain in `input`/`output`; constraints are checked
/// again against concrete invocation shapes before decoding any output.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageTaskContract {
    pub input: oq_inference::TensorMetadata,
    pub output: oq_inference::TensorMetadata,
    pub input_layout: &'static str,
    pub adapter: ImageTaskAdapter,
    pub preprocessing: ImageTaskPreprocessing,
    pub resize: ImageTaskResize,
    pub input_shape: Vec<ImageDimensionConstraint>,
    pub output_shape: Vec<ImageDimensionConstraint>,
}

pub fn image_task_contract(
    config: &super::InferenceConfig,
    info: &super::InferenceSessionInfo,
) -> Result<ImageTaskContract, String> {
    use ImageDimensionConstraint::{Fixed, InputHeightTimes, InputWidthTimes, Positive};
    use ImageTaskAdapter::*;
    use ImageTaskPreprocessing::*;
    use ImageTaskResize::*;
    let [input] = info.inputs.as_slice() else {
        return Err("image task requires exactly one tensor input".to_owned());
    };
    let output = if config.task == InferenceTask::BackgroundRemoval && config.model_id == "u2netp" {
        // The catalog rembg U²Net-P export names d0 (the primary mask) "1959".
        // Its six side-output masks remain visible in the complete model interface.
        info.outputs.iter().find(|output| output.name == "1959")
            .ok_or_else(|| "catalog u2netp requires its explicitly bound primary output \"1959\"".to_owned())?
    } else {
        let [output] = info.outputs.as_slice() else {
            return Err("image task requires exactly one tensor output or an explicit catalog output binding".to_owned());
        };
        output
    };
    for (tensors, names) in [(&info.inputs, &info.input_names), (&info.outputs, &info.output_names)] {
        if tensors.len() != names.len() || tensors.iter().zip(names).enumerate().any(|(index, (tensor, name))| {
            tensor.name.is_empty() || tensor.name.contains('\0') || tensor.name != *name
                || names[..index].contains(name)
        }) {
            return Err("image task tensor names must match the exact declared interface".to_owned());
        }
    }
    for tensor in [input, output] {
        if tensor.element_type != oq_inference::TensorElementType::Float32 {
            return Err(format!("image task tensor {:?} requires FP32, received {:?}", tensor.name, tensor.element_type));
        }
    }
    let target = config.target_size.max(1);
    let (adapter, preprocessing, resize, channels, output_shape) = match config.task {
        InferenceTask::SuperResolution if config.model_id == "super-resolution-3x" => (
            LuminanceSuperResolution3x, Bt601LuminanceZeroToOne, Square(224), 1,
            vec![Fixed(1), Fixed(1), Fixed(672), Fixed(672)],
        ),
        InferenceTask::BackgroundRemoval => (
            AlphaMask, RgbZeroToOne, Square(if config.model_id == "u2netp" { 320 } else { 512 }), 3,
            vec![Fixed(1), Fixed(1), Positive, Positive],
        ),
        InferenceTask::DepthEstimation => (
            DepthMap, BgrImageNetMeanStd, Square(256), 3,
            if output.dimensions.len() == 3 { vec![Fixed(1), Positive, Positive] }
            else { vec![Fixed(1), Fixed(1), Positive, Positive] },
        ),
        InferenceTask::Detection => (
            CocoYolo8400, RgbZeroToOne, Letterbox(target), 3,
            vec![Fixed(1), Fixed(84), Fixed(8400)],
        ),
        InferenceTask::Segmentation => (
            Cityscapes19, RgbZeroToOne, Letterbox(target), 3,
            vec![Fixed(1), Fixed(19), Positive, Positive],
        ),
        _ => (
            RgbImage, RgbZeroToOne, Source, 3,
            if config.task == InferenceTask::SuperResolution && config.model_id == "realesrgan-x4" {
                vec![Fixed(1), Fixed(3), InputHeightTimes(4), InputWidthTimes(4)]
            } else { vec![Fixed(1), Fixed(3), Positive, Positive] },
        ),
    };
    let spatial = match resize { Source => Positive, Square(size) | Letterbox(size) => Fixed(size as u64) };
    let input_shape = vec![Fixed(1), Fixed(channels), spatial, spatial];
    validate_declared_shape(input, &input_shape, &input.dimensions)?;
    validate_declared_shape(output, &output_shape, &input.dimensions)?;
    if adapter == Cityscapes19 {
        if let (oq_inference::TensorDimension::Static(height), oq_inference::TensorDimension::Static(width)) =
            (&output.dimensions[2], &output.dimensions[3]) {
            if height != width {
                return Err("Cityscapes decoder requires a square [1,19,H,H] output".to_owned());
            }
        }
    }
    Ok(ImageTaskContract {
        input: input.clone(), output: output.clone(), input_layout: "NCHW",
        adapter, preprocessing, resize, input_shape, output_shape,
    })
}

fn constraint_value(constraint: ImageDimensionConstraint, input_height: Option<u64>, input_width: Option<u64>) -> Option<u64> {
    use ImageDimensionConstraint::*;
    match constraint {
        Fixed(value) => Some(value),
        Positive => None,
        InputHeightTimes(scale) => input_height.and_then(|value| value.checked_mul(scale as u64)),
        InputWidthTimes(scale) => input_width.and_then(|value| value.checked_mul(scale as u64)),
    }
}

pub(super) fn validate_declared_shape(
    tensor: &oq_inference::TensorMetadata,
    constraints: &[ImageDimensionConstraint],
    input: &[oq_inference::TensorDimension],
) -> Result<(), String> {
    if tensor.dimensions.len() != constraints.len() {
        return Err(format!("image task tensor {:?} requires rank {}, received rank {}", tensor.name, constraints.len(), tensor.dimensions.len()));
    }
    for (axis, (dimension, constraint)) in tensor.dimensions.iter().zip(constraints).enumerate() {
        if let oq_inference::TensorDimension::Static(actual) = dimension {
            let input_axis = |axis| match input.get(axis) {
                Some(oq_inference::TensorDimension::Static(value)) => Some(*value),
                _ => None,
            };
            let expected = constraint_value(*constraint, input_axis(2), input_axis(3));
            if *actual == 0 || *actual > u32::MAX as u64 || expected.is_some_and(|expected| expected != *actual) {
                return Err(format!("image task tensor {:?} axis {axis} requires {constraint:?}, received {actual}", tensor.name));
            }
        }
    }
    Ok(())
}

impl ImageTaskContract {
    pub(super) fn validate_input(&self, shape: &[i64], length: usize) -> Result<(), String> {
        self.validate_concrete(&self.input, &self.input_shape, shape, length, shape)
    }

    /// Validate the task decoder contract even when the model declares symbolic output axes.
    pub fn validate_output(&self, output: &TensorOutput, input_shape: &[i64]) -> Result<(), String> {
        self.validate_concrete(&self.output, &self.output_shape, &output.shape, output.data.len(), input_shape)?;
        if self.adapter == ImageTaskAdapter::Cityscapes19 && output.shape[2] != output.shape[3] {
            return Err("Cityscapes decoder requires a square [1,19,H,H] output".to_owned());
        }
        Ok(())
    }

    fn validate_concrete(&self, tensor: &oq_inference::TensorMetadata,
        constraints: &[ImageDimensionConstraint], shape: &[i64], length: usize, input_shape: &[i64],
    ) -> Result<(), String> {
        if shape.len() != constraints.len() {
            return Err(format!("image task tensor {:?} requires rank {}, received rank {}", tensor.name, constraints.len(), shape.len()));
        }
        let mut expected_length = 1usize;
        for (axis, ((&actual, declared), &constraint)) in shape.iter().zip(&tensor.dimensions).zip(constraints).enumerate() {
            let input_axis = |axis| input_shape.get(axis).and_then(|&value| u64::try_from(value).ok());
            let expected = constraint_value(constraint, input_axis(2), input_axis(3));
            let declared_matches = match declared {
                oq_inference::TensorDimension::Static(value) => *value == actual as u64,
                _ => true,
            };
            if actual <= 0 || actual > u32::MAX as i64 || !declared_matches || expected.is_some_and(|expected| expected != actual as u64) {
                return Err(format!("image task tensor {:?} axis {axis} requires {constraint:?} and {declared:?}, received {actual}", tensor.name));
            }
            expected_length = expected_length.checked_mul(actual as usize)
                .ok_or_else(|| "image task tensor size overflow".to_owned())?;
        }
        if length != expected_length {
            return Err(format!("image task tensor {:?} has {length} values, expected {expected_length}", tensor.name));
        }
        Ok(())
    }
}
