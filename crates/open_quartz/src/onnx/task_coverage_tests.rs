use super::*;
use serde_json::json;

fn tile(channels: usize) -> TileDescriptor {
    TileDescriptor {
        tile_x: 0,
        tile_y: 0,
        tile_width: 2,
        tile_height: 2,
        patch_x: 0,
        patch_y: 0,
        patch_width: 2,
        patch_height: 2,
        pad_left: 0,
        pad_top: 0,
        pad_right: 0,
        pad_bottom: 0,
        input: tensor_descriptor(channels, 2, 2),
    }
}

fn plan(family: OnnxModelFamily, channels: usize) -> BrowserOnnxTaskPlan {
    BrowserOnnxTaskPlan {
        model_id: "coverage".to_owned(),
        task: OnnxTask::Generic,
        family,
        source_width: 2,
        source_height: 2,
        target_size: None,
        thresholds: None,
        class_labels: None,
        execution: OnnxExecutionPlan::Tiled {
            output_width: 2,
            output_height: 2,
            scale: 1,
            channels,
            tile_size: 2,
            min_tile_size: 1,
            tile_pad: 0,
            fixed_size: Some(2),
            tiles: vec![tile(channels)],
        },
    }
}

#[test]
fn task_helpers_cover_shapes_params_validation_and_tile_generation() {
    assert!(plan_browser_onnx_task(BrowserOnnxTaskPlanRequest {
        model_id: None,
        task: None,
        source_width: 0,
        source_height: 1,
        target_size: None,
        tile_size: None,
        params: Value::Null,
        input_shape: vec![],
        output_shape: vec![],
    })
    .is_err());
    let ycbcr = plan_browser_onnx_task(BrowserOnnxTaskPlanRequest {
        model_id: None,
        task: None,
        source_width: 1,
        source_height: 1,
        target_size: None,
        tile_size: None,
        params: Value::Null,
        input_shape: vec![json!(1), json!(1), json!(1), json!(1)],
        output_shape: vec![json!(1), json!(1), json!(1), json!(1)],
    })
    .unwrap();
    assert_eq!(ycbcr.model_id, "custom");
    assert_eq!(ycbcr.family, OnnxModelFamily::GenericYcbcr);
    assert_eq!(number_param(&json!({"x": 2.5}), "x", 1.0), 2.5);
    assert_eq!(number_param(&json!({}), "x", 1.0), 1.0);
    assert_eq!(shape_dim(&[json!(1), json!("dynamic")], 0), Some(1));
    assert_eq!(fixed_square(&[json!(1), json!(3), json!(4)]), None);
    assert_eq!(shape_dim(&[json!(1), json!("dynamic")], 1), None);
    assert_eq!(shape_dim(&[], 0), None);
    assert_eq!(
        fixed_square(&[json!(1), json!(3), json!(4), json!(4)]),
        Some(4)
    );
    assert_eq!(
        fixed_square(&[json!(1), json!(3), json!(4), json!(5)]),
        None
    );
    assert_eq!(
        model_scale(
            &[json!(1), json!(3), json!(2)],
            &[json!(1), json!(3), json!(8)]
        ),
        4
    );
    assert_eq!(output_size(&[], 7, 9), (7, 9));
    assert!(validate_rgba(&[0; 16], 2, 2).is_ok());
    assert!(validate_rgba(&[], 0, 0).is_err());
    assert!(is_sampler(DataType::Sampler2d));
    assert!(is_sampler(DataType::SamplerCube));
    assert!(!is_sampler(DataType::Float));
    assert_eq!(float_to_byte(-1.0), 0);
    assert_eq!(float_to_byte(2.0), 255);

    let OnnxExecutionPlan::Tiled {
        tiles, tile_size, ..
    } = tiled_execution(5, 3, 2, 3, None, Some(2))
    else {
        panic!()
    };
    assert_eq!(tile_size, 2);
    assert_eq!(tiles.len(), 6);
    assert_eq!(tiles.last().unwrap().tile_width, 1);
    let OnnxExecutionPlan::Tiled { tile_size, .. } = tiled_execution(2, 2, 1, 3, Some(1), None)
    else {
        panic!()
    };
    assert_eq!(tile_size, 1);
}

