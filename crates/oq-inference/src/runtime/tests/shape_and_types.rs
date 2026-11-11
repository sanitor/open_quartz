use super::*;

fn identity_input() -> Vec<NamedTensor> {
    vec![NamedTensor {
        name: "input".into(),
        data: vec![-3.5, 7.25],
        shape: vec![1, 2],
    }]
}

fn assert_identity_retry(session: &mut InferenceSession, stamp: RequestStamp) {
    let result = session
        .invoke_with_output_shapes(
            stamp,
            identity_input(),
            &["output".into()],
            &[("output", &[1, 2])],
        )
        .unwrap();
    assert_eq!(result.stamp, stamp);
    assert_eq!(result.outputs.len(), 1);
    assert_eq!(result.outputs[0].name, "output");
    assert_eq!(result.outputs[0].shape, [1, 2]);
    assert_eq!(result.outputs[0].data, [-3.5, 7.25]);
    assert_eq!(result.metrics.input_cpu_bytes, 8);
    assert_eq!(result.metrics.output_cpu_bytes, 8);
    assert_eq!(session.generation(), stamp.session_generation);
}

#[test]
fn task_owned_output_shapes_validate_before_run_and_allow_exact_retry() {
    configure_runtime();
    let mut session =
        InferenceSession::from_memory(&identity_model(&[("input", "output")], &[None, Some(2)]))
            .unwrap();
    let stamp = RequestStamp {
        request_id: 301,
        session_generation: session.generation(),
    };
    assert_identity_retry(&mut session, stamp);

    let wrong_rank = session.invoke_with_output_shapes(
        stamp,
        identity_input(),
        &["output".into()],
        &[("output", &[2])],
    );
    assert!(
        matches!(wrong_rank, Err(InferenceError::Rank { name, expected: 2, actual: 1 }) if name == "output")
    );
    assert_identity_retry(&mut session, stamp);

    for shape in [[0, 2], [-1, 2]] {
        assert!(matches!(
            session.invoke_with_output_shapes(
                stamp,
                identity_input(),
                &["output".into()],
                &[("output", &shape)]
            ),
            Err(InferenceError::InvalidModelInterface(_))
        ));
        assert_identity_retry(&mut session, stamp);
    }

    assert!(matches!(
        session.invoke_with_output_shapes(stamp, identity_input(), &["output".into()], &[("output", &[1, 3])]),
        Err(InferenceError::Dimension { name, axis: 1, actual: 3, .. }) if name == "output"
    ));
    assert_identity_retry(&mut session, stamp);

    // The element count fits usize on the supported 64-bit hosts, but its FP32
    // byte count cannot fit u64. No huge tensor allocation is attempted.
    assert!(matches!(
        session.invoke_with_output_shapes(stamp, identity_input(), &["output".into()], &[("output", &[i64::MAX, 2])]),
        Err(InferenceError::SizeOverflow(name)) if name == "output"
    ));
    assert_identity_retry(&mut session, stamp);

    assert!(matches!(
        session.invoke_with_output_shapes_and_limits(
            stamp,
            identity_input(),
            &["output".into()],
            &[("output", &[2, 2])],
            TensorLimits { max_tensor_bytes: 12, max_invocation_bytes: 64 },
        ),
        Err(InferenceError::Capacity { name, bytes: 16, limit: 12 }) if name == "output"
    ));
    assert_identity_retry(&mut session, stamp);
}

#[test]
fn task_owned_output_shape_checks_actual_ort_result_then_session_remains_usable() {
    configure_runtime();
    let mut session =
        InferenceSession::from_memory(&identity_model(&[("input", "output")], &[None, Some(2)]))
            .unwrap();
    let stamp = RequestStamp {
        request_id: 302,
        session_generation: session.generation(),
    };
    // A symbolic output permits this claim at preflight. ORT really returns
    // [1, 2], so only the post-execution exact-shape check can reject it.
    assert!(matches!(
        session.invoke_with_output_shapes(stamp, identity_input(), &["output".into()], &[("output", &[2, 2])]),
        Err(InferenceError::Dimension { name, axis: 0, actual: 1, .. }) if name == "output"
    ));
    assert_identity_retry(&mut session, stamp);
}

#[test]
fn task_owned_output_element_product_overflow_is_checked_without_allocating() {
    configure_runtime();
    let mut session =
        InferenceSession::from_memory(&identity_model(&[("input", "output")], &[None, None]))
            .unwrap();
    let stamp = RequestStamp {
        request_id: 303,
        session_generation: session.generation(),
    };
    let input = || {
        vec![NamedTensor {
            name: "input".into(),
            data: vec![9.0],
            shape: vec![1, 1],
        }]
    };
    assert!(matches!(
        session.invoke_with_output_shapes(stamp, input(), &["output".into()], &[("output", &[i64::MAX, i64::MAX])]),
        Err(InferenceError::SizeOverflow(name)) if name == "output"
    ));
    let result = session
        .invoke_with_output_shapes(stamp, input(), &["output".into()], &[("output", &[1, 1])])
        .unwrap();
    assert_eq!(result.outputs[0].shape, [1, 1]);
    assert_eq!(result.outputs[0].data, [9.0]);
}

