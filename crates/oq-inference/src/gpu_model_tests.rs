use super::*;

// Tiny in-code ONNX descriptions: zero weights are generated to the exact real
// profile dimensions rather than committing a model binary or requiring ORT.
#[derive(Clone, PartialEq, prost::Message)]
struct FixtureModel {
    #[prost(int64, tag = "1")]
    ir_version: i64,
    #[prost(message, optional, tag = "7")]
    graph: Option<FixtureGraph>,
    #[prost(message, repeated, tag = "8")]
    opset: Vec<Opset>,
    #[prost(bytes = "vec", repeated, tag = "20")]
    training_info: Vec<Vec<u8>>,
    #[prost(bytes = "vec", repeated, tag = "25")]
    functions: Vec<Vec<u8>>,
}
#[derive(Clone, PartialEq, prost::Message)]
struct Opset {
    #[prost(int64, tag = "2")]
    version: i64,
}
#[derive(Clone, PartialEq, prost::Message)]
struct FixtureGraph {
    #[prost(message, repeated, tag = "1")]
    node: Vec<proto::Node>,
    #[prost(message, repeated, tag = "5")]
    initializer: Vec<proto::Tensor>,
    #[prost(message, repeated, tag = "11")]
    input: Vec<Value>,
    #[prost(message, repeated, tag = "12")]
    output: Vec<Value>,
    #[prost(bytes = "vec", repeated, tag = "14")]
    quantization_annotation: Vec<Vec<u8>>,
    #[prost(bytes = "vec", repeated, tag = "15")]
    sparse_initializer: Vec<Vec<u8>>,
}
#[derive(Clone, PartialEq, prost::Message)]
struct Value {
    #[prost(string, tag = "1")]
    name: String,
    #[prost(message, optional, tag = "2")]
    value_type: Option<ValueType>,
}
#[derive(Clone, PartialEq, prost::Message)]
struct ValueType {
    #[prost(message, optional, tag = "1")]
    tensor: Option<TensorType>,
}
#[derive(Clone, PartialEq, prost::Message)]
struct TensorType {
    #[prost(int32, tag = "1")]
    element_type: i32,
    #[prost(message, optional, tag = "2")]
    shape: Option<Shape>,
}
#[derive(Clone, PartialEq, prost::Message)]
struct Shape {
    #[prost(message, repeated, tag = "1")]
    dims: Vec<Dimension>,
}
#[derive(Clone, PartialEq, prost::Message)]
struct Dimension {
    #[prost(int64, optional, tag = "1")]
    size: Option<i64>,
    #[prost(string, optional, tag = "2")]
    symbol: Option<String>,
}
fn value(name: &str, dimensions: &[i64]) -> Value {
    Value {
        name: name.into(),
        value_type: Some(ValueType {
            tensor: Some(TensorType {
                element_type: 1,
                shape: Some(Shape {
                    dims: dimensions
                        .iter()
                        .map(|dimension| Dimension {
                            size: if *dimension == -1 {
                                None
                            } else {
                                Some(*dimension)
                            },
                            symbol: if *dimension == -1 {
                                Some("batch_size".into())
                            } else {
                                None
                            },
                        })
                        .collect(),
                }),
            }),
        }),
    }
}
fn ints(name: &str, values: &[i64]) -> proto::Attribute {
    proto::Attribute {
        name: name.into(),
        r#type: Some(7),
        ints: values.to_vec(),
        ..Default::default()
    }
}
fn node(operation: &str, inputs: &[&str], output: &str) -> proto::Node {
    proto::Node {
        op_type: operation.into(),
        input: inputs.iter().map(|name| (*name).into()).collect(),
        output: vec![output.into()],
        ..Default::default()
    }
}
fn constant(output: &str, shape: &[i64], raw: bool) -> proto::Node {
    let mut tensor = proto::Tensor {
        dims: vec![shape.len() as i64],
        data_type: Some(7),
        ..Default::default()
    };
    if raw {
        tensor.raw_data = Some(shape.iter().flat_map(|value| value.to_le_bytes()).collect());
    } else {
        tensor.int64_data = shape.to_vec();
    }
    let mut constant = node("Constant", &[], output);
    constant.attribute.push(proto::Attribute {
        name: "value".into(),
        r#type: Some(4),
        t: Some(tensor),
        ..Default::default()
    });
    constant
}
fn fixture(raw: bool) -> FixtureModel {
    let mut graph = FixtureGraph::default();
    graph.input.push(value("input", &[-1, 1, 224, 224]));
    graph.output.push(value("output", &[-1, 1, 672, 672]));
    let profile = [(1, 64, 5), (64, 64, 3), (64, 32, 3), (32, 9, 3)];
    let mut previous = "input".to_string();
    for (index, (input, output, kernel)) in profile.into_iter().enumerate() {
        let weights = format!("conv{index}.weight");
        let bias = format!("conv{index}.bias");
        for (name, dims) in [
            (weights.clone(), vec![output, input, kernel, kernel]),
            (bias.clone(), vec![output]),
        ] {
            let count = dims.iter().product::<i64>() as usize;
            let mut tensor = proto::Tensor {
                name: name.clone(),
                dims: dims.clone(),
                data_type: Some(1),
                ..Default::default()
            };
            if raw {
                tensor.raw_data = Some(vec![0; count * 4]);
            } else {
                tensor.float_data = vec![0.0; count];
            }
            graph.initializer.push(tensor);
            graph.input.push(value(&name, &dims));
        }
        let next = format!("conv{index}");
        let mut conv = node("Conv", &[&previous, &weights, &bias], &next);
        conv.attribute = vec![
            ints("kernel_shape", &[kernel, kernel]),
            ints("pads", &[kernel / 2; 4]),
            ints("strides", &[1, 1]),
            ints("dilations", &[1, 1]),
            proto::Attribute {
                name: "group".into(),
                r#type: Some(2),
                i: Some(1),
                ..Default::default()
            },
        ];
        graph.node.push(conv);
        previous = next;
        if index < 3 {
            let next = format!("relu{index}");
            graph.node.push(node("Relu", &[&previous], &next));
            previous = next;
        }
    }
    graph
        .node
        .push(constant("shape1", &[-1, 1, 3, 3, 224, 224], raw));
    graph
        .node
        .push(node("Reshape", &[&previous, "shape1"], "reshape1"));
    let mut transpose = node("Transpose", &["reshape1"], "transpose");
    transpose.attribute.push(ints("perm", &[0, 1, 4, 2, 5, 3]));
    graph.node.push(transpose);
    graph.node.push(constant("shape2", &[-1, 1, 672, 672], raw));
    graph
        .node
        .push(node("Reshape", &["transpose", "shape2"], "output"));
    FixtureModel {
        ir_version: 4,
        graph: Some(graph),
        opset: vec![Opset { version: 10 }],
        training_info: Vec::new(),
        functions: Vec::new(),
    }
}
fn rejected(change: impl FnOnce(&mut FixtureGraph), case: &str) {
    let mut model = fixture(false);
    change(model.graph.as_mut().unwrap());
    assert!(
        GpuModel::from_onnx(&model.encode_to_vec()).is_err(),
        "invalid graph admitted: {case}"
    );
}

