//! Preparation-only compiler for the resident super-resolution-3x profile.
//!
//! The accepted ONNX-10 network is four FP32 NCHW convolutions (64/64/32/9
//! channels, 5/3/3/3 kernels), ReLU after the first three, and an exact
//! reshape/transpose/reshape pixel shuffle. Batch is specialized to one and
//! spatial dimensions to 224 × 224. Every executable node, edge, attribute and
//! embedded weight is validated before the GPU backend allocates resources.
//! RGB-to-Y conversion and Y-to-RGB reconstruction belong to the GPU backend;
//! this source model's actual interface remains single-channel luminance.
//! There is deliberately no execution, image staging, or CPU fallback here.

use crate::artifact::{
    ArtifactDimensionValue, ArtifactInputRequirement, ArtifactInterface, ArtifactShape,
    ArtifactValue, ArtifactValueKind,
};
use prost::Message;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// OIHW contiguous FP32 weights; pads are ONNX [top, left, bottom, right].
#[derive(Clone, Debug)]
pub struct ConvLayer {
    pub weights: Vec<f32>,
    pub bias: Vec<f32>,
    pub in_channels: u32,
    pub out_channels: u32,
    pub kernel: [u32; 2],
    pub stride: [u32; 2],
    pub pads: [u32; 4],
    pub relu: bool,
}

#[derive(Clone, Debug)]
pub struct ModelPlan {
    pub input_width: u32,
    pub input_height: u32,
    pub output_width: u32,
    pub output_height: u32,
    pub scale: u32,
    pub input_name: String,
    pub output_name: String,
    /// SHA-256 of the original, unmodified ONNX bytes.
    pub source_sha256: String,
    /// Length of the original artifact, retained without another file read.
    pub source_byte_length: u64,
    /// Original artifact manifest, including initializer-backed graph inputs.
    pub source_interface: ArtifactInterface,
}

#[derive(Clone, Debug)]
pub struct GpuModel {
    pub plan: ModelPlan,
    pub layers: Vec<ConvLayer>,
}