#[test]
fn ort_identity_metadata_preserves_supported_non_fp32_types_and_rejects_fp32_invocation() {
    configure_runtime();
    // These are ONNX TensorProto element IDs, not the SDK's enum discriminants.
    for (onnx_type, expected) in [
        (2, TensorElementType::Uint8),
        (3, TensorElementType::Int8),
        (4, TensorElementType::Uint16),
        (5, TensorElementType::Int16),
        (6, TensorElementType::Int32),
        (7, TensorElementType::Int64),
        (8, TensorElementType::String),
        (9, TensorElementType::Bool),
        (10, TensorElementType::Float16),
        (11, TensorElementType::Float64),
        (12, TensorElementType::Uint32),
        (13, TensorElementType::Uint64),
        (16, TensorElementType::BFloat16),
    ] {
        let mut session = InferenceSession::from_memory(&identity_model_with_type(
            &[("input", "output")],
            &[None, Some(2)],
            onnx_type,
        ))
        .unwrap();
        for metadata in [&session.info().inputs[0], &session.info().outputs[0]] {
            assert_eq!(metadata.element_type, expected);
            assert_eq!(
                metadata.dimensions,
                [
                    TensorDimension::Symbol("batch".into()),
                    TensorDimension::Static(2)
                ]
            );
        }
        for metadata in [
            &session.info().input_values[0],
            &session.info().output_values[0],
        ] {
            assert!(
                matches!(&metadata.kind, ValueKind::Tensor { element_type, dimensions }
                if *element_type == expected && *dimensions == [TensorDimension::Symbol("batch".into()), TensorDimension::Static(2)])
            );
        }
        let stamp = RequestStamp {
            request_id: onnx_type,
            session_generation: session.generation(),
        };
        // Deliberately valid FP32 tensor storage must not be reinterpreted as
        // the declared type, including strings and half-precision elements.
        for _ in 0..2 {
            assert!(matches!(
                session.invoke(stamp, identity_input(), &["output".into()]),
                Err(InferenceError::UnsupportedType { name, .. }) if name == "input"
            ));
            assert_eq!(session.generation(), stamp.session_generation);
        }
    }
}

fn node(inputs: &[&str], outputs: &[&str], op: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    for input in inputs {
        bytes.extend(message(1, input.as_bytes()));
    }
    for output in outputs {
        bytes.extend(message(2, output.as_bytes()));
    }
    bytes.extend(message(4, op.as_bytes()));
    bytes
}

fn graph_model(graph: &[u8], opset: u64) -> Vec<u8> {
    [
        integer(1, 8),
        message(7, graph),
        message(8, &integer(2, opset)),
    ]
    .concat()
}

fn recursive_output_model(optional: bool) -> Vec<u8> {
    // A real round trip through the container returns a tensor alongside the
    // container output, allowing the public FP32 API to execute the graph.
    let (construct, extract, container_type) = if optional {
        ("Optional", "OptionalGetElement", 9)
    } else {
        ("SequenceConstruct", "SequenceAt", 4)
    };
    let tensor_type = [integer(1, 1), message(2, &message(1, &integer(1, 2)))].concat();
    let nested_type = message(container_type, &message(1, &message(1, &tensor_type)));
    let mut graph = message(2, b"recursive-round-trip");
    graph.extend(message(1, &node(&["input"], &["container"], construct)));
    if optional {
        graph.extend(message(1, &node(&["container"], &["output"], extract)));
    } else {
        // Scalar int64 index initializer at position zero.
        let index = [integer(2, 7), integer(7, 0), message(8, b"index")].concat();
        graph.extend(message(5, &index));
        graph.extend(message(
            1,
            &node(&["container", "index"], &["output"], extract),
        ));
    }
    graph.extend(message(11, &value_info("input", &[Some(2)], 1)));
    graph.extend(message(
        12,
        &[message(1, b"container"), message(2, &nested_type)].concat(),
    ));
    graph.extend(message(12, &value_info("output", &[Some(2)], 1)));
    graph_model(&graph, 15)
}

