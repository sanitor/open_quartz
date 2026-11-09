use super::proto::type_proto::Value;
use super::*;

fn tensor(element_type: i32, shape: Option<Vec<proto::Dimension>>) -> proto::TypeProto {
    proto::TypeProto {
        value: Some(Value::Tensor(proto::TensorTypeProto {
            elem_type: Some(element_type),
            shape: shape.map(|dim| proto::TensorShapeProto { dim }),
        })),
        denotation: None,
    }
}

fn port(name: &str, value_type: proto::TypeProto) -> proto::ValueInfoProto {
    proto::ValueInfoProto {
        name: Some(name.into()),
        r#type: Some(value_type),
        ..Default::default()
    }
}

fn entry(key: &str, value: &str) -> proto::StringStringEntryProto {
    proto::StringStringEntryProto {
        key: Some(key.into()),
        value: Some(value.into()),
    }
}

fn inspect_graph(graph: proto::GraphProto) -> Result<ArtifactInterface, ArtifactError> {
    inspect_onnx(
        &proto::ModelProto {
            graph: Some(graph),
            ..Default::default()
        }
        .encode_to_vec(),
    )
}

fn kind(value: &ArtifactValue) -> &ArtifactValueKind {
    &value.value_type.as_ref().unwrap().kind
}

#[test]
fn independent_wire_fixture_distinguishes_scalar_from_unknown_rank() {
    // Model.graph(7) -> Graph.input(11) -> ValueInfo(name=1,type=2)
    // -> Type.tensor(1) -> Tensor(elem_type=1,shape=2).
    // This is hand-authored wire data, independent of the selected schema encoder.
    let scalar = [
        0x3a, 0x0b, 0x5a, 0x09, 0x0a, 0x01, b'x', 0x12, 0x04, 0x0a, 0x02, 0x12, 0x00,
    ];
    let unknown_rank = [
        0x3a, 0x09, 0x5a, 0x07, 0x0a, 0x01, b'x', 0x12, 0x02, 0x0a, 0x00,
    ];
    let scalar = inspect_onnx(&scalar).unwrap();
    let unknown_rank = inspect_onnx(&unknown_rank).unwrap();
    assert_eq!(
        kind(&scalar.inputs[0].value),
        &ArtifactValueKind::Tensor {
            element_type: None,
            shape: ArtifactShape::KnownRank(vec![]),
        }
    );
    assert_eq!(
        kind(&unknown_rank.inputs[0].value),
        &ArtifactValueKind::Tensor {
            element_type: None,
            shape: ArtifactShape::UnknownRank,
        }
    );
    // Serialization must not erase the distinction either.
    assert_ne!(
        serde_json::to_value(scalar).unwrap(),
        serde_json::to_value(unknown_rank).unwrap()
    );
}

