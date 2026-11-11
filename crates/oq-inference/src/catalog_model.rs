//! Preparation-only ONNX validation and native GPU graph descriptors.
//!
//! Original graph math and artifact metadata are retained; this module neither
//! optimizes a CPU graph nor executes image tensors. The native backend compiles
//! the selected output, including constant shape expressions, once at preparation.

use crate::artifact::{ArtifactDimensionValue, ArtifactInputRequirement, ArtifactShape, ArtifactValue, ArtifactValueKind};
use crate::gpu_model::{proto, ModelPlan};
use prost::Message;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

const MAX_DIMENSION: usize = 16_384;
const MAX_BYTES: usize = 2_147_483_648;
const OPS: &[&str] = &[
    "Conv", "PRelu", "DepthToSpace", "Constant", "Resize", "Add", "Relu", "MaxPool",
    "Concat", "Shape", "Gather", "Unsqueeze", "Slice", "Cast", "Sigmoid", "Clip",
    "GlobalAveragePool", "Reshape", "MatMul", "Expand", "Mul", "BatchNormalization",
    "InstanceNormalization", "Sub", "Div", "ConstantOfShape", "Transpose", "Pad",
    "Neg", "Squeeze", "Split", "Softmax", "ArgMax",
];

#[derive(Clone, Debug)]
pub struct CatalogModel {
    pub plan: ModelPlan,
    pub model_id: String,
    pub graph_json: Vec<u8>,
    pub tensor_blob: Vec<u8>,
    pub input_shape: Vec<usize>,
    pub output_shape: Vec<usize>,
    pub output_name: String,
    pub output_dtype: i32,
    #[cfg(feature = "gpu-diagnostics")]
    pub diagnostic_logits_shape: Option<Vec<usize>>,
}