#[test]
fn compiles_full_profile_with_typed_and_raw_payloads() {
    for raw in [false, true] {
        let bytes = fixture(raw).encode_to_vec();
        let model = GpuModel::from_onnx(&bytes).unwrap();
        assert_eq!(
            (model.plan.input_width, model.plan.input_height),
            (224, 224)
        );
        assert_eq!(
            (
                model.plan.output_width,
                model.plan.output_height,
                model.plan.scale
            ),
            (672, 672, 3)
        );
        assert_eq!(model.plan.input_name, "input");
        assert_eq!(model.plan.output_name, "output");
        assert_eq!(
            model.plan.source_sha256,
            format!("{:x}", Sha256::digest(&bytes))
        );
        assert_eq!(model.plan.source_byte_length, bytes.len() as u64);
        assert_eq!(
            model.plan.source_interface,
            crate::inspect_onnx(&bytes).unwrap()
        );
        assert_eq!(model.plan.source_interface.inputs.len(), 9);
        assert_eq!(model.layers.len(), 4);
        assert_eq!(model.layers[0].weights.len(), 64 * 25);
        assert_eq!(model.layers[1].weights.len(), 64 * 64 * 9);
        assert_eq!(model.layers[2].weights.len(), 32 * 64 * 9);
        assert_eq!(model.layers[3].weights.len(), 9 * 32 * 9);
        assert_eq!(
            model
                .layers
                .iter()
                .map(|layer| layer.relu)
                .collect::<Vec<_>>(),
            [true, true, true, false]
        );
        assert_eq!(
            model
                .layers
                .iter()
                .map(|layer| (
                    layer.in_channels,
                    layer.out_channels,
                    layer.kernel,
                    layer.stride,
                    layer.pads,
                    layer.relu,
                ))
                .collect::<Vec<_>>(),
            vec![
                (1, 64, [5, 5], [1, 1], [2, 2, 2, 2], true),
                (64, 64, [3, 3], [1, 1], [1, 1, 1, 1], true),
                (64, 32, [3, 3], [1, 1], [1, 1, 1, 1], true),
                (32, 9, [3, 3], [1, 1], [1, 1, 1, 1], false),
            ]
        );
        assert!(model
            .layers
            .iter()
            .all(|layer| layer.weights.iter().all(|value| *value == 0.0)
                && layer.bias.iter().all(|value| *value == 0.0)));
    }
}