#[test]
fn task_tile_codecs_cover_rgb_ycbcr_depth_alpha_and_errors() {
    let rgba = vec![
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
    ];
    let rgb_tile = tile(3);
    assert_eq!(encode_rgb_tile(&rgba, 2, &rgb_tile, 2, 2).len(), 12);
    assert_eq!(encode_ycbcr_tile(&rgba, 2, &tile(1)).len(), 4);
    assert_eq!(encode_depth_tile(&rgba, 2, &rgb_tile).unwrap().len(), 12);
    assert_eq!(extract_patch_rgba(&rgba, 2, &rgb_tile), rgba);
    assert_eq!(crop_rgba(&rgba, 2, 1, 0, 1, 2).unwrap().len(), 8);
    assert!(crop_rgba(&rgba, 2, 4, 0, 1, 1).is_err());

    let rgb = decode_rgb_tile(&[1.0; 12], &rgb_tile, 1, 2, 2).unwrap();
    assert_eq!(rgb.len(), 16);
    let ycbcr = decode_ycbcr_tile(&rgba, &[0.5; 4], 2, 2, &tile(1), 1, 2).unwrap();
    assert_eq!(ycbcr.len(), 16);
    assert!(decode_ycbcr_tile(&rgba, &[], 2, 2, &tile(1), 1, 2).is_err());
    let alpha = decode_alpha_tile(&rgba, &[0.5; 4], 2, &rgb_tile, 2).unwrap();
    assert_eq!(alpha.len(), 16);
    let depth = decode_depth_tile(&[0.0, 1.0, 2.0, 3.0], &rgb_tile, 2).unwrap();
    assert_eq!(depth.len(), 16);
    assert!(decode_depth_tile(&[], &rgb_tile, 2).is_err());
}

#[test]
fn public_tile_encode_decode_cover_families_indices_and_wrong_plan() {
    let rgba = vec![128; 16];
    for (family, channels, raw_len) in [
        (OnnxModelFamily::GenericRgb, 3, 12),
        (OnnxModelFamily::GenericYcbcr, 1, 4),
        (OnnxModelFamily::SuperResolutionYcbcr, 1, 4),
        (OnnxModelFamily::DepthEstimation, 3, 4),
        (OnnxModelFamily::BackgroundRemoval, 3, 4),
    ] {
        let current = plan(family, channels);
        let encoded = encode_browser_onnx_input(
            &rgba,
            BrowserOnnxTensorRequest {
                plan: current.clone(),
                tile_index: Some(0),
            },
        )
        .unwrap();
        assert!(!encoded.tensor.is_empty());
        let decoded = decode_browser_onnx_output(
            &rgba,
            &vec![0.5; raw_len],
            BrowserOnnxOutputRequest {
                plan: current,
                tile_index: Some(0),
                output_shape: vec![],
            },
        )
        .unwrap();
        assert_eq!(decoded.rgba.unwrap().len(), 16);
    }
    for (family, channels) in [
        (OnnxModelFamily::GenericRgb, 3),
        (OnnxModelFamily::GenericYcbcr, 1),
        (OnnxModelFamily::BackgroundRemoval, 3),
        (OnnxModelFamily::DepthEstimation, 3),
    ] {
        assert!(decode_browser_onnx_output(
            &rgba,
            &[],
            BrowserOnnxOutputRequest {
                plan: plan(family, channels),
                tile_index: Some(0),
                output_shape: vec![],
            }
        )
        .is_err());
    }
    let tiled = plan(OnnxModelFamily::GenericRgb, 3);
    assert!(encode_browser_onnx_input(
        &[],
        BrowserOnnxTensorRequest {
            plan: tiled.clone(),
            tile_index: Some(0)
        }
    )
    .is_err());
    assert!(decode_browser_onnx_output(
        &[],
        &[0.0; 12],
        BrowserOnnxOutputRequest {
            plan: tiled,
            tile_index: Some(0),
            output_shape: vec![]
        }
    )
    .is_err());
    let mut invalid_depth_tile = tile(3);
    invalid_depth_tile.patch_width = 0;
    invalid_depth_tile.patch_height = 0;
    invalid_depth_tile.input.shape = vec![1, 3, 0, 0];
    assert!(encode_depth_tile(&rgba, 2, &invalid_depth_tile).is_err());
    let mut invalid_depth_plan = plan(OnnxModelFamily::DepthEstimation, 3);
    let OnnxExecutionPlan::Tiled { tiles, .. } = &mut invalid_depth_plan.execution else {
        unreachable!()
    };
    tiles[0] = invalid_depth_tile;
    assert!(encode_browser_onnx_input(
        &rgba,
        BrowserOnnxTensorRequest {
            plan: invalid_depth_plan,
            tile_index: Some(0)
        }
    )
    .is_err());
    let tiled = plan(OnnxModelFamily::GenericRgb, 3);
    assert!(encode_browser_onnx_input(
        &rgba,
        BrowserOnnxTensorRequest {
            plan: tiled.clone(),
            tile_index: None
        }
    )
    .is_err());
    assert!(encode_browser_onnx_input(
        &rgba,
        BrowserOnnxTensorRequest {
            plan: tiled.clone(),
            tile_index: Some(9)
        }
    )
    .is_err());
    assert!(decode_browser_onnx_output(
        &rgba,
        &[0.0; 12],
        BrowserOnnxOutputRequest {
            plan: tiled.clone(),
            tile_index: None,
            output_shape: vec![]
        }
    )
    .is_err());
    assert!(decode_browser_onnx_output(
        &rgba,
        &[0.0; 12],
        BrowserOnnxOutputRequest {
            plan: tiled,
            tile_index: Some(9),
            output_shape: vec![]
        }
    )
    .is_err());

    let wrong = BrowserOnnxTaskPlan {
        execution: OnnxExecutionPlan::Single {
            input: tensor_descriptor(3, 2, 2),
            output: tensor_descriptor(3, 2, 2),
        },
        ..plan(OnnxModelFamily::GenericRgb, 3)
    };
    assert!(decode_tile(&rgba, &[0.0; 12], &wrong, &rgb_tile()).is_err());
}