impl CatalogModel {
    pub fn from_onnx(bytes: &[u8], model_id: &str, input_width: u32, input_height: u32) -> Result<Self, String> {
        let (fixed, scale, output_dtype, selected_name) = match model_id {
            "yolov8n" => (Some(640), 1, 1, None),
            "yolo26n-sem" => (Some(640), 1, 2, None),
            "u2netp" => (Some(320), 1, 1, Some("1959")),
            "modnet" => (Some(512), 1, 1, None),
            "midas-small" => (Some(256), 1, 1, Some("output_depth")),
            "realesrgan-x4" => (None, 4, 1, None),
            _ => return Err(format!("unsupported resident catalog model {model_id:?}")),
        };
        if input_width == 0 || input_height == 0 || fixed.is_some_and(|size| input_width != size || input_height != size) {
            return Err(format!("invalid prepared dimensions {input_width}x{input_height} for {model_id}"));
        }
        let output_width = input_width.checked_mul(scale).ok_or("output width overflow")?;
        let output_height = input_height.checked_mul(scale).ok_or("output height overflow")?;
        let input_shape = vec![1, 3, input_height as usize, input_width as usize];
        checked_storage(&input_shape, 4)?;
        let output_shape = match model_id {
            "yolov8n" => vec![1, 84, 8400],
            "yolo26n-sem" => vec![1, 640, 640],
            "midas-small" => vec![1, 256, 256],
            "realesrgan-x4" => vec![1, 3, output_height as usize, output_width as usize],
            _ => vec![1, 1, output_height as usize, output_width as usize],
        };
        // Export is FP32 even when the source class map is UINT8.
        checked_storage(&output_shape, 4)?;
        let interface = crate::inspect_onnx(bytes).map_err(|error| error.to_string())?;
        let model = proto::Model::decode(bytes).map_err(|error| error.to_string())?;
        if !model.training_info.is_empty() || !model.functions.is_empty() {
            return Err("catalog graphs cannot contain training graphs or local functions".into());
        }
        if !matches!(interface.ir_version, Some(3..=10)) || interface.opset_imports.len() != 1 {
            return Err("unsupported ONNX IR or multiple operator domains".into());
        }
        let opset = &interface.opset_imports[0];
        if !opset.domain.as_deref().unwrap_or("").is_empty() || !matches!(opset.version, Some(11..=17)) {
            return Err("catalog requires a standard ONNX opset between 11 and 17".into());
        }
        let graph = model.graph.ok_or("ONNX model has no graph")?;
        if !graph.sparse_initializer.is_empty() || !graph.quantization_annotation.is_empty() {
            return Err("sparse or quantization-annotated catalog graphs are unsupported".into());
        }
        let runtime_inputs: Vec<_> = interface.inputs.iter().filter(|input| input.requirement != ArtifactInputRequirement::OverridableInitializer).collect();
        if runtime_inputs.len() != 1 || runtime_inputs[0].requirement != ArtifactInputRequirement::Required {
            return Err("catalog graph requires exactly one required image input".into());
        }
        let input = &runtime_inputs[0].value;
        let dynamic_input = matches!(model_id, "modnet" | "realesrgan-x4");
        validate_interface(input, 1, &input_shape, |axis| dynamic_input && axis != 1)?;
        if interface.outputs.len() != if model_id == "u2netp" { 7 } else { 1 } {
            return Err("catalog output count does not match task interface".into());
        }
        let output = if let Some(name) = selected_name {
            interface.outputs.iter().find(|value| value.name == name).ok_or_else(|| format!("missing selected output {name}"))?
        } else {
            &interface.outputs[0]
        };
        let dynamic_output = matches!(model_id, "modnet" | "realesrgan-x4" | "midas-small");
        validate_interface(output, output_dtype, &output_shape, |axis| dynamic_output && (model_id == "midas-small" || axis != 1))?;
        // The bounded batch specialization must refer to the same source batch.
        if dynamic_input && batch_dimension(input)? != batch_dimension(output)? {
            return Err("input and output batch declarations differ".into());
        }
        let mut outputs = Vec::with_capacity(interface.outputs.len());
        for value in &interface.outputs {
            if model_id == "u2netp" {
                if !["1959", "1960", "1961", "1962", "1963", "1964", "1965"].contains(&value.name.as_str()) {
                    return Err("unexpected U2-Net side output".into());
                }
                validate_interface(value, 1, &output_shape, |_| value.name != "1959" && value.name != "1960")?;
            }
            outputs.push(json!({"name":value.name,"shape":output_shape,"dtype":output_dtype}));
        }
        let mut available = BTreeSet::new();
        available.insert(input.name.clone());
        let mut tensor_blob = Vec::new();
        let mut initializers = Vec::with_capacity(graph.initializer.len());
        for tensor in &graph.initializer {
            if tensor.name.is_empty() || !available.insert(tensor.name.clone()) {
                return Err(format!("duplicate or empty initializer {:?}", tensor.name));
            }
            let mut descriptor = tensor_descriptor(tensor, &mut tensor_blob)?;
            descriptor.as_object_mut().ok_or("invalid tensor descriptor")?.insert("name".into(), json!(tensor.name));
            initializers.push(descriptor);
        }
        for declared in &interface.inputs {
            if declared.requirement == ArtifactInputRequirement::OverridableInitializer {
                let tensor = graph.initializer.iter().find(|tensor| tensor.name == declared.value.name).ok_or("initializer-backed input has no dense initializer")?;
                let shape = tensor_shape(tensor)?;
                validate_interface(&declared.value, tensor.data_type.ok_or("missing initializer dtype")?, &shape, |_| false)?;
            }
        }
        let mut nodes = Vec::with_capacity(graph.node.len());
        if graph.node.is_empty() { return Err("catalog graph contains no nodes".into()); }
        for (index, node) in graph.node.iter().enumerate() {
            if !node.domain.is_empty() || !node.overload.is_empty() || !OPS.contains(&node.op_type.as_str()) {
                return Err(format!("unsupported catalog operation {:?}", node.op_type));
            }
            validate_node_arity(node)?;
            for edge in &node.input {
                if !edge.is_empty() && !available.contains(edge) {
                    return Err(format!("{} consumes unavailable edge {edge:?}", node.op_type));
                }
            }
            let mut attributes = Map::new();
            for attribute in &node.attribute {
                if attribute.name.is_empty() || attributes.contains_key(&attribute.name) {
                    return Err("empty or duplicate node attribute".into());
                }
                attributes.insert(attribute.name.clone(), attribute_value(attribute, &mut tensor_blob)?);
            }
            if node.op_type == "Constant" && (attributes.len() != 1 || !attributes.contains_key("value")) {
                return Err("Constant requires exactly one tensor value".into());
            }
            for edge in &node.output {
                if edge.is_empty() || !available.insert(edge.clone()) {
                    return Err(format!("duplicate or empty produced edge {edge:?}"));
                }
            }
            nodes.push(json!({"op":node.op_type,"name":format!("node_{index}_{}",node.op_type),"inputs":node.input,"outputs":node.output,"attributes":attributes}));
        }
        for value in &interface.outputs {
            if !available.contains(&value.name) { return Err(format!("output {:?} has no producer", value.name)); }
        }
        let output_name = output.name.clone();
        let graph_descriptor = json!({"nodes":nodes,"initializers":initializers,
            "inputs":[{"name":input.name,"shape":input_shape,"dtype":1}],"outputs":outputs,
            "selected_output":output_name,"opset":opset.version,"model_id":model_id});
        #[cfg(feature = "gpu-diagnostics")]
        let mut graph_descriptor = graph_descriptor;
        #[cfg(feature = "gpu-diagnostics")]
        let diagnostic_logits_shape = if model_id == "yolo26n-sem" {
            let argmax: Vec<_> = graph.node.iter().filter(|n| n.op_type == "ArgMax").collect();
            if argmax.len()!=1 || argmax[0].input.len()!=1 { return Err("semantic diagnostic requires one original ArgMax".into()); }
            let logits_name=&argmax[0].input[0];
            let resize=graph.node.iter().find(|n| n.output.contains(logits_name)).ok_or("missing semantic logits producer")?;
            if resize.op_type!="Resize" { return Err("semantic diagnostic logits must be spatial Resize".into()); }
            let conv=graph.node.iter().find(|n| n.output.first()==resize.input.first()).ok_or("missing semantic classifier producer")?;
            if conv.op_type!="Conv" || conv.input.len()<2 { return Err("semantic diagnostic classifier must be Conv".into()); }
            let weights=graph.initializer.iter().find(|t|t.name==conv.input[1]).ok_or("missing semantic classifier weights")?;
            let classes=usize::try_from(*weights.dims.first().ok_or("missing semantic classifier channels")?).map_err(|_|"invalid semantic classifier channels")?;
            if classes==0 { return Err("empty semantic classifier channels".into()); }
            let shape=vec![1,classes,input_height as usize,input_width as usize];
            graph_descriptor["diagnostic_output"]=json!({"name":logits_name,"shape":shape,"dtype":1});
            Some(shape)
        } else { None };
        let graph_json = serde_json::to_vec(&graph_descriptor).map_err(|error| error.to_string())?;
        let plan = ModelPlan {
            input_width, input_height, output_width, output_height, scale,
            input_name: input.name.clone(), output_name: output_name.clone(),
            source_sha256: format!("{:x}", Sha256::digest(bytes)),
            source_byte_length: bytes.len() as u64, source_interface: interface,
        };
        Ok(Self { plan, model_id: model_id.into(), graph_json, tensor_blob, input_shape, output_shape, output_name, output_dtype,
            #[cfg(feature = "gpu-diagnostics")]
            diagnostic_logits_shape })
    }
}