#[test]
fn validates_weight_payload_lengths_shapes_and_finiteness() {
    rejected(
        |graph| {
            graph.initializer[0].float_data.pop();
        },
        "payload length",
    );
    rejected(
        |graph| {
            graph.initializer[0].float_data[0] = f32::NAN;
        },
        "non-finite",
    );
    rejected(
        |graph| {
            graph.initializer[1].float_data[0] = f32::INFINITY;
        },
        "non-finite",
    );
    rejected(
        |graph| {
            graph.initializer[0].dims[0] = 63;
        },
        "disagrees",
    );
    rejected(
        |graph| {
            graph.initializer[0].data_type = Some(10);
        },
        "disagrees",
    );
    rejected(
        |graph| {
            graph.initializer[0].raw_data = Some(vec![0; 4]);
        },
        "conflicting",
    );
    rejected(
        |graph| {
            graph.initializer[0].data_location = Some(1);
        },
        "embedded",
    );
    rejected(
        |graph| {
            graph.initializer[0].segment = Some(Vec::new());
        },
        "complete",
    );
    let mut model = fixture(true);
    model.graph.as_mut().unwrap().initializer[0]
        .raw_data
        .as_mut()
        .unwrap()[..4]
        .copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(GpuModel::from_onnx(&model.encode_to_vec())
        .unwrap_err()
        .contains("non-finite"));
}