#[test]
fn ort_recursive_sequence_and_optional_metadata_comes_from_executable_models() {
    configure_runtime();
    for optional in [false, true] {
        let mut session = InferenceSession::from_memory(&recursive_output_model(optional)).unwrap();
        let kind = &session.info().output_values[0].kind;
        let element = match kind {
            ValueKind::Optional { element } if optional => element,
            ValueKind::Sequence { element } if !optional => element,
            other => panic!("unexpected ORT container metadata: {other:?}"),
        };
        assert_eq!(
            **element,
            ValueKind::Tensor {
                element_type: TensorElementType::Float32,
                dimensions: vec![TensorDimension::Static(2)],
            }
        );
        assert_eq!(
            session.info().output_values[0].optionality,
            if optional {
                ValueOptionality::Optional
            } else {
                ValueOptionality::Unknown
            }
        );
        let stamp = RequestStamp {
            request_id: 304,
            session_generation: session.generation(),
        };
        let input = || {
            vec![NamedTensor {
                name: "input".into(),
                data: vec![2.5, -8.0],
                shape: vec![2],
            }]
        };
        assert!(matches!(
            session.invoke(stamp, input(), &["container".into()]),
            Err(InferenceError::UnsupportedType { name, .. }) if name == "container"
        ));
        let result = session.invoke(stamp, input(), &["output".into()]).unwrap();
        assert_eq!(result.outputs[0].shape, [2]);
        assert_eq!(result.outputs[0].data, [2.5, -8.0]);
    }
}

fn gather_model() -> Vec<u8> {
    let to_int64 = [message(1, b"to"), integer(3, 7), integer(20, 2)].concat();
    let cast = [
        node(&["index"], &["integer_index"], "Cast"),
        message(5, &to_int64),
    ]
    .concat();
    let graph = [
        message(2, b"gather-runtime-failure"),
        message(1, &cast),
        message(1, &node(&["input", "integer_index"], &["output"], "Gather")),
        message(11, &value_info("input", &[Some(2)], 1)),
        message(11, &value_info("index", &[Some(1)], 1)),
        message(12, &value_info("output", &[Some(1)], 1)),
    ]
    .concat();
    graph_model(&graph, 13)
}

#[test]
fn ort_execution_failure_is_typed_and_valid_retry_executes_same_session() {
    configure_runtime();
    let mut session = InferenceSession::from_memory(&gather_model()).unwrap();
    let stamp = RequestStamp {
        request_id: 305,
        session_generation: session.generation(),
    };
    let inputs = |index| {
        vec![
            NamedTensor {
                name: "input".into(),
                data: vec![11.0, -4.25],
                shape: vec![2],
            },
            NamedTensor {
                name: "index".into(),
                data: vec![index],
                shape: vec![1],
            },
        ]
    };
    assert!(matches!(
        session.invoke(stamp, inputs(2.0), &["output".into()]),
        Err(InferenceError::Runtime {
            operation: "run model",
            ..
        })
    ));
    let result = session
        .invoke(stamp, inputs(1.0), &["output".into()])
        .unwrap();
    assert_eq!(result.outputs[0].shape, [1]);
    assert_eq!(result.outputs[0].data, [-4.25]);
    assert_eq!(result.stamp, stamp);
    assert_eq!(session.generation(), stamp.session_generation);
}

#[test]
fn well_formed_model_with_unregistered_operator_reports_ort_load_failure() {
    configure_runtime();
    let graph = [
        message(2, b"unsupported-operator"),
        message(
            1,
            &node(&["input"], &["output"], "OpenQuartzUnregisteredOperator"),
        ),
        message(11, &value_info("input", &[Some(2)], 1)),
        message(12, &value_info("output", &[Some(2)], 1)),
    ]
    .concat();
    assert!(matches!(
        InferenceSession::from_memory(&graph_model(&graph, 13)),
        Err(InferenceError::Runtime {
            operation: "load model",
            ..
        })
    ));
    // Failure to load a distinct, well-formed graph must not poison the global
    // runtime used to construct and execute a subsequent supported model.
    let mut session =
        InferenceSession::from_memory(&identity_model(&[("input", "output")], &[None, Some(2)]))
            .unwrap();
    let stamp = RequestStamp {
        request_id: 306,
        session_generation: session.generation(),
    };
    assert_identity_retry(&mut session, stamp);
}

#[test]
fn task_owned_output_enforces_combined_input_output_capacity_after_execution() {
    configure_runtime();
    let mut session =
        InferenceSession::from_memory(&identity_model(&[("input", "output")], &[None, Some(2)]))
            .unwrap();
    let stamp = RequestStamp {
        request_id: 307,
        session_generation: session.generation(),
    };
    assert!(matches!(
        session.invoke_with_output_shapes_and_limits(
            stamp,
            identity_input(),
            &["output".into()],
            &[("output", &[1, 2])],
            TensorLimits { max_tensor_bytes: 8, max_invocation_bytes: 12 },
        ),
        Err(InferenceError::Capacity { name, bytes: 16, limit: 12 }) if name == "invocation"
    ));
    assert_identity_retry(&mut session, stamp);
}
