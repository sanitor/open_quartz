use super::*;
use crate::{
    catalog_gpu::{CatalogGpuSession, CatalogOptions},
    catalog_model::CatalogModel,
    gpu_model::proto,
};
use prost::Message;

fn varint(mut n: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let byte = (n & 127) as u8;
        n >>= 7;
        out.push(byte | if n == 0 { 0 } else { 128 });
        if n == 0 {
            return out;
        }
    }
}
fn number(field: u64, value: u64) -> Vec<u8> {
    [varint(field << 3), varint(value)].concat()
}
fn message(field: u64, value: &[u8]) -> Vec<u8> {
    [
        varint((field << 3) | 2),
        varint(value.len() as u64),
        value.to_vec(),
    ]
    .concat()
}
fn image_value(name: &str, dims: &[u64]) -> Vec<u8> {
    let shape = dims
        .iter()
        .flat_map(|&n| message(1, &number(1, n)))
        .collect::<Vec<_>>();
    let tensor = [number(1, 1), message(2, &shape)].concat();
    [
        message(1, name.as_bytes()),
        message(2, &message(1, &tensor)),
    ]
    .concat()
}
fn mask_model() -> Vec<u8> {
    // A full supported MODNet-shaped graph with analytically constant alpha.
    // The three zero channel weights and bias 0.5 make the independent oracle exact.
    let graph = proto::Graph {
        node: vec![proto::Node {
            op_type: "Conv".into(),
            input: vec!["rgb".into(), "weights".into(), "bias".into()],
            output: vec!["alpha".into()],
            ..Default::default()
        }],
        initializer: vec![
            proto::Tensor {
                name: "weights".into(),
                dims: vec![1, 3, 1, 1],
                data_type: Some(1),
                float_data: vec![0.0; 3],
                ..Default::default()
            },
            proto::Tensor {
                name: "bias".into(),
                dims: vec![1],
                data_type: Some(1),
                float_data: vec![0.5],
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let graph = [
        graph.encode_to_vec(),
        message(11, &image_value("rgb", &[1, 3, 512, 512])),
        message(12, &image_value("alpha", &[1, 1, 512, 512])),
    ]
    .concat();
    [number(1, 8), message(7, &graph), message(8, &number(2, 13))].concat()
}
#[test]
fn catalog_mask_native_graph_keeps_source_rgb_and_bounded_alpha_leases_across_resize() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance
            .request_adapter(&Default::default())
            .await
            .expect("hardware Metal required");
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let (device, queue) = (Arc::new(device), Arc::new(queue));
        let bytes = mask_model();
        let model = CatalogModel::from_onnx(&bytes, "modnet", 512, 512).unwrap();
        for options in [
            CatalogOptions {
                score_threshold: f32::NAN,
                iou_threshold: 0.5,
            },
            CatalogOptions {
                score_threshold: 0.5,
                iou_threshold: 1.1,
            },
        ] {
            assert!(matches!(
                CatalogGpuSession::prepare(device.clone(), queue.clone(), model.clone(), options),
                Err(GpuError::InvalidImage(_))
            ));
        }
        let mut session =
            CatalogGpuSession::prepare(device.clone(), queue.clone(), model, Default::default())
                .unwrap();
        let source = texture(
            &device,
            2,
            2,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let mut held = Vec::new();
        for (color, expected) in [
            ("red", [255, 0, 0, 128]),
            ("green", [0, 255, 0, 128]),
            ("blue", [0, 0, 255, 128]),
        ] {
            generate(&device, &queue, &source, color);
            let output = session.submit_texture(&source, 2, 2).unwrap();
            assert_eq!(
                readback_texture(&device, &queue, output.texture()),
                expected.repeat(4)
            );
            held.push(output);
        }
        assert!(matches!(
            session.submit_texture(&source, 2, 2),
            Err(GpuError::Capacity)
        ));
        assert_eq!(
            readback_texture(&device, &queue, held[0].texture()),
            [255, 0, 0, 128].repeat(4)
        );
        let dropped = held.remove(1);
        drop(dropped);
        let replaced = session.submit_texture(&source, 2, 2).unwrap();
        assert_eq!(
            readback_texture(&device, &queue, replaced.texture()),
            [0, 0, 255, 128].repeat(4)
        );
        drop(replaced);
        drop(held.remove(1));
        let resized = texture(
            &device,
            3,
            1,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        generate(&device, &queue, &resized, "green");
        let output = session.submit_texture(&resized, 3, 1).unwrap();
        assert_eq!(
            readback_texture(&device, &queue, output.texture()),
            [0, 255, 0, 128].repeat(3)
        );
        assert!(matches!(
            session.submit_texture(&resized, 0, 1),
            Err(GpuError::InvalidImage(_))
        ));
        assert_eq!(session.counters().host_image_bytes, 0);
        assert_eq!(session.counters().submissions, 5);
        drop(session);
        assert_eq!(
            readback_texture(&device, &queue, held[0].texture()),
            [255, 0, 0, 128].repeat(4)
        );
        assert_eq!(
            readback_texture(&device, &queue, output.texture()),
            [0, 255, 0, 128].repeat(3)
        );
    });
}

fn constant_catalog_model(
    id: &str,
    side: u64,
    output_name: &str,
    output_shape: &[u64],
    dtype: i32,
    values: Vec<f32>,
) -> Vec<u8> {
    let tensor = proto::Tensor {
        dims: output_shape.iter().map(|&d| d as i64).collect(),
        data_type: Some(dtype),
        float_data: if dtype == 1 { values.clone() } else { vec![] },
        int32_data: if dtype == 2 {
            values.iter().map(|v| *v as i32).collect()
        } else {
            vec![]
        },
        ..Default::default()
    };
    let node = proto::Node {
        op_type: "Constant".into(),
        output: vec![output_name.into()],
        attribute: vec![proto::Attribute {
            name: "value".into(),
            r#type: Some(4),
            t: Some(tensor),
            ..Default::default()
        }],
        ..Default::default()
    };
    let graph = proto::Graph {
        node: vec![node],
        ..Default::default()
    };
    let value = |name: &str, dims: &[u64], ty: u64| {
        let shape = dims
            .iter()
            .flat_map(|&n| message(1, &number(1, n)))
            .collect::<Vec<_>>();
        let tensor = [number(1, ty), message(2, &shape)].concat();
        [
            message(1, name.as_bytes()),
            message(2, &message(1, &tensor)),
        ]
        .concat()
    };
    let graph = [
        graph.encode_to_vec(),
        message(11, &image_value("rgb", &[1, 3, side, side])),
        message(12, &value(output_name, output_shape, dtype as u64)),
    ]
    .concat();
    let _ = id;
    [number(1, 8), message(7, &graph), message(8, &number(2, 13))].concat()
}

fn read_buffer(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: &wgpu::Buffer,
    size: u64,
) -> Vec<u8> {
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("explicit catalog result oracle"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(source, 0, &staging, 0, size);
    let submission = queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    staging.map_async(wgpu::MapMode::Read, .., move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(Duration::from_secs(30)),
        })
        .unwrap();
    rx.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
    let mapped = staging.get_mapped_range(..).unwrap();
    let bytes = mapped.to_vec();
    drop(mapped);
    staging.unmap();
    bytes
}

#[test]
fn catalog_depth_segmentation_and_detection_preserve_raw_tensor_and_presentation() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance
            .request_adapter(&Default::default())
            .await
            .expect("hardware Metal required");
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let (device, queue) = (Arc::new(device), Arc::new(queue));
        for (id, side, name, shape, dtype, values, expected) in [
            (
                "midas-small",
                256,
                "output_depth",
                vec![1, 256, 256],
                1,
                vec![2.0; 256 * 256],
                [0, 0, 0, 255],
            ),
            (
                "yolo26n-sem",
                640,
                "classes",
                vec![1, 640, 640],
                2,
                vec![2.0; 640 * 640],
                [162, 35, 35, 255],
            ),
            (
                "yolov8n",
                640,
                "boxes",
                vec![1, 84, 8400],
                1,
                vec![0.0; 84 * 8400],
                [255, 0, 0, 255],
            ),
        ] {
            let bytes = if id == "yolo26n-sem" {
                semantic_model()
            } else {
                constant_catalog_model(id, side, name, &shape, dtype, values)
            };
            let model = CatalogModel::from_onnx(&bytes, id, side as u32, side as u32).unwrap();
            let mut session = CatalogGpuSession::prepare(
                device.clone(),
                queue.clone(),
                model,
                Default::default(),
            )
            .unwrap();
            let source = texture(
                &device,
                2,
                2,
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            );
            generate(&device, &queue, &source, "red");
            let output = session.submit_texture(&source, 2, 2).unwrap();
            assert_eq!(
                readback_texture(&device, &queue, output.texture()),
                expected.repeat(4),
                "{id}"
            );
            assert_eq!(
                read_buffer(&device, &queue, &output.raw_result_buffer(), 4),
                if id == "yolov8n" { 0.0f32 } else { 2.0f32 }.to_ne_bytes(),
                "{id} original tensor"
            );
            if id == "midas-small" {
                let bytes = read_buffer(&device, &queue, &output.depth_range_buffer().unwrap(), 8);
                assert_eq!(bytes, [2.0f32.to_ne_bytes(), 2.0f32.to_ne_bytes()].concat());
            }
            if id == "yolov8n" {
                assert_eq!(
                    read_buffer(&device, &queue, &output.detection_buffer().unwrap(), 16),
                    vec![0; 16]
                );
            } else {
                assert!(output.detection_buffer().is_none());
            }
            assert_eq!(session.counters().host_image_bytes, 0);
        }
    });
}