#[test]
fn validates_convolution_attributes_without_ignoring_any() {
    for (name, values) in [
        ("strides", vec![2, 2]),
        ("dilations", vec![2, 2]),
        ("kernel_shape", vec![3, 3]),
        ("pads", vec![0; 4]),
    ] {
        rejected(
            |graph| {
                let attr = graph.node[0]
                    .attribute
                    .iter_mut()
                    .find(|attr| attr.name == name)
                    .unwrap();
                attr.ints = values;
            },
            if name == "pads" { "pads" } else { name },
        );
    }
    rejected(
        |graph| {
            graph.node[0]
                .attribute
                .iter_mut()
                .find(|attr| attr.name == "group")
                .unwrap()
                .i = Some(2);
        },
        "group",
    );
    rejected(
        |graph| {
            graph.node[0].attribute.push(ints("unknown", &[1]));
        },
        "unsupported Conv attribute",
    );
    rejected(
        |graph| {
            graph.node[0].attribute.push(ints("strides", &[1, 1]));
        },
        "duplicate",
    );
    rejected(
        |graph| {
            graph.node[0].attribute[0].r#type = Some(2);
        },
        "invalid attribute payload",
    );
    rejected(
        |graph| {
            graph.node[0].attribute[0].i = Some(5);
        },
        "invalid attribute payload",
    );
    rejected(
        |graph| {
            graph.node[1].attribute.push(ints("axis", &[1]));
        },
        "Relu",
    );
    rejected(
        |graph| {
            graph.node[0].attribute.retain(|attr| attr.name != "pads");
        },
        "padding explicitly",
    );
    rejected(
        |graph| {
            graph.node[0].attribute.push(proto::Attribute {
                name: "auto_pad".into(),
                r#type: Some(3),
                s: Some(b"SAME_UPPER".to_vec()),
                ..Default::default()
            });
        },
        "auto_pad",
    );
    rejected(
        |graph| {
            graph.node[0].output.clear();
        },
        "requires one named output",
    );
    rejected(
        |graph| {
            graph.node[0].output.push("extra".into());
        },
        "requires one named output",
    );
    rejected(
        |graph| {
            let pads = graph.node[0]
                .attribute
                .iter_mut()
                .find(|attr| attr.name == "pads")
                .unwrap();
            pads.r#type = Some(2);
            pads.i = Some(1);
            pads.ints.clear();
        },
        "wrong ONNX type",
    );
    rejected(
        |graph| {
            graph.node[0].attribute.push(proto::Attribute {
                name: "auto_pad".into(),
                r#type: Some(2),
                i: Some(1),
                ..Default::default()
            });
        },
        "wrong ONNX type",
    );
}

#[test]
fn rejects_unsupported_topology_branches_and_disconnected_edges() {
    rejected(
        |graph| {
            graph.node[1].op_type = "Sigmoid".into();
        },
        "unsupported resident topology",
    );
    rejected(
        |graph| {
            graph.node[0].domain = "custom".into();
        },
        "custom-domain",
    );
    rejected(
        |graph| {
            graph.node.push(node("Identity", &["conv0"], "branch"));
        },
        "branched or unused",
    );
    rejected(
        |graph| {
            graph.node[2].input[0] = "conv0".into();
        },
        "branched or unused",
    );
    rejected(
        |graph| {
            graph.node[2].input[0] = "missing".into();
        },
        "unavailable edge",
    );
    rejected(
        |graph| {
            graph.node[2].input[0] = "conv2".into();
        },
        "unavailable edge",
    );
    rejected(
        |graph| {
            graph.node[2].output[0] = "conv0".into();
        },
        "duplicate produced edge",
    );
    rejected(
        |graph| {
            graph.node.push(constant("unused", &[1], false));
        },
        "unused edge",
    );
    rejected(
        |graph| {
            graph.output[0].name = "missing".into();
        },
        "no producer",
    );
    rejected(
        |graph| {
            graph.node[0].input.swap(1, 2);
        },
        "dimensions",
    );
}

#[test]
fn rejects_model_level_features_and_ambiguous_initializer_storage() {
    let mut model = fixture(false);
    model.training_info.push(vec![0x0a, 0x00]);
    assert!(GpuModel::from_onnx(&model.encode_to_vec())
        .unwrap_err()
        .contains("training graphs or local functions"));
    let mut model = fixture(false);
    model.functions.push(vec![0x0a, 0x00]);
    assert!(GpuModel::from_onnx(&model.encode_to_vec())
        .unwrap_err()
        .contains("training graphs or local functions"));
    let mut model = fixture(false);
    model.ir_version = 11;
    assert!(GpuModel::from_onnx(&model.encode_to_vec())
        .unwrap_err()
        .contains("IR version"));
    rejected(
        |graph| {
            graph.initializer.push(graph.initializer[0].clone());
        },
        "duplicate or empty initializer",
    );
    rejected(
        |graph| {
            graph.initializer[0].name.clear();
        },
        "duplicate or empty initializer",
    );
    rejected(
        |graph| {
            graph.sparse_initializer.push(vec![0x0a, 0x00]);
        },
        "dense unquantized",
    );
    rejected(
        |graph| {
            graph.quantization_annotation.push(vec![0x0a, 0x00]);
        },
        "dense unquantized",
    );
}