fn rgb_tile() -> TileDescriptor {
    tile(3)
}

#[test]
fn single_codecs_cover_generic_detection_segmentation_and_failures() {
    let rgba = vec![128; 16];
    let descriptor = tensor_descriptor(3, 2, 2);
    let mut generic = plan(OnnxModelFamily::GenericRgb, 3);
    generic.execution = OnnxExecutionPlan::Single {
        input: descriptor.clone(),
        output: descriptor.clone(),
    };
    let encoded = encode_browser_onnx_input(
        &rgba,
        BrowserOnnxTensorRequest {
            plan: generic.clone(),
            tile_index: None,
        },
    )
    .unwrap();
    assert_eq!(encoded.tensor.len(), 12);
    assert!(encode_browser_onnx_input(
        &[],
        BrowserOnnxTensorRequest {
            plan: generic.clone(),
            tile_index: None
        }
    )
    .is_err());
    assert_eq!(
        encode_single(&rgba, &generic, &descriptor)
            .unwrap()
            .tensor
            .len(),
        12
    );
    let decoded = decode_browser_onnx_output(
        &rgba,
        &[0.5; 12],
        BrowserOnnxOutputRequest {
            plan: generic.clone(),
            tile_index: None,
            output_shape: vec![json!(1), json!(3), json!(2), json!(2)],
        },
    )
    .unwrap();
    assert_eq!(decoded.rgba.unwrap().len(), 16);
    assert!(decode_browser_onnx_output(
        &rgba,
        &[],
        BrowserOnnxOutputRequest {
            plan: generic,
            tile_index: None,
            output_shape: vec![]
        }
    )
    .is_err());

    let mut detection = plan(OnnxModelFamily::Detection, 3);
    detection.target_size = Some(2);
    detection.execution = OnnxExecutionPlan::Single {
        input: descriptor.clone(),
        output: descriptor.clone(),
    };
    let encoded = encode_single(&rgba, &detection, &descriptor).unwrap();
    assert!(encode_browser_onnx_input(
        &[],
        BrowserOnnxTensorRequest {
            plan: detection.clone(),
            tile_index: None
        }
    )
    .is_err());
    assert!(encoded.scale.is_some());
    let output = decode_browser_onnx_output(
        &rgba,
        &[],
        BrowserOnnxOutputRequest {
            plan: detection,
            tile_index: None,
            output_shape: vec![],
        },
    )
    .unwrap();
    assert!(output.data.is_some());

    let mut segmentation = plan(OnnxModelFamily::Segmentation, 3);
    segmentation.target_size = Some(2);
    segmentation.execution = OnnxExecutionPlan::Single {
        input: descriptor.clone(),
        output: descriptor,
    };
    assert!(decode_browser_onnx_output(
        &rgba,
        &[0.0; 8],
        BrowserOnnxOutputRequest {
            plan: segmentation,
            tile_index: None,
            output_shape: vec![]
        }
    )
    .is_err());
}