#[test]
fn preserves_symbols_anonymous_axes_denotations_and_source_integer_values() {
    let dimensions = vec![
        proto::Dimension {
            value: Some(proto::dimension::Value::DimParam("batch".into())),
            denotation: Some("DATA_BATCH".into()),
        },
        proto::Dimension {
            value: Some(proto::dimension::Value::DimParam("width".into())),
            denotation: None,
        },
        proto::Dimension {
            value: Some(proto::dimension::Value::DimParam("batch".into())),
            denotation: None,
        },
        proto::Dimension {
            value: Some(proto::dimension::Value::DimParam(String::new())),
            denotation: None,
        },
        proto::Dimension {
            value: None,
            denotation: Some("DATA_CHANNEL".into()),
        },
        proto::Dimension {
            value: Some(proto::dimension::Value::DimValue(0)),
            denotation: None,
        },
        proto::Dimension {
            value: Some(proto::dimension::Value::DimValue(-1)),
            denotation: None,
        },
    ];
    let mut value_type = tensor(1, Some(dimensions));
    value_type.denotation = Some("IMAGE".into());
    let mut input = port("image", value_type);
    input.doc_string = Some("Original port documentation".into());
    input.metadata_props = vec![entry("layout", "NCHW")];
    let interface = inspect_graph(proto::GraphProto {
        input: vec![input],
        ..Default::default()
    })
    .unwrap();
    let input = &interface.inputs[0].value;
    assert_eq!(
        input.doc_string.as_deref(),
        Some("Original port documentation")
    );
    assert_eq!(input.metadata["layout"], "NCHW");
    assert_eq!(
        input.value_type.as_ref().unwrap().denotation.as_deref(),
        Some("IMAGE")
    );
    let ArtifactValueKind::Tensor {
        shape: ArtifactShape::KnownRank(dimensions),
        ..
    } = kind(input)
    else {
        panic!("expected declared-rank tensor");
    };
    assert_eq!(
        dimensions.iter().map(|dim| &dim.value).collect::<Vec<_>>(),
        vec![
            &ArtifactDimensionValue::Symbol("batch".into()),
            &ArtifactDimensionValue::Symbol("width".into()),
            &ArtifactDimensionValue::Symbol("batch".into()),
            &ArtifactDimensionValue::Symbol(String::new()),
            &ArtifactDimensionValue::Dynamic,
            &ArtifactDimensionValue::Static(0),
            &ArtifactDimensionValue::Static(-1),
        ]
    );
    assert_eq!(dimensions[0].denotation.as_deref(), Some("DATA_BATCH"));
    assert_eq!(dimensions[4].denotation.as_deref(), Some("DATA_CHANNEL"));
}