impl GpuModel {
    /// Compile only the supported resident profile; never approximate other graphs.
    /// CPU work here is model preparation, not per-frame tensor processing.
    pub fn from_onnx(bytes: &[u8]) -> Result<Self, String> {
        let interface = crate::inspect_onnx(bytes).map_err(|error| error.to_string())?;
        let model = proto::Model::decode(bytes).map_err(|error| error.to_string())?;
        if !model.training_info.is_empty() || !model.functions.is_empty() {
            return Err("resident profile does not support training graphs or local functions".into());
        }
        if interface.opset_imports.len() != 1
            || !interface.opset_imports[0].domain.as_deref().unwrap_or("").is_empty()
            || interface.opset_imports[0].version != Some(10)
        {
            return Err("resident profile requires exactly the standard ONNX opset 10".into());
        }
        if !matches!(interface.ir_version, Some(3..=10)) {
            return Err("unsupported ONNX IR version for resident profile".into());
        }
        let graph = model.graph.ok_or("ONNX model has no graph")?;
        if !graph.sparse_initializer.is_empty() || !graph.quantization_annotation.is_empty() {
            return Err("resident profile requires dense unquantized initializers".into());
        }
        let runtime_inputs: Vec<_> = interface.inputs.iter()
            .filter(|input| input.requirement != ArtifactInputRequirement::OverridableInitializer)
            .collect();
        if runtime_inputs.len() != 1
            || runtime_inputs[0].requirement != ArtifactInputRequirement::Required
            || interface.outputs.len() != 1
        {
            return Err("resident profile requires one required image input and one output".into());
        }
        let input = &runtime_inputs[0].value;
        let output = &interface.outputs[0];
        let input_batch = image_shape(input, 224)?;
        let output_batch = image_shape(output, 672)?;
        if input_batch != output_batch {
            return Err("input and output batch declarations differ".into());
        }
        let mut tensors = BTreeMap::new();
        let mut available = BTreeSet::new();
        available.insert(input.name.clone());
        for tensor in graph.initializer {
            if tensor.name.is_empty() || !available.insert(tensor.name.clone()) {
                return Err(format!("duplicate or empty initializer name {:?}", tensor.name));
            }
            validate_tensor_storage(&tensor)?;
            tensors.insert(tensor.name.clone(), tensor);
        }
        // Initializer-backed inputs are legal in the original IR-4 artifact,
        // but must describe the very same immutable weights used by this plan.
        for declared in &interface.inputs {
            if declared.requirement == ArtifactInputRequirement::OverridableInitializer {
                let tensor = tensors.get(&declared.value.name).ok_or("missing initializer")?;
                validate_initializer_interface(&declared.value, tensor)?;
            }
        }
        let mut uses = BTreeMap::<String, usize>::new();
        let mut executable = Vec::new();
        for node in &graph.node {
            if !node.domain.is_empty() || !node.overload.is_empty() {
                return Err("custom-domain or overloaded nodes are unsupported".into());
            }
            if node.output.len() != 1 || node.output[0].is_empty() {
                return Err(format!("{} requires one named output", node.op_type));
            }
            for edge in &node.input {
                if edge.is_empty() || !available.contains(edge) {
                    return Err(format!("{} references unavailable edge {edge:?}", node.op_type));
                }
                *uses.entry(edge.clone()).or_default() += 1;
            }
            if !available.insert(node.output[0].clone()) {
                return Err(format!("duplicate produced edge {:?}", node.output[0]));
            }
            if node.op_type == "Constant" {
                if !node.input.is_empty() {
                    return Err("Constant must have no inputs".into());
                }
                let attributes = attributes(node)?;
                if attributes.len() != 1 {
                    return Err("Constant must contain only its tensor value attribute".into());
                }
                let value = attributes.get("value").ok_or("Constant requires value")?;
                value.expect_type(4)?;
                let tensor = value.t.as_ref().ok_or("Constant requires a tensor value")?.clone();
                validate_tensor_storage(&tensor)?;
                if tensor.data_type != Some(7) {
                    return Err("only INT64 reshape constants are supported".into());
                }
                tensors.insert(node.output[0].clone(), tensor);
            } else {
                executable.push(node);
            }
        }
        if !available.contains(&output.name) {
            return Err("declared output has no producer".into());
        }
        for edge in &available {
            let expected = usize::from(edge != &output.name);
            if uses.get(edge).copied().unwrap_or(0) != expected {
                return Err(format!("resident profile rejects branched or unused edge {edge:?}"));
            }
        }
        let operations = ["Conv", "Relu", "Conv", "Relu", "Conv", "Relu", "Conv", "Reshape", "Transpose", "Reshape"];
        if executable.len() != operations.len()
            || executable.iter().zip(operations).any(|(node, operation)| node.op_type != operation)
        {
            return Err("unsupported resident topology: expected Conv/Relu × 3, Conv, Reshape, Transpose, Reshape".into());
        }
        let profile = [(1, 64, 5), (64, 64, 3), (64, 32, 3), (32, 9, 3)];
        let mut layers = Vec::with_capacity(4);
        let mut previous = input.name.as_str();
        for (index, (in_channels, out_channels, kernel)) in profile.into_iter().enumerate() {
            let node = executable[index * 2];
            if node.input.len() != 3 || node.input[0] != previous {
                return Err("convolution must consume the preceding image, weights and bias".into());
            }
            let attrs = attributes(node)?;
            for key in attrs.keys() {
                if !["auto_pad", "dilations", "group", "kernel_shape", "pads", "strides"].contains(key) {
                    return Err(format!("unsupported Conv attribute {key:?}"));
                }
            }
            expect_int(&attrs, "group", 1)?;
            expect_ints(&attrs, "dilations", &[1, 1])?;
            expect_ints(&attrs, "strides", &[1, 1])?;
            expect_ints(&attrs, "kernel_shape", &[kernel, kernel])?;
            // ONNX defaults to zero padding, not the profile's same padding.
            let padding = kernel / 2;
            let pads = attrs.get("pads").ok_or("Conv must declare same padding explicitly")?;
            pads.expect_type(7)?;
            if pads.ints != [padding; 4] {
                return Err("Conv pads do not match the supported same-padding profile".into());
            }
            if let Some(auto_pad) = attrs.get("auto_pad") {
                auto_pad.expect_type(3)?;
                if auto_pad.s.as_deref() != Some(b"NOTSET".as_slice()) {
                    return Err("Conv auto_pad must be NOTSET".into());
                }
            }
            let weight = tensors.get(&node.input[1]).ok_or("Conv weights must be embedded initializers")?;
            let bias = tensors.get(&node.input[2]).ok_or("Conv bias must be an embedded initializer")?;
            if weight.dims != [out_channels, in_channels, kernel, kernel] || bias.dims != [out_channels] {
                return Err(format!("Conv {index} weight or bias dimensions do not match the resident profile"));
            }
            let weights = float_values(weight)?;
            let bias = float_values(bias)?;
            previous = &node.output[0];
            if index < 3 {
                let relu = executable[index * 2 + 1];
                if relu.input.as_slice() != [previous] || !relu.attribute.is_empty() {
                    return Err("Relu must consume only the preceding convolution, without attributes".into());
                }
                previous = &relu.output[0];
            }
            layers.push(ConvLayer {
                weights, bias, in_channels: in_channels as u32, out_channels: out_channels as u32,
                kernel: [kernel as u32; 2], stride: [1; 2], pads: [padding as u32; 4], relu: index < 3,
            });
        }
        let first = executable[7];
        validate_reshape(first, previous, &tensors, &[1, 1, 3, 3, 224, 224])?;
        let transpose = executable[8];
        if transpose.input.as_slice() != [first.output[0].as_str()] {
            return Err("pixel shuffle Transpose must consume the first Reshape".into());
        }
        let attrs = attributes(transpose)?;
        if attrs.len() != 1 {
            return Err("pixel shuffle Transpose requires exactly perm".into());
        }
        let perm = attrs.get("perm").ok_or("pixel shuffle Transpose requires perm")?;
        perm.expect_type(7)?;
        if perm.ints != [0, 1, 4, 2, 5, 3] {
            return Err("Transpose is not the required channel=(dy*3+dx) pixel shuffle".into());
        }
        let last = executable[9];
        validate_reshape(last, &transpose.output[0], &tensors, &[1, 1, 672, 672])?;
        if last.output[0] != output.name {
            return Err("pixel shuffle result must be the declared output".into());
        }
        let plan = ModelPlan {
            input_width: 224, input_height: 224, output_width: 672, output_height: 672, scale: 3,
            input_name: input.name.clone(), output_name: output.name.clone(),
            source_sha256: format!("{:x}", Sha256::digest(bytes)), source_interface: interface,
            source_byte_length: bytes.len() as u64,
        };
        Ok(Self { plan, layers })
    }
}