fn dimensions(value: &ArtifactValue) -> Result<(i32, &[crate::artifact::ArtifactDimension]), String> {
    match value.value_type.as_ref().map(|value| &value.kind) {
        Some(ArtifactValueKind::Tensor { element_type: Some(dtype), shape: ArtifactShape::KnownRank(dims) }) => Ok((*dtype, dims)),
        _ => Err(format!("{:?} requires a ranked dense tensor interface", value.name)),
    }
}

fn batch_dimension(value: &ArtifactValue) -> Result<&ArtifactDimensionValue, String> {
    dimensions(value)?.1.first().map(|dimension| &dimension.value).ok_or("missing batch dimension".into())
}

fn validate_interface(value: &ArtifactValue, expected_dtype: i32, expected: &[usize], dynamic: impl Fn(usize) -> bool) -> Result<(), String> {
    let (dtype, dims) = dimensions(value)?;
    if value.name.is_empty() || dtype != expected_dtype || dims.len() != expected.len() {
        return Err(format!("invalid task tensor interface {:?}", value.name));
    }
    for (axis, (dimension, expected)) in dims.iter().zip(expected).enumerate() {
        match &dimension.value {
            ArtifactDimensionValue::Static(actual) if *actual > 0 && usize::try_from(*actual).ok() == Some(*expected) => (),
            ArtifactDimensionValue::Symbol(symbol) if dynamic(axis) && !symbol.is_empty() => (),
            ArtifactDimensionValue::Dynamic if dynamic(axis) => (),
            ArtifactDimensionValue::Static(-1) if dynamic(axis) => (),
            _ => return Err(format!("{:?} axis {axis} does not match prepared dimension {expected}", value.name)),
        }
    }
    Ok(())
}