#[test]
fn catalog_graph_rejects_unavailable_edges_duplicate_producers_and_invalid_domains() {
    let original = mask_model();
    let parsed = proto::Model::decode(original.as_slice()).unwrap();
    for fault in 0..10 {
        let mut graph = parsed.graph.clone().unwrap();
        match fault {
            0 => graph.node.clear(),
            1 => graph.node[0].op_type = "UnknownOperation".into(),
            2 => graph.node[0].domain = "custom.domain".into(),
            3 => graph.node[0].input[0] = "missing-image".into(),
            4 => {
                let node = graph.node[0].clone();
                graph.node.push(node);
            }
            5 => graph.node[0].output[0] = "missing-output".into(),
            6 => {
                let weight = graph.initializer[0].clone();
                graph.initializer.push(weight);
            }
            7 => graph.node[0].input.clear(),
            8 => {
                graph.node[0].attribute = vec![
                    proto::Attribute {
                        name: "kernel_shape".into(),
                        r#type: Some(7),
                        ints: vec![1, 1],
                        ..Default::default()
                    };
                    2
                ]
            }
            _ => graph.node[0].output[0] = String::new(),
        }
        let encoded = [
            graph.encode_to_vec(),
            message(11, &image_value("rgb", &[1, 3, 512, 512])),
            message(12, &image_value("alpha", &[1, 1, 512, 512])),
        ]
        .concat();
        let bytes = [
            number(1, 8),
            message(7, &encoded),
            message(8, &number(2, 13)),
        ]
        .concat();
        assert!(
            CatalogModel::from_onnx(&bytes, "modnet", 512, 512).is_err(),
            "invalid graph {fault} admitted"
        );
    }
    assert!(CatalogModel::from_onnx(&original, "modnet", 511, 512).is_err());
    assert!(CatalogModel::from_onnx(&original, "unrecognized", 512, 512).is_err());
}