fn image_shape(value: &ArtifactValue, size: i64) -> Result<ArtifactDimensionValue, String> {
    let value_type = value.value_type.as_ref().ok_or("image has no tensor type")?;
    let ArtifactValueKind::Tensor { element_type: Some(1), shape: ArtifactShape::KnownRank(shape) } = &value_type.kind else {
        return Err("image interface must be a shaped FP32 tensor".into());
    };
    if shape.len() != 4 || shape[1].value != ArtifactDimensionValue::Static(1)
        || shape[2].value != ArtifactDimensionValue::Static(size)
        || shape[3].value != ArtifactDimensionValue::Static(size)
    {
        return Err(format!("image interface must be NCHW [batch,1,{size},{size}]"));
    }
    match &shape[0].value {
        ArtifactDimensionValue::Static(1) => Ok(shape[0].value.clone()),
        ArtifactDimensionValue::Symbol(name) if !name.is_empty() => Ok(shape[0].value.clone()),
        _ => Err("resident batch must be one or a named dimension specialized to one".into()),
    }
}

fn validate_initializer_interface(value: &ArtifactValue, tensor: &proto::Tensor) -> Result<(), String> {
    let Some(value_type) = &value.value_type else { return Err("initializer input has no type".into()); };
    let ArtifactValueKind::Tensor { element_type, shape: ArtifactShape::KnownRank(shape) } = &value_type.kind else {
        return Err("initializer input must declare a tensor shape".into());
    };
    if *element_type != tensor.data_type || shape.len() != tensor.dims.len()
        || shape.iter().zip(&tensor.dims).any(|(dimension, size)| dimension.value != ArtifactDimensionValue::Static(*size))
    {
        return Err(format!("initializer input {:?} disagrees with its embedded tensor", value.name));
    }
    Ok(())
}