fn checked_storage(shape: &[usize], element_bytes: usize) -> Result<usize, String> {
    if shape.len() > 8 || shape.iter().any(|dimension| *dimension > MAX_DIMENSION) {
        return Err("tensor rank or dimension exceeds bounded GPU profile".into());
    }
    let count = shape.iter().try_fold(1usize, |count, dimension| count.checked_mul(*dimension)).ok_or("tensor element count overflow")?;
    let bytes = count.checked_mul(element_bytes).ok_or("tensor byte count overflow")?;
    if bytes > MAX_BYTES { return Err("tensor exceeds bounded GPU byte allocation".into()); }
    Ok(bytes)
}

fn tensor_shape(tensor: &proto::Tensor) -> Result<Vec<usize>, String> {
    tensor.dims.iter().map(|dimension| usize::try_from(*dimension).map_err(|_| "negative tensor dimension".into())).collect()
}

fn tensor_descriptor(tensor: &proto::Tensor, blob: &mut Vec<u8>) -> Result<Value, String> {
    if tensor.segment.is_some() || !tensor.external_data.is_empty() || !matches!(tensor.data_location, None | Some(0))
        || !tensor.string_data.is_empty() || !tensor.double_data.is_empty() || !tensor.uint64_data.is_empty() {
        return Err(format!("unsupported storage for tensor {:?}", tensor.name));
    }
    let dtype = tensor.data_type.ok_or("tensor has no dtype")?;
    let element_bytes = match dtype { 1 | 6 => 4, 7 => 8, 2 => 1, _ => return Err(format!("unsupported tensor dtype {dtype}")) };
    let shape = tensor_shape(tensor)?;
    let length = checked_storage(&shape, element_bytes)?;
    let count = length / element_bytes;
    let typed_count = usize::from(!tensor.float_data.is_empty()) + usize::from(!tensor.int32_data.is_empty()) + usize::from(!tensor.int64_data.is_empty());
    if tensor.raw_data.is_some() && typed_count != 0 { return Err("tensor mixes raw and typed payloads".into()); }
    if typed_count > 1 { return Err("tensor mixes typed payloads".into()); }
    if (dtype != 1 && !tensor.float_data.is_empty()) || (!matches!(dtype, 2 | 6) && !tensor.int32_data.is_empty()) || (dtype != 7 && !tensor.int64_data.is_empty()) {
        return Err("tensor payload does not match dtype".into());
    }
    if let Some(raw) = &tensor.raw_data {
        if raw.len() != length { return Err("raw tensor payload length does not match shape".into()); }
        if dtype == 1 && raw.chunks_exact(4).any(|bytes| !f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]).is_finite()) {
            return Err("nonfinite FP32 tensor payload".into());
        }
    } else {
        let actual = match dtype { 1 => tensor.float_data.len(), 7 => tensor.int64_data.len(), _ => tensor.int32_data.len() };
        if actual != count { return Err("typed tensor payload length does not match shape".into()); }
        if tensor.float_data.iter().any(|value| !value.is_finite()) || (dtype == 2 && tensor.int32_data.iter().any(|value| !(0..=255).contains(value))) {
            return Err("nonfinite or out-of-range tensor payload".into());
        }
    }
    let offset = blob.len();
    if offset.checked_add(length).filter(|total| *total <= MAX_BYTES).is_none() { return Err("tensor blob allocation exceeds bounded profile".into()); }
    blob.try_reserve(length).map_err(|error| format!("cannot allocate tensor blob: {error}"))?;
    if let Some(raw) = &tensor.raw_data { blob.extend_from_slice(raw); }
    else {
        match dtype {
            1 => for value in &tensor.float_data { blob.extend_from_slice(&value.to_le_bytes()); },
            6 => for value in &tensor.int32_data { blob.extend_from_slice(&value.to_le_bytes()); },
            7 => for value in &tensor.int64_data { blob.extend_from_slice(&value.to_le_bytes()); },
            2 => for value in &tensor.int32_data { blob.push(*value as u8); },
            _ => unreachable!(),
        }
    }
    Ok(json!({"shape":shape,"dtype":dtype,"offset":offset,"length":length}))
}