fn reconstruction_model() -> Vec<u8> {
    let mut weights = vec![0.0f32; 48 * 3];
    for output in 0..48 {
        weights[output * 3 + output % 3] = 1.0;
    }
    let graph = proto::Graph {
        node: vec![
            proto::Node {
                op_type: "Conv".into(),
                input: vec!["rgb".into(), "weights".into()],
                output: vec!["features".into()],
                ..Default::default()
            },
            proto::Node {
                op_type: "DepthToSpace".into(),
                input: vec!["features".into()],
                output: vec!["upscaled".into()],
                attribute: vec![proto::Attribute {
                    name: "blocksize".into(),
                    r#type: Some(2),
                    i: Some(4),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        initializer: vec![proto::Tensor {
            name: "weights".into(),
            dims: vec![48, 3, 1, 1],
            data_type: Some(1),
            float_data: weights,
            ..Default::default()
        }],
        ..Default::default()
    };
    let graph = [
        graph.encode_to_vec(),
        message(11, &image_value("rgb", &[1, 3, 2, 2])),
        message(12, &image_value("upscaled", &[1, 3, 8, 8])),
    ]
    .concat();
    [number(1, 8), message(7, &graph), message(8, &number(2, 13))].concat()
}

#[test]
fn catalog_reconstruction_resizes_native_profile_without_overwriting_prior_output() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance
            .request_adapter(&Default::default())
            .await
            .expect("hardware Metal required");
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let (device, queue) = (Arc::new(device), Arc::new(queue));
        let bytes = reconstruction_model();
        let model = CatalogModel::from_onnx(&bytes, "realesrgan-x4", 2, 2).unwrap();
        let mut session =
            CatalogGpuSession::prepare(device.clone(), queue.clone(), model, Default::default())
                .unwrap();
        let source = texture(
            &device,
            2,
            2,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        generate(&device, &queue, &source, "red");
        let first = session.submit_texture(&source, 2, 2).unwrap();
        assert_eq!((first.width(), first.height()), (8, 8));
        assert_eq!(
            readback_texture(&device, &queue, first.texture()),
            [255, 0, 0, 255].repeat(64)
        );
        let small = texture(
            &device,
            3,
            1,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        generate(&device, &queue, &small, "green");
        let resized = session.submit_texture(&small, 3, 1).unwrap();
        assert_eq!((resized.width(), resized.height()), (12, 4));
        assert_eq!(
            readback_texture(&device, &queue, resized.texture()),
            [0, 255, 0, 255].repeat(48)
        );
        assert_eq!(
            (
                session.model_plan().input_width,
                session.model_plan().output_width
            ),
            (3, 12)
        );
        assert_eq!(
            readback_texture(&device, &queue, first.texture()),
            [255, 0, 0, 255].repeat(64)
        );
        let restored = session.submit_texture(&source, 2, 2).unwrap();
        assert_eq!(
            readback_texture(&device, &queue, restored.texture()),
            [255, 0, 0, 255].repeat(64)
        );
        assert_eq!(
            readback_texture(&device, &queue, resized.texture()),
            [0, 255, 0, 255].repeat(48)
        );
        assert_eq!(session.counters().host_image_bytes, 0);
    });
}

fn semantic_model() -> Vec<u8> {
    let mut bias = vec![0.0f32; 19];
    bias[2] = 1.0;
    let graph = proto::Graph {
        node: vec![
            proto::Node {
                op_type: "Conv".into(),
                input: vec!["rgb".into(), "weights".into(), "bias".into()],
                output: vec!["classifier".into()],
                ..Default::default()
            },
            proto::Node {
                op_type: "Resize".into(),
                input: vec!["classifier".into(), String::new(), "scales".into()],
                output: vec!["logits".into()],
                attribute: vec![
                    proto::Attribute {
                        name: "coordinate_transformation_mode".into(),
                        r#type: Some(3),
                        s: Some(b"asymmetric".to_vec()),
                        ..Default::default()
                    },
                    proto::Attribute {
                        name: "nearest_mode".into(),
                        r#type: Some(3),
                        s: Some(b"floor".to_vec()),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            proto::Node {
                op_type: "ArgMax".into(),
                input: vec!["logits".into()],
                output: vec!["ids".into()],
                attribute: vec![
                    proto::Attribute {
                        name: "axis".into(),
                        r#type: Some(2),
                        i: Some(1),
                        ..Default::default()
                    },
                    proto::Attribute {
                        name: "keepdims".into(),
                        r#type: Some(2),
                        i: Some(0),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            proto::Node {
                op_type: "Cast".into(),
                input: vec!["ids".into()],
                output: vec!["classes".into()],
                attribute: vec![proto::Attribute {
                    name: "to".into(),
                    r#type: Some(2),
                    i: Some(2),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        initializer: vec![
            proto::Tensor {
                name: "weights".into(),
                dims: vec![19, 3, 1, 1],
                data_type: Some(1),
                float_data: vec![0.0; 19 * 3],
                ..Default::default()
            },
            proto::Tensor {
                name: "bias".into(),
                dims: vec![19],
                data_type: Some(1),
                float_data: bias,
                ..Default::default()
            },
            proto::Tensor {
                name: "scales".into(),
                dims: vec![4],
                data_type: Some(1),
                float_data: vec![1.0; 4],
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let shape = [1, 640, 640]
        .iter()
        .flat_map(|&n| message(1, &number(1, n)))
        .collect::<Vec<_>>();
    let ty = [number(1, 2), message(2, &shape)].concat();
    let output = [message(1, b"classes"), message(2, &message(1, &ty))].concat();
    let graph = [
        graph.encode_to_vec(),
        message(11, &image_value("rgb", &[1, 3, 640, 640])),
        message(12, &output),
    ]
    .concat();
    [number(1, 8), message(7, &graph), message(8, &number(2, 13))].concat()
}