fn validate_tensor_storage(tensor: &proto::Tensor) -> Result<(), String> {
    if tensor.data_location.unwrap_or(0) != 0 || !tensor.external_data.is_empty() || tensor.segment.is_some() {
        return Err("resident weights and constants must be complete embedded tensors".into());
    }
    if !tensor.int32_data.is_empty() || !tensor.string_data.is_empty()
        || !tensor.double_data.is_empty() || !tensor.uint64_data.is_empty()
    {
        return Err("unsupported tensor payload encoding".into());
    }
    if tensor.dims.is_empty() || tensor.dims.iter().any(|dimension| *dimension <= 0) {
        return Err("resident tensor dimensions must be positive and non-scalar".into());
    }
    Ok(())
}

fn element_count(tensor: &proto::Tensor) -> Result<usize, String> {
    tensor.dims.iter().try_fold(1usize, |count, dimension| {
        let dimension = usize::try_from(*dimension).map_err(|_| "invalid tensor dimension")?;
        count.checked_mul(dimension).ok_or_else(|| "tensor element count overflows".to_string())
    })
}

fn float_values(tensor: &proto::Tensor) -> Result<Vec<f32>, String> {
    if tensor.data_type != Some(1) || !tensor.int64_data.is_empty() {
        return Err("convolution weights and biases must be FP32".into());
    }
    let count = element_count(tensor)?;
    let values = if let Some(raw) = &tensor.raw_data {
        if !tensor.float_data.is_empty() || count.checked_mul(4) != Some(raw.len()) {
            return Err("FP32 raw payload length or conflicting typed payload is invalid".into());
        }
        raw.chunks_exact(4).map(|chunk| f32::from_le_bytes(chunk.try_into().expect("four bytes"))).collect()
    } else {
        if tensor.float_data.len() != count {
            return Err("FP32 typed payload length does not match dimensions".into());
        }
        tensor.float_data.clone()
    };
    if values.iter().any(|value| !value.is_finite()) {
        return Err("convolution weights or biases contain non-finite FP32 values".into());
    }
    Ok(values)
}

fn int64_values(tensor: &proto::Tensor) -> Result<Vec<i64>, String> {
    if tensor.data_type != Some(7) || !tensor.float_data.is_empty() {
        return Err("reshape shapes must be INT64".into());
    }
    let count = element_count(tensor)?;
    if let Some(raw) = &tensor.raw_data {
        if !tensor.int64_data.is_empty() || count.checked_mul(8) != Some(raw.len()) {
            return Err("INT64 raw payload length or conflicting typed payload is invalid".into());
        }
        Ok(raw.chunks_exact(8).map(|chunk| i64::from_le_bytes(chunk.try_into().expect("eight bytes"))).collect())
    } else if tensor.int64_data.len() == count {
        Ok(tensor.int64_data.clone())
    } else {
        Err("INT64 typed payload length does not match dimensions".into())
    }
}