fn attribute_value(attribute: &proto::Attribute, blob: &mut Vec<u8>) -> Result<Value, String> {
    let kind = attribute.r#type.ok_or("attribute has no type")?;
    if !attribute.ref_attr_name.is_empty() || attribute.g.is_some() || !attribute.strings.is_empty()
        || !attribute.tensors.is_empty() || !attribute.graphs.is_empty() || attribute.tp.is_some()
        || !attribute.type_protos.is_empty() || attribute.sparse_tensor.is_some() || !attribute.sparse_tensors.is_empty()
        || (kind != 1 && attribute.f.is_some()) || (kind != 2 && attribute.i.is_some())
        || (kind != 3 && attribute.s.is_some()) || (kind != 4 && attribute.t.is_some())
        || (kind != 6 && !attribute.floats.is_empty()) || (kind != 7 && !attribute.ints.is_empty()) {
        return Err(format!("invalid attribute payload {:?}", attribute.name));
    }
    match kind {
        1 => { let value = attribute.f.ok_or("missing FLOAT attribute payload")?; if !value.is_finite() { return Err("nonfinite FLOAT attribute".into()); } Ok(json!(value)) },
        2 => Ok(json!(attribute.i.ok_or("missing INT attribute payload")?)),
        3 => Ok(json!(std::str::from_utf8(attribute.s.as_deref().ok_or("missing STRING attribute payload")?).map_err(|_| "attribute string is not UTF-8")?)),
        4 => tensor_descriptor(attribute.t.as_ref().ok_or("missing TENSOR attribute payload")?, blob),
        6 => { if attribute.floats.iter().any(|value| !value.is_finite()) { return Err("nonfinite FLOATS attribute".into()); } Ok(json!(attribute.floats)) },
        7 => Ok(json!(attribute.ints)),
        _ => Err(format!("unsupported attribute type {kind}")),
    }
}

