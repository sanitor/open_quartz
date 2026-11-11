//! Deterministic real SR3 ONNX fixture for resident integration tests.
//! Center-tap convolutions preserve luminance; all nine shuffle channels carry it.

fn varint(mut value: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        let byte = (value & 127) as u8;
        value >>= 7;
        bytes.push(byte | if value != 0 { 128 } else { 0 });
        if value == 0 {
            break;
        }
    }
    bytes
}
fn number(tag: u64, value: i64) -> Vec<u8> {
    let mut bytes = varint(tag << 3);
    bytes.extend(varint(value as u64));
    bytes
}
fn message(tag: u64, value: &[u8]) -> Vec<u8> {
    let mut bytes = varint((tag << 3) | 2);
    bytes.extend(varint(value.len() as u64));
    bytes.extend(value);
    bytes
}
fn text(tag: u64, value: &str) -> Vec<u8> {
    message(tag, value.as_bytes())
}
fn join(parts: impl IntoIterator<Item = Vec<u8>>) -> Vec<u8> {
    parts.into_iter().flatten().collect()
}
fn value(name: &str, dimensions: &[i64], dtype: i64) -> Vec<u8> {
    let shape = join(dimensions.iter().map(|&dimension| {
        message(
            1,
            &if dimension < 0 {
                text(2, "batch_size")
            } else {
                number(1, dimension)
            },
        )
    }));
    join([
        text(1, name),
        message(
            2,
            &message(1, &join([number(1, dtype), message(2, &shape)])),
        ),
    ])
}
fn node(op: &str, inputs: &[&str], output: &str, attributes: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = join(inputs.iter().map(|input| text(1, input)));
    bytes.extend(text(2, output));
    bytes.extend(text(4, op));
    for attribute in attributes {
        bytes.extend(message(5, attribute));
    }
    bytes
}
fn ints(name: &str, values: &[i64]) -> Vec<u8> {
    join([
        text(1, name),
        message(8, &join(values.iter().map(|&value| varint(value as u64)))),
        number(20, 7),
    ])
}
fn constant(name: &str, values: &[i64]) -> Vec<u8> {
    let tensor = join([
        number(1, values.len() as i64),
        number(2, 7),
        message(
            9,
            &values
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect::<Vec<_>>(),
        ),
    ]);
    node(
        "Constant",
        &[],
        name,
        &[join([text(1, "value"), message(5, &tensor), number(20, 4)])],
    )
}
pub(crate) fn sr3_model() -> Vec<u8> {
    let mut graph = Vec::new();
    graph.extend(text(2, "resident-center-tap-SR3"));
    graph.extend(message(11, &value("input", &[-1, 1, 224, 224], 1)));
    graph.extend(message(12, &value("output", &[-1, 1, 672, 672], 1)));
    let mut previous = "input".to_owned();
    for (index, (input, output, kernel)) in [
        (1usize, 64usize, 5usize),
        (64, 64, 3),
        (64, 32, 3),
        (32, 9, 3),
    ]
    .into_iter()
    .enumerate()
    {
        let weight_name = format!("conv{index}.weight");
        let bias_name = format!("conv{index}.bias");
        let shape = [output as i64, input as i64, kernel as i64, kernel as i64];
        let mut weights = vec![0f32; output * input * kernel * kernel];
        for channel in 0..if index == 3 { 9 } else { 1 } {
            weights[channel * input * kernel * kernel + kernel * (kernel / 2) + kernel / 2] = 1.;
        }
        for (name, dims, data) in [
            (&weight_name, shape.to_vec(), weights),
            (&bias_name, vec![output as i64], vec![0f32; output]),
        ] {
            let mut tensor = join(dims.iter().map(|&dim| number(1, dim)));
            tensor.extend(number(2, 1));
            tensor.extend(text(8, name));
            tensor.extend(message(
                9,
                &data
                    .iter()
                    .flat_map(|value| value.to_le_bytes())
                    .collect::<Vec<_>>(),
            ));
            graph.extend(message(5, &tensor));
            graph.extend(message(11, &value(name, &dims, 1)));
        }
        let next = format!("conv{index}");
        graph.extend(message(
            1,
            &node(
                "Conv",
                &[&previous, &weight_name, &bias_name],
                &next,
                &[
                    ints("kernel_shape", &[kernel as i64; 2]),
                    ints("pads", &[(kernel / 2) as i64; 4]),
                    ints("strides", &[1, 1]),
                    ints("dilations", &[1, 1]),
                    join([text(1, "group"), number(3, 1), number(20, 2)]),
                ],
            ),
        ));
        previous = next;
        if index < 3 {
            let next = format!("relu{index}");
            graph.extend(message(1, &node("Relu", &[&previous], &next, &[])));
            previous = next;
        }
    }
    graph.extend(message(1, &constant("shape1", &[-1, 1, 3, 3, 224, 224])));
    graph.extend(message(
        1,
        &node("Reshape", &[&previous, "shape1"], "reshape1", &[]),
    ));
    graph.extend(message(
        1,
        &node(
            "Transpose",
            &["reshape1"],
            "transpose",
            &[ints("perm", &[0, 1, 4, 2, 5, 3])],
        ),
    ));
    graph.extend(message(1, &constant("shape2", &[-1, 1, 672, 672])));
    graph.extend(message(
        1,
        &node("Reshape", &["transpose", "shape2"], "output", &[]),
    ));
    join([
        number(1, 4),
        text(2, "open-quartz-resident-test"),
        message(7, &graph),
        message(8, &number(2, 10)),
    ])
}

pub(crate) fn mask_model() -> Vec<u8> {
    let mut graph = join([
        text(2, "resident-constant-alpha"),
        message(11, &value("input", &[1, 3, 512, 512], 1)),
        message(12, &value("output", &[1, 1, 512, 512], 1)),
        message(
            1,
            &node("Conv", &["input", "weights", "bias"], "output", &[]),
        ),
    ]);
    for (name, dims, data) in [
        ("weights", vec![1, 3, 1, 1], vec![0.0f32; 3]),
        ("bias", vec![1], vec![0.5f32]),
    ] {
        let tensor = join([
            join(dims.iter().map(|&d| number(1, d))),
            number(2, 1),
            text(8, name),
            message(
                9,
                &data
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect::<Vec<_>>(),
            ),
        ]);
        graph.extend(message(5, &tensor));
    }
    join([number(1, 8), message(7, &graph), message(8, &number(2, 13))])
}