fn validate_reshape(node: &proto::Node, previous: &str, tensors: &BTreeMap<String, proto::Tensor>, expected: &[i64]) -> Result<(), String> {
    if node.input.len() != 2 || node.input[0] != previous || !node.attribute.is_empty() {
        return Err("pixel shuffle Reshape must have image and constant shape inputs, without attributes".into());
    }
    let tensor = tensors.get(&node.input[1]).ok_or("Reshape shape is not constant")?;
    if tensor.dims != [expected.len() as i64] {
        return Err("Reshape shape tensor must be a vector of the correct rank".into());
    }
    let actual = int64_values(tensor)?;
    // A leading -1 infers batch one from the validated convolution element count.
    // Zero copies that same batch dimension in ONNX-10. No other inference is allowed.
    if actual.len() != expected.len() || !matches!(actual[0], -1..=1) || actual[1..] != expected[1..] {
        return Err("Reshape does not implement the exact resident pixelshuffle3 mapping".into());
    }
    Ok(())
}

fn attributes(node: &proto::Node) -> Result<BTreeMap<&str, &proto::Attribute>, String> {
    let mut result = BTreeMap::new();
    for attribute in &node.attribute {
        attribute.validate_payload()?;
        if attribute.name.is_empty() || result.insert(attribute.name.as_str(), attribute).is_some() {
            return Err("duplicate or unnamed node attribute".into());
        }
    }
    Ok(result)
}

fn expect_int(attrs: &BTreeMap<&str, &proto::Attribute>, name: &str, expected: i64) -> Result<(), String> {
    if let Some(attribute) = attrs.get(name) {
        attribute.expect_type(2)?;
        if attribute.i != Some(expected) { return Err(format!("unsupported {name} value")); }
    }
    Ok(())
}

fn expect_ints(attrs: &BTreeMap<&str, &proto::Attribute>, name: &str, expected: &[i64]) -> Result<(), String> {
    if let Some(attribute) = attrs.get(name) {
        attribute.expect_type(7)?;
        if attribute.ints != expected { return Err(format!("unsupported {name} values")); }
    }
    Ok(())
}