#[test]
fn rejects_missing_tensors_and_malformed_constant_and_reshape_data() {
    rejected(
        |graph| {
            graph.node[0].input[1] = "input".into();
        },
        "Conv weights must be embedded initializers",
    );
    rejected(
        |graph| {
            graph.node[8].input[1] = "input".into();
        },
        "branched or unused",
    );
    rejected(
        |graph| {
            graph.node[7].attribute[0].t.as_mut().unwrap().dims = vec![1];
        },
        "shape tensor must be a vector",
    );
    rejected(
        |graph| {
            graph.node[7].attribute[0].t.as_mut().unwrap().dims.clear();
        },
        "positive and non-scalar",
    );
    rejected(
        |graph| {
            graph.node[7].attribute[0].t.as_mut().unwrap().data_type = Some(1);
        },
        "only INT64 reshape constants",
    );
    rejected(
        |graph| {
            graph.node[7].attribute[0].t.as_mut().unwrap().raw_data = Some(vec![0; 48]);
        },
        "raw payload length or conflicting",
    );
    rejected(
        |graph| {
            graph.node[7].attribute[0].r#type = Some(2);
        },
        "invalid attribute payload",
    );
    rejected(
        |graph| {
            graph.node[7].attribute.clear();
        },
        "Constant must contain only its tensor value attribute",
    );
    rejected(
        |graph| {
            graph.node[7].input.push("input".into());
        },
        "Constant must have no inputs",
    );
}
#[test]
fn rejects_missing_graph_and_initializer_interfaces_and_invalid_tensor_storage() {
    let mut model = fixture(false);
    model.graph = None;
    assert!(GpuModel::from_onnx(&model.encode_to_vec())
        .unwrap_err()
        .contains("no graph"));
    rejected(
        |graph| {
            graph.node[0].input[2] = "input".into();
        },
        "Conv bias must be an embedded initializer",
    );
    rejected(
        |graph| {
            graph.node[7].attribute = vec![ints("not_value", &[1])];
        },
        "Constant requires value",
    );
    rejected(
        |graph| {
            graph.node[7].attribute[0] = proto::Attribute {
                name: "value".into(),
                r#type: Some(2),
                i: Some(1),
                ..Default::default()
            };
        },
        "wrong ONNX type",
    );
    rejected(
        |graph| {
            graph.input[1].value_type = None;
        },
        "initializer input has no type",
    );
    rejected(
        |graph| {
            graph.input[1]
                .value_type
                .as_mut()
                .unwrap()
                .tensor
                .as_mut()
                .unwrap()
                .shape = None;
        },
        "initializer input must declare a tensor shape",
    );
    rejected(
        |graph| {
            graph.initializer[0].int32_data.push(0);
        },
        "unsupported tensor payload encoding",
    );
    rejected(
        |graph| {
            graph.initializer[0].data_type = Some(10);
            graph.input[1]
                .value_type
                .as_mut()
                .unwrap()
                .tensor
                .as_mut()
                .unwrap()
                .element_type = 10;
        },
        "weights and biases must be FP32",
    );
    rejected(
        |graph| {
            graph.input[0].value_type = None;
        },
        "image has no tensor type",
    );
}

#[test]
fn rejects_interface_batch_and_tensor_contract_mismatches() {
    rejected(
        |graph| {
            graph.output[0] = value("output", &[1, 1, 672, 672]);
        },
        "batch declarations differ",
    );
    rejected(
        |graph| {
            graph.input[0] = value("input", &[1, 1, 224, 224]);
        },
        "batch declarations differ",
    );
    rejected(
        |graph| {
            graph.input[0]
                .value_type
                .as_mut()
                .unwrap()
                .tensor
                .as_mut()
                .unwrap()
                .element_type = 10;
        },
        "shaped FP32",
    );
    rejected(
        |graph| {
            graph.initializer[0].float_data[0] = f32::NEG_INFINITY;
        },
        "non-finite",
    );
}