fn validate_node_arity(node: &proto::Node) -> Result<(), String> {
    let (minimum, maximum, outputs) = match node.op_type.as_str() {
        "Constant" => (0, 0, 1),
        "Conv" => (2, 3, 1), "BatchNormalization" => (5, 5, 1),
        "InstanceNormalization" => (3, 3, 1), "Resize" => (1, 4, 1),
        "Slice" => (3, 5, 1), "Clip" => (1, 3, 1), "Pad" => (2, 3, 1),
        "Unsqueeze" | "Squeeze" | "Split" => (1, 2, if node.op_type == "Split" { usize::MAX } else { 1 }),
        "Concat" => (1, usize::MAX, 1),
        "Add" | "Sub" | "Mul" | "Div" | "PRelu" | "Gather" | "Reshape" | "Expand" | "MatMul" => (2, 2, 1),
        _ => (1, 1, 1),
    };
    if node.input.len() < minimum || node.input.len() > maximum || node.output.is_empty() || (outputs != usize::MAX && node.output.len() != outputs)
        || node.input.iter().take(minimum).enumerate().any(|(axis, edge)| edge.is_empty() && !(node.op_type == "Resize" && axis > 0)) {
        return Err(format!("malformed {} node arity", node.op_type));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tensor(dtype: i32, dims: Vec<i64>, raw: Vec<u8>) -> proto::Tensor {
        proto::Tensor {
            data_type: Some(dtype),
            dims,
            raw_data: Some(raw),
            ..Default::default()
        }
    }

    #[test]
    fn validates_decoded_protobuf_tensor_payloads() {
        let source = tensor(1, vec![2], 1f32.to_le_bytes().to_vec());
        let decoded = proto::Tensor::decode(source.encode_to_vec().as_slice()).unwrap();
        assert!(tensor_descriptor(&decoded, &mut Vec::new()).is_err());
        let source = tensor(1, vec![1], f32::NAN.to_le_bytes().to_vec());
        let decoded = proto::Tensor::decode(source.encode_to_vec().as_slice()).unwrap();
        assert!(tensor_descriptor(&decoded, &mut Vec::new()).is_err());
    }

    #[test]
    fn packs_canonical_typed_integer_payloads() {
        let source = proto::Tensor {
            data_type: Some(7),
            dims: vec![2],
            int64_data: vec![-1, 4],
            ..Default::default()
        };
        let mut blob = Vec::new();
        let descriptor = tensor_descriptor(&source, &mut blob).unwrap();
        assert_eq!(descriptor["length"], 16);
        assert_eq!(blob, [(-1i64).to_le_bytes(), 4i64.to_le_bytes()].concat());
    }

    #[test]
    fn rejects_mixed_and_overflowing_payloads() {
        let mut source = tensor(1, vec![1], 1f32.to_le_bytes().to_vec());
        source.int64_data.push(1);
        assert!(tensor_descriptor(&source, &mut Vec::new()).is_err());
        assert!(tensor_descriptor(
            &tensor(7, vec![i64::MAX, i64::MAX], vec![]),
            &mut Vec::new()
        )
        .is_err());
        let attribute = proto::Attribute {
            name: "axis".into(),
            r#type: Some(2),
            i: Some(1),
            f: Some(1.0),
            ..Default::default()
        };
        let decoded = proto::Attribute::decode(attribute.encode_to_vec().as_slice()).unwrap();
        assert!(attribute_value(&decoded, &mut Vec::new()).is_err());
    }
    #[test]
    fn validates_interface_dimensions_and_bounded_storage() {
        let value = ArtifactValue {
            name: "input".into(),
            value_type: Some(crate::artifact::ArtifactType {
                denotation: None,
                kind: ArtifactValueKind::Tensor {
                    element_type: Some(1),
                    shape: ArtifactShape::KnownRank(vec![
                        crate::artifact::ArtifactDimension {
                            value: ArtifactDimensionValue::Static(1),
                            denotation: None,
                        },
                        crate::artifact::ArtifactDimension {
                            value: ArtifactDimensionValue::Symbol("width".into()),
                            denotation: None,
                        },
                    ]),
                },
            }),
            doc_string: None,
            metadata: Default::default(),
        };
        assert!(validate_interface(&value, 1, &[1, 640], |axis| axis == 1).is_ok());
        assert!(validate_interface(&value, 1, &[2, 640], |axis| axis == 1).is_err());
        assert!(validate_interface(&value, 6, &[1, 640], |_| true).is_err());
        assert!(checked_storage(&[1, 3, 640, 640], 4).is_ok());
        assert!(checked_storage(&[MAX_DIMENSION + 1], 1).is_err());
        assert!(checked_storage(&[1; 9], 1).is_err());
    }

    #[test]
    fn rejects_invalid_tensor_shapes_storage_and_values() {
        assert!(tensor_shape(&tensor(1, vec![-1], 0f32.to_le_bytes().to_vec())).is_err());
        assert!(tensor_descriptor(&tensor(3, vec![1], vec![0]), &mut Vec::new()).is_err());
        assert!(tensor_descriptor(&tensor(1, vec![1], vec![0]), &mut Vec::new()).is_err());
        let mut tensor = proto::Tensor {
            data_type: Some(2),
            dims: vec![1],
            int32_data: vec![256],
            ..Default::default()
        };
        assert!(tensor_descriptor(&tensor, &mut Vec::new()).is_err());
        tensor.int32_data = vec![1];
        assert_eq!(
            tensor_descriptor(&tensor, &mut Vec::new()).unwrap()["length"],
            1
        );
        let mut tensor = proto::Tensor {
            data_type: Some(6),
            dims: vec![1],
            int32_data: vec![7],
            ..Default::default()
        };
        assert_eq!(
            tensor_descriptor(&tensor, &mut Vec::new()).unwrap()["length"],
            4
        );
        tensor.data_location = Some(1);
        assert!(tensor_descriptor(&tensor, &mut Vec::new()).is_err());
    }

    #[test]
    fn attribute_and_node_arity_branches_are_checked() {
        for (kind, valid) in [
            (1, true),
            (2, true),
            (3, true),
            (6, true),
            (7, true),
            (99, false),
        ] {
            let attribute = proto::Attribute {
                name: "a".into(),
                r#type: Some(kind),
                f: (kind == 1).then_some(1.0),
                i: (kind == 2).then_some(2),
                s: (kind == 3).then(|| b"ok".to_vec()),
                floats: if kind == 6 { vec![1.0] } else { vec![] },
                ints: if kind == 7 { vec![1] } else { vec![] },
                ..Default::default()
            };
            assert_eq!(attribute_value(&attribute, &mut Vec::new()).is_ok(), valid);
        }
        for (op, inputs, outputs, valid) in [
            ("Constant", 0, 1, true),
            ("Conv", 2, 1, true),
            ("Resize", 1, 1, true),
            ("Split", 1, 2, true),
            ("Concat", 0, 1, false),
            ("Add", 1, 1, false),
            ("Relu", 1, 2, false),
        ] {
            let node = proto::Node {
                op_type: op.into(),
                input: vec!["x".into(); inputs],
                output: vec!["y".into(); outputs],
                ..Default::default()
            };
            assert_eq!(validate_node_arity(&node).is_ok(), valid, "{op}");
        }
    }
}