// Selected official ONNX protobuf fields. Unlike artifact::proto, this schema
// deliberately decodes initializer and Constant payloads for one-time upload.
pub(crate) mod proto {
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Model {
        #[prost(message, optional, tag = "7")]
        pub graph: Option<Graph>,
        #[prost(bytes = "vec", repeated, tag = "20")]
        pub training_info: Vec<Vec<u8>>,
        #[prost(bytes = "vec", repeated, tag = "25")]
        pub functions: Vec<Vec<u8>>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Graph {
        #[prost(message, repeated, tag = "1")]
        pub node: Vec<Node>,
        #[prost(message, repeated, tag = "5")]
        pub initializer: Vec<Tensor>,
        #[prost(bytes = "vec", repeated, tag = "14")]
        pub quantization_annotation: Vec<Vec<u8>>,
        #[prost(bytes = "vec", repeated, tag = "15")]
        pub sparse_initializer: Vec<Vec<u8>>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Node {
        #[prost(string, repeated, tag = "1")]
        pub input: Vec<String>,
        #[prost(string, repeated, tag = "2")]
        pub output: Vec<String>,
        #[prost(string, tag = "4")]
        pub op_type: String,
        #[prost(message, repeated, tag = "5")]
        pub attribute: Vec<Attribute>,
        #[prost(string, tag = "7")]
        pub domain: String,
        #[prost(string, tag = "8")]
        pub overload: String,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Attribute {
        #[prost(string, tag = "1")]
        pub name: String,
        #[prost(float, optional, tag = "2")]
        pub f: Option<f32>,
        #[prost(int64, optional, tag = "3")]
        pub i: Option<i64>,
        #[prost(bytes = "vec", optional, tag = "4")]
        pub s: Option<Vec<u8>>,
        #[prost(message, optional, tag = "5")]
        pub t: Option<Tensor>,
        #[prost(bytes = "vec", optional, tag = "6")]
        pub g: Option<Vec<u8>>,
        #[prost(float, repeated, tag = "7")]
        pub floats: Vec<f32>,
        #[prost(int64, repeated, tag = "8")]
        pub ints: Vec<i64>,
        #[prost(bytes = "vec", repeated, tag = "9")]
        pub strings: Vec<Vec<u8>>,
        #[prost(message, repeated, tag = "10")]
        pub tensors: Vec<Tensor>,
        #[prost(bytes = "vec", repeated, tag = "11")]
        pub graphs: Vec<Vec<u8>>,
        #[prost(bytes = "vec", optional, tag = "14")]
        pub tp: Option<Vec<u8>>,
        #[prost(bytes = "vec", repeated, tag = "15")]
        pub type_protos: Vec<Vec<u8>>,
        #[prost(int32, optional, tag = "20")]
        pub r#type: Option<i32>,
        #[prost(string, tag = "21")]
        pub ref_attr_name: String,
        #[prost(bytes = "vec", optional, tag = "22")]
        pub sparse_tensor: Option<Vec<u8>>,
        #[prost(bytes = "vec", repeated, tag = "23")]
        pub sparse_tensors: Vec<Vec<u8>>,
    }
    impl Attribute {
        pub fn expect_type(&self, expected: i32) -> Result<(), String> {
            if self.r#type == Some(expected) { Ok(()) }
            else { Err(format!("attribute {:?} has the wrong ONNX type", self.name)) }
        }
        pub fn validate_payload(&self) -> Result<(), String> {
            if !self.ref_attr_name.is_empty() || self.f.is_some() || self.g.is_some()
                || !self.floats.is_empty() || !self.strings.is_empty() || !self.tensors.is_empty()
                || !self.graphs.is_empty() || self.tp.is_some() || !self.type_protos.is_empty()
                || self.sparse_tensor.is_some() || !self.sparse_tensors.is_empty()
            {
                return Err(format!("unsupported attribute payload {:?}", self.name));
            }
            let valid = match self.r#type {
                Some(2) => self.i.is_some() && self.s.is_none() && self.t.is_none() && self.ints.is_empty(),
                Some(3) => self.i.is_none() && self.s.is_some() && self.t.is_none() && self.ints.is_empty(),
                Some(4) => self.i.is_none() && self.s.is_none() && self.t.is_some() && self.ints.is_empty(),
                Some(7) => self.i.is_none() && self.s.is_none() && self.t.is_none(),
                _ => false,
            };
            if valid { Ok(()) } else { Err(format!("invalid attribute payload {:?}", self.name)) }
        }
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Tensor {
        #[prost(int64, repeated, tag = "1")]
        pub dims: Vec<i64>,
        #[prost(int32, optional, tag = "2")]
        pub data_type: Option<i32>,
        #[prost(bytes = "vec", optional, tag = "3")]
        pub segment: Option<Vec<u8>>,
        #[prost(float, repeated, tag = "4")]
        pub float_data: Vec<f32>,
        #[prost(int32, repeated, tag = "5")]
        pub int32_data: Vec<i32>,
        #[prost(bytes = "vec", repeated, tag = "6")]
        pub string_data: Vec<Vec<u8>>,
        #[prost(int64, repeated, tag = "7")]
        pub int64_data: Vec<i64>,
        #[prost(string, tag = "8")]
        pub name: String,
        #[prost(bytes = "vec", optional, tag = "9")]
        pub raw_data: Option<Vec<u8>>,
        #[prost(double, repeated, tag = "10")]
        pub double_data: Vec<f64>,
        #[prost(uint64, repeated, tag = "11")]
        pub uint64_data: Vec<u64>,
        #[prost(bytes = "vec", repeated, tag = "13")]
        pub external_data: Vec<Vec<u8>>,
        #[prost(int32, optional, tag = "14")]
        pub data_location: Option<i32>,
    }
}

#[cfg(test)]
#[path = "gpu_model_tests.rs"]
mod tests;