#[test]
fn validates_exact_pixelshuffle_mapping_and_shape_payloads() {
    rejected(
        |graph| {
            graph.node[9].attribute[0].ints = vec![0, 1, 4, 3, 5, 2];
        },
        "pixel shuffle",
    );
    rejected(
        |graph| {
            graph.node[9].attribute.clear();
        },
        "exactly perm",
    );
    rejected(
        |graph| {
            graph.node[7].attribute[0].t.as_mut().unwrap().int64_data[2] = 1;
        },
        "pixelshuffle3",
    );
    rejected(
        |graph| {
            graph.node[10].attribute[0].t.as_mut().unwrap().int64_data[2] = 671;
        },
        "pixelshuffle3",
    );
    rejected(
        |graph| {
            graph.node[7].attribute[0]
                .t
                .as_mut()
                .unwrap()
                .int64_data
                .pop();
        },
        "payload length",
    );
    rejected(
        |graph| {
            graph.node[7].attribute[0].t.as_mut().unwrap().data_type = Some(1);
        },
        "INT64",
    );
    rejected(
        |graph| {
            graph.node[8].attribute.push(ints("allowzero", &[1]));
        },
        "without attributes",
    );
    rejected(
        |graph| {
            graph.node[9].input[0] = "conv3".into();
        },
        "branched or unused",
    );
    rejected(
        |graph| {
            graph.node[9].attribute[0] = proto::Attribute {
                name: "perm".into(),
                r#type: Some(2),
                i: Some(1),
                ..Default::default()
            };
        },
        "wrong ONNX type",
    );
    for batch in [-1, 0, 1] {
        let mut fixture = fixture(false);
        let graph = fixture.graph.as_mut().unwrap();
        for index in [7, 10] {
            graph.node[index].attribute[0]
                .t
                .as_mut()
                .unwrap()
                .int64_data[0] = batch;
        }
        assert!(GpuModel::from_onnx(&fixture.encode_to_vec()).is_ok());
    }
}

#[test]
fn preserves_named_interface_and_rejects_wrong_profile() {
    rejected(
        |graph| {
            graph.input[0] = value("input", &[-1, 3, 224, 224]);
        },
        "NCHW",
    );
    rejected(
        |graph| {
            graph.input[0] = value("input", &[-1, 1, 112, 112]);
        },
        "NCHW",
    );
    rejected(
        |graph| {
            graph.output[0] = value("output", &[-1, 1, 448, 448]);
        },
        "NCHW",
    );
    rejected(
        |graph| {
            graph.output[0] = value("output", &[2, 1, 672, 672]);
        },
        "batch",
    );
    rejected(
        |graph| {
            graph.input.push(value("extra", &[1, 1, 224, 224]));
        },
        "one required image input",
    );
    rejected(
        |graph| {
            graph.input[0].name.clear();
        },
        "unavailable edge",
    );
    rejected(
        |graph| {
            graph.output[0].name.clear();
        },
        "declared output has no producer",
    );
    let mut fixture = fixture(false);
    fixture.opset[0].version = 11;
    assert!(GpuModel::from_onnx(&fixture.encode_to_vec())
        .unwrap_err()
        .contains("opset 10"));
    assert!(GpuModel::from_onnx(b"not an ONNX model").is_err());
}

#[test]
#[ignore = "optional cached-model check; set OQ_SUPER_RESOLUTION_MODEL to the ONNX path"]
fn compiles_selected_cached_model() {
    let path = std::env::var_os("OQ_SUPER_RESOLUTION_MODEL")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(
                std::env::var_os("HOME").expect("HOME or OQ_SUPER_RESOLUTION_MODEL"),
            )
            .join("Library/Application Support/com.openquartz.app/models/super-resolution-3x.onnx")
        });
    let bytes = std::fs::read(path).expect("read selected super-resolution model");
    let model = GpuModel::from_onnx(&bytes).expect("compile actual ONNX-10 profile");
    assert_eq!(
        model.plan.source_sha256,
        "85f36ff88cc504a24af5e0602148bc56a8aa09a58eca8c0da2756f3e8186035e"
    );
    assert_eq!(
        model
            .layers
            .iter()
            .map(|layer| (layer.in_channels, layer.out_channels))
            .collect::<Vec<_>>(),
        [(1, 64), (64, 64), (64, 32), (32, 9)]
    );
}