#[test]
fn recursively_preserves_optional_sequence_map_and_sparse_types() {
    let sparse = proto::TypeProto {
        value: Some(Value::SparseTensor(proto::TensorTypeProto {
            elem_type: Some(22),
            shape: Some(proto::TensorShapeProto::default()),
        })),
        denotation: Some("sparse-value".into()),
    };
    let map = proto::TypeProto {
        value: Some(Value::Map(proto::MapTypeProto {
            key_type: Some(8),
            value_type: Some(Box::new(sparse)),
        })),
        denotation: None,
    };
    let sequence = proto::TypeProto {
        value: Some(Value::Sequence(proto::SequenceTypeProto {
            elem_type: Some(Box::new(map)),
        })),
        denotation: None,
    };
    let optional = proto::TypeProto {
        value: Some(Value::Optional(proto::SequenceTypeProto {
            elem_type: Some(Box::new(sequence)),
        })),
        denotation: Some("outer".into()),
    };
    let interface = inspect_graph(proto::GraphProto {
        input: vec![port("nested", optional)],
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        interface.inputs[0].requirement,
        ArtifactInputRequirement::Optional
    );
    let ArtifactValueKind::Optional {
        element: Some(sequence),
    } = kind(&interface.inputs[0].value)
    else {
        panic!("expected optional");
    };
    let ArtifactValueKind::Sequence { element: Some(map) } = &sequence.kind else {
        panic!("expected sequence");
    };
    let ArtifactValueKind::Map {
        key_type,
        value: Some(sparse),
    } = &map.kind
    else {
        panic!("expected map");
    };
    assert_eq!(*key_type, Some(8));
    assert_eq!(sparse.denotation.as_deref(), Some("sparse-value"));
    assert_eq!(
        sparse.kind,
        ArtifactValueKind::SparseTensor {
            element_type: Some(22),
            shape: ArtifactShape::KnownRank(vec![]),
        }
    );
}

#[test]
fn preserves_every_current_and_unknown_dtype_code_without_runtime_mapping() {
    let codes: Vec<i32> = (0..=28).chain([123456, -7]).collect();
    let interface = inspect_graph(proto::GraphProto {
        output: codes
            .iter()
            .map(|&code| port(&format!("type_{code}"), tensor(code, None)))
            .collect(),
        ..Default::default()
    })
    .unwrap();
    for (output, code) in interface.outputs.iter().zip(codes) {
        assert_eq!(
            kind(output),
            &ArtifactValueKind::Tensor {
                element_type: Some(code),
                shape: ArtifactShape::UnknownRank,
            }
        );
    }
}

#[test]
fn preserves_model_graph_metadata_versions_and_all_opset_domains() {
    let model = proto::ModelProto {
        ir_version: Some(13),
        producer_name: Some("fixture-exporter".into()),
        producer_version: Some("3.2.1".into()),
        domain: Some("org.example.model".into()),
        model_version: Some(42),
        doc_string: Some("Model **description**".into()),
        graph: Some(proto::GraphProto {
            name: Some("named-graph".into()),
            doc_string: Some("Graph description".into()),
            metadata_props: vec![entry("graph-key", "graph-value")],
            ..Default::default()
        }),
        opset_import: vec![
            proto::OperatorSetIdProto {
                domain: None,
                version: Some(23),
            },
            proto::OperatorSetIdProto {
                domain: Some("com.example.custom".into()),
                version: Some(7),
            },
            proto::OperatorSetIdProto {
                domain: Some("ai.onnx.ml".into()),
                version: Some(5),
            },
        ],
        metadata_props: vec![entry("arbitrary-key", "literal custom value")],
    };
    let interface = inspect_onnx(&model.encode_to_vec()).unwrap();
    assert_eq!(interface.ir_version, Some(13));
    assert_eq!(interface.producer_name.as_deref(), Some("fixture-exporter"));
    assert_eq!(interface.producer_version.as_deref(), Some("3.2.1"));
    assert_eq!(interface.domain.as_deref(), Some("org.example.model"));
    assert_eq!(interface.model_version, Some(42));
    assert_eq!(
        interface.doc_string.as_deref(),
        Some("Model **description**")
    );
    assert_eq!(interface.graph_name.as_deref(), Some("named-graph"));
    assert_eq!(
        interface.graph_doc_string.as_deref(),
        Some("Graph description")
    );
    assert_eq!(interface.metadata["arbitrary-key"], "literal custom value");
    assert_eq!(interface.graph_metadata["graph-key"], "graph-value");
    assert_eq!(
        interface.opset_imports,
        vec![
            ArtifactOpsetImport {
                domain: None,
                version: Some(23)
            },
            ArtifactOpsetImport {
                domain: Some("com.example.custom".into()),
                version: Some(7)
            },
            ArtifactOpsetImport {
                domain: Some("ai.onnx.ml".into()),
                version: Some(5)
            },
        ]
    );
    let encoded = serde_json::to_vec(&interface).unwrap();
    assert_eq!(
        serde_json::from_slice::<ArtifactInterface>(&encoded).unwrap(),
        interface
    );
}

#[test]
fn separates_required_inputs_from_dense_and_sparse_initializer_defaults() {
    let dense = proto::TensorProto {
        name: Some("weight".into()),
        data_type: Some(24),
        dims: vec![2, 3],
        data_location: Some(1),
        external_data: vec![
            entry("location", "weights.bin"),
            entry("offset", "4096"),
            entry("length", "6"),
            entry("future-key", "opaque"),
        ],
        doc_string: Some("Weight documentation".into()),
        metadata_props: vec![entry("quantization", "source fact")],
    };
    let sparse = proto::SparseTensorProto {
        dims: vec![10, 20],
        values: Some(proto::TensorProto {
            name: Some("sparse".into()),
            data_type: Some(1),
            dims: vec![3],
            data_location: Some(1),
            external_data: vec![entry("location", "sparse-values.bin")],
            ..Default::default()
        }),
        indices: Some(proto::TensorProto {
            data_type: Some(7),
            dims: vec![3, 2],
            external_data: vec![entry("location", "sparse-indices.bin")],
            ..Default::default()
        }),
    };
    let interface = inspect_graph(proto::GraphProto {
        input: vec![
            port("data", tensor(1, None)),
            port("weight", tensor(24, None)),
            port("sparse", tensor(1, None)),
        ],
        initializer: vec![dense],
        sparse_initializer: vec![sparse],
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        interface
            .inputs
            .iter()
            .map(|input| input.requirement)
            .collect::<Vec<_>>(),
        vec![
            ArtifactInputRequirement::Required,
            ArtifactInputRequirement::OverridableInitializer,
            ArtifactInputRequirement::OverridableInitializer,
        ]
    );
    let ArtifactInitializer::Dense(weight) = &interface.initializers[0] else {
        panic!("expected dense initializer")
    };
    assert_eq!(weight.dimensions, vec![2, 3]);
    assert_eq!(weight.element_type, Some(24));
    assert_eq!(weight.data_location, Some(1));
    assert_eq!(weight.external_data["location"], "weights.bin");
    assert_eq!(weight.external_data["offset"], "4096");
    assert_eq!(weight.external_data["length"], "6");
    assert_eq!(weight.external_data["future-key"], "opaque");
    assert_eq!(weight.doc_string.as_deref(), Some("Weight documentation"));
    assert_eq!(weight.metadata["quantization"], "source fact");
    let ArtifactInitializer::Sparse {
        dimensions,
        values,
        indices,
    } = &interface.initializers[1]
    else {
        panic!("expected sparse initializer")
    };
    assert_eq!(dimensions, &vec![10, 20]);
    assert_eq!(
        values.as_ref().unwrap().external_data["location"],
        "sparse-values.bin"
    );
    assert_eq!(
        indices.as_ref().unwrap().external_data["location"],
        "sparse-indices.bin"
    );
    assert_eq!(interface.initializers[1].name(), Some("sparse"));
}

#[test]
fn payload_fields_are_skipped_and_packed_dimensions_are_accepted() {
    // An independent fixture schema includes the intentionally omitted payload.
    #[derive(Clone, PartialEq, prost::Message)]
    struct PayloadTensor {
        #[prost(int64, repeated, packed = "true", tag = "1")]
        dims: Vec<i64>,
        #[prost(int32, optional, tag = "2")]
        data_type: Option<i32>,
        #[prost(bytes = "vec", tag = "9")]
        raw_data: Vec<u8>,
        #[prost(string, tag = "8")]
        name: String,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    struct PayloadGraph {
        #[prost(message, repeated, tag = "5")]
        initializers: Vec<PayloadTensor>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    struct PayloadModel {
        #[prost(message, optional, tag = "7")]
        graph: Option<PayloadGraph>,
        #[prost(bytes = "vec", tag = "100")]
        unknown_future_field: Vec<u8>,
    }
    let bytes = PayloadModel {
        graph: Some(PayloadGraph {
            initializers: vec![PayloadTensor {
                dims: vec![2, 2],
                data_type: Some(2),
                raw_data: vec![1, 2, 3, 4],
                name: "embedded".into(),
            }],
        }),
        unknown_future_field: vec![0xff, 0x00, 0x0f],
    }
    .encode_to_vec();
    let interface = inspect_onnx(&bytes).unwrap();
    let ArtifactInitializer::Dense(tensor) = &interface.initializers[0] else {
        panic!("expected dense")
    };
    assert_eq!(tensor.dimensions, vec![2, 2]);
    assert_eq!(tensor.name.as_deref(), Some("embedded"));
    assert!(tensor.external_data.is_empty());
    assert!(!serde_json::to_string(&interface)
        .unwrap()
        .contains("rawData"));
}

#[test]
fn rejects_duplicate_inputs_and_outputs_but_allows_shared_input_output_name() {
    for output in [false, true] {
        let ports = vec![port("same", tensor(1, None)), port("same", tensor(1, None))];
        let graph = if output {
            proto::GraphProto {
                output: ports,
                ..Default::default()
            }
        } else {
            proto::GraphProto {
                input: ports,
                ..Default::default()
            }
        };
        assert!(
            matches!(inspect_graph(graph), Err(ArtifactError::DuplicateName { scope, name })
            if scope == if output { "output" } else { "input" } && name == "same")
        );
    }
    assert!(inspect_graph(proto::GraphProto {
        input: vec![port("same", tensor(1, None))],
        output: vec![port("same", tensor(1, None))],
        ..Default::default()
    })
    .is_ok());
}

#[test]
fn rejects_duplicate_initializer_names_across_dense_and_sparse_lists() {
    assert!(matches!(inspect_graph(proto::GraphProto {
        initializer: vec![proto::TensorProto { name: Some("same".into()), ..Default::default() }],
        sparse_initializer: vec![proto::SparseTensorProto {
            values: Some(proto::TensorProto { name: Some("same".into()), ..Default::default() }),
            ..Default::default()
        }],
        ..Default::default()
    }), Err(ArtifactError::DuplicateName { scope: "initializer", name }) if name == "same"));
}

#[test]
fn rejects_duplicate_metadata_at_every_inspected_scope() {
    let duplicates = || vec![entry("same", "first"), entry("same", "second")];
    let mut model = proto::ModelProto {
        graph: Some(proto::GraphProto::default()),
        ..Default::default()
    };
    model.metadata_props = duplicates();
    assert!(matches!(
        inspect_onnx(&model.encode_to_vec()),
        Err(ArtifactError::DuplicateMetadata { .. })
    ));
    let graphs = [
        proto::GraphProto {
            metadata_props: duplicates(),
            ..Default::default()
        },
        proto::GraphProto {
            input: vec![proto::ValueInfoProto {
                name: Some("x".into()),
                metadata_props: duplicates(),
                ..Default::default()
            }],
            ..Default::default()
        },
        proto::GraphProto {
            output: vec![proto::ValueInfoProto {
                name: Some("x".into()),
                metadata_props: duplicates(),
                ..Default::default()
            }],
            ..Default::default()
        },
        proto::GraphProto {
            initializer: vec![proto::TensorProto {
                metadata_props: duplicates(),
                ..Default::default()
            }],
            ..Default::default()
        },
        proto::GraphProto {
            initializer: vec![proto::TensorProto {
                external_data: duplicates(),
                ..Default::default()
            }],
            ..Default::default()
        },
    ];
    for graph in graphs {
        assert!(
            matches!(inspect_graph(graph), Err(ArtifactError::DuplicateMetadata { key, .. }) if key == "same")
        );
    }
}

#[test]
fn rejects_malformed_protobuf_missing_graph_and_excessive_recursion() {
    for bytes in [
        vec![0x3a, 0x02, 0x5a],
        vec![0x80],
        vec![0x00],
        vec![0x3a, 0x01, 0x0f],
    ] {
        assert!(matches!(
            inspect_onnx(&bytes),
            Err(ArtifactError::InvalidProtobuf(_))
        ));
    }
    assert!(matches!(
        inspect_onnx(&[]),
        Err(ArtifactError::MissingGraph)
    ));
    let mut nested = tensor(1, None);
    for _ in 0..120 {
        nested = proto::TypeProto {
            value: Some(Value::Optional(proto::SequenceTypeProto {
                elem_type: Some(Box::new(nested)),
            })),
            denotation: None,
        };
    }
    assert!(matches!(
        inspect_graph(proto::GraphProto {
            input: vec![port("nested", nested)],
            ..Default::default()
        }),
        Err(ArtifactError::InvalidProtobuf(_))
    ));
}

#[test]
fn preserves_opaque_and_absent_types_without_guessing() {
    let interface = inspect_graph(proto::GraphProto {
        output: vec![
            port(
                "opaque",
                proto::TypeProto {
                    value: Some(Value::Opaque(proto::OpaqueTypeProto {
                        domain: Some("custom".into()),
                        name: Some("handle".into()),
                    })),
                    denotation: None,
                },
            ),
            port("unknown", proto::TypeProto::default()),
            proto::ValueInfoProto {
                name: Some("absent".into()),
                ..Default::default()
            },
        ],
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        kind(&interface.outputs[0]),
        &ArtifactValueKind::Opaque {
            domain: Some("custom".into()),
            name: Some("handle".into())
        }
    );
    assert_eq!(kind(&interface.outputs[1]), &ArtifactValueKind::Unknown);
    assert!(interface.outputs[2].value_type.is_none());
}
