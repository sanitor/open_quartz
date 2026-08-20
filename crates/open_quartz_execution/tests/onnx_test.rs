use open_quartz_execution::onnx::{
    apply_alpha_mask, decode_segmentation_output, decode_yolo_output, iou, letterbox_preprocess,
    mask_to_rgba, nms, plan_browser_onnx_task, resize_mask_nearest, rgba_to_chw,
    segment_postprocess, BrowserOnnxCompletionRequest, BrowserOnnxOutputRequest,
    BrowserOnnxTaskPlanRequest, BrowserOnnxTensorRequest, Detection, OnnxExecutionPlan,
    OnnxModelFamily, OnnxTask, CITYSCAPES_PALETTE, COCO_CLASSES,
};
use open_quartz_execution::onnx::{build_browser_onnx_completion, decode_browser_onnx_output, encode_browser_onnx_input};
use serde_json::json;

#[test]
fn rgba_preprocessing_produces_normalized_chw() {
    let tensor = rgba_to_chw(&[255, 128, 0, 255], 1, 1).unwrap();
    assert_eq!(tensor, [1.0, 128.0 / 255.0, 0.0]);

    let letterbox = letterbox_preprocess(&[255, 0, 0, 255, 0, 255, 0, 255], 2, 1, 2).unwrap();
    assert_eq!(letterbox.shape, [1, 3, 2, 2]);
    assert_eq!((letterbox.resized_width, letterbox.resized_height), (2, 1));
    assert_eq!(letterbox.scale, 1.0);
}

#[test]
fn rgba_output_and_alpha_mask_validate_dimensions() {
    let source = [10, 20, 30, 255, 40, 50, 60, 255];
    let masked = apply_alpha_mask(&source, &[0.0, 1.0], 2, 1).unwrap();
    assert_eq!(masked, [10, 20, 30, 0, 40, 50, 60, 255]);
    assert!(apply_alpha_mask(&source, &[1.0], 2, 1).is_err());
}

#[test]
fn detection_iou_decode_and_nms_match_typescript_contract() {
    assert!((iou(&[0.0, 0.0, 1.0, 1.0], &[0.5, 0.5, 1.0, 1.0]) - 0.25).abs() < 1e-6);
    const BOXES: usize = 8400;
    let mut raw = vec![0.0; 84 * BOXES];
    raw[0] = 320.0;
    raw[BOXES] = 320.0;
    raw[BOXES * 2] = 320.0;
    raw[BOXES * 3] = 160.0;
    raw[BOXES * 4] = 0.9;
    let decoded = decode_yolo_output(&raw, 640, 640, 1.0, 0.0, 0.0, 0.25);
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].class_id, 0);
    assert!((decoded[0].bbox[0] - 0.25).abs() < 1e-6);
    assert_eq!(COCO_CLASSES.len(), 80);

    let overlapping = vec![
        Detection {
            bbox: [0.0, 0.0, 1.0, 1.0],
            score: 0.9,
            class_id: 0,
        },
        Detection {
            bbox: [0.0, 0.0, 1.0, 1.0],
            score: 0.8,
            class_id: 1,
        },
    ];
    assert_eq!(nms(&overlapping, 0.45), vec![overlapping[0].clone()]);
}

#[test]
fn segmentation_decodes_resizes_and_colorizes() {
    let mut raw = vec![0.0; 19 * 4];
    for (pixel, class) in [0, 1, 2, 3].into_iter().enumerate() {
        raw[class * 4 + pixel] = 1.0;
    }
    let decoded = decode_segmentation_output(&raw, 2, 2, 1.0, 0.0, 0.0).unwrap();
    assert_eq!(decoded.class_map, [0, 1, 2, 3]);
    let resized = resize_mask_nearest(&decoded.class_map, 2, 2, 4, 4).unwrap();
    assert_eq!(resized.len(), 16);
    let rgba = mask_to_rgba(&[0, 1], &CITYSCAPES_PALETTE);
    assert_eq!(&rgba[0..4], &CITYSCAPES_PALETTE[0]);
    assert_eq!(&rgba[4..8], &CITYSCAPES_PALETTE[1]);

    let result = segment_postprocess(&raw, 2, 2, 1.0, 0.0, 0.0).unwrap();
    assert_eq!(result.mask_rgba.len(), 16);
    assert_eq!(result.class_counts.iter().sum::<usize>(), 4);
}

#[test]
fn browser_task_plan_freezes_model_family_dispatch_and_defaults() {
    let cases = [
        ("super-resolution-3x", OnnxTask::SuperResolution, OnnxModelFamily::SuperResolutionYcbcr),
        ("realesrgan-x4", OnnxTask::SuperResolution, OnnxModelFamily::SuperResolutionRgb),
        ("u2netp", OnnxTask::BackgroundRemoval, OnnxModelFamily::BackgroundRemoval),
        ("modnet", OnnxTask::BackgroundRemoval, OnnxModelFamily::BackgroundRemoval),
        ("midas-small", OnnxTask::DepthEstimation, OnnxModelFamily::DepthEstimation),
        ("yolov8n", OnnxTask::Detection, OnnxModelFamily::Detection),
        ("yolo26n-sem", OnnxTask::Segmentation, OnnxModelFamily::Segmentation),
    ];
    for (model_id, task, family) in cases {
        let plan = plan_browser_onnx_task(BrowserOnnxTaskPlanRequest {
            model_id: Some(model_id.to_owned()),
            task: Some(task),
            source_width: 32,
            source_height: 16,
            target_size: None,
            tile_size: None,
            params: json!({}),
            input_shape: Vec::new(),
            output_shape: Vec::new(),
        })
        .unwrap();
        assert_eq!(plan.family, family, "{model_id}");
    }

    let detection = plan_browser_onnx_task(BrowserOnnxTaskPlanRequest {
        model_id: Some("yolov8n".to_owned()),
        task: Some(OnnxTask::Detection),
        source_width: 320,
        source_height: 240,
        target_size: None,
        tile_size: None,
        params: json!({ "scoreThreshold": 0.35, "iouThreshold": 0.55 }),
        input_shape: Vec::new(),
        output_shape: Vec::new(),
    })
    .unwrap();
    assert_eq!(detection.target_size, Some(640));
    assert_eq!(detection.thresholds.unwrap().score, 0.35);
    assert_eq!(detection.class_labels.unwrap()[0], "person");
    match detection.execution {
        OnnxExecutionPlan::Single { input, .. } => assert_eq!(input.shape, [1, 3, 640, 640]),
        _ => panic!("detection must use a single letterbox tensor"),
    }
}

#[test]
fn browser_task_plan_freezes_tile_tensor_descriptors() {
    let fixed = plan_browser_onnx_task(BrowserOnnxTaskPlanRequest {
        model_id: Some("super-resolution-3x".to_owned()),
        task: Some(OnnxTask::SuperResolution),
        source_width: 300,
        source_height: 210,
        target_size: None,
        tile_size: None,
        params: json!({}),
        input_shape: Vec::new(),
        output_shape: Vec::new(),
    })
    .unwrap();
    match fixed.execution {
        OnnxExecutionPlan::Tiled { scale, fixed_size, tile_size, tiles, .. } => {
            assert_eq!(scale, 3);
            assert_eq!(fixed_size, Some(224));
            assert_eq!(tile_size, 208);
            assert_eq!(tiles[0].input.shape, [1, 1, 224, 224]);
        }
        _ => panic!("super-resolution-3x must use fixed tiled tensors"),
    }

    let generic = plan_browser_onnx_task(BrowserOnnxTaskPlanRequest {
        model_id: Some("custom".to_owned()),
        task: Some(OnnxTask::Generic),
        source_width: 16,
        source_height: 16,
        target_size: None,
        tile_size: Some(8),
        params: json!({}),
        input_shape: vec![json!(1), json!(3), json!("h"), json!("w")],
        output_shape: vec![json!(1), json!(3), json!("h"), json!("w")],
    })
    .unwrap();
    match generic.execution {
        OnnxExecutionPlan::Tiled { scale, channels, fixed_size, tiles, .. } => {
            assert_eq!((scale, channels, fixed_size), (1, 3, None));
            assert_eq!(tiles.len(), 4);
            assert_eq!(tiles[0].input.shape, [1, 3, 16, 16]);
        }
        _ => panic!("generic RGB must use dynamic tiled tensors"),
    }
}

#[test]
fn browser_rust_preprocess_and_postprocess_tile_boundaries() {
    let plan = plan_browser_onnx_task(BrowserOnnxTaskPlanRequest {
        model_id: Some("realesrgan-x4".to_owned()),
        task: Some(OnnxTask::SuperResolution),
        source_width: 2,
        source_height: 1,
        target_size: None,
        tile_size: Some(2),
        params: json!({}),
        input_shape: Vec::new(),
        output_shape: Vec::new(),
    })
    .unwrap();
    let rgba = [255, 0, 0, 255, 0, 128, 255, 255];
    let tensor = encode_browser_onnx_input(
        &rgba,
        BrowserOnnxTensorRequest {
            plan: plan.clone(),
            tile_index: Some(0),
        },
    )
    .unwrap();
    assert_eq!(tensor.descriptor.shape, [1, 3, 1, 2]);
    assert_eq!(tensor.tensor, [1.0, 0.0, 0.0, 128.0 / 255.0, 0.0, 1.0]);

    let raw = vec![1.0; 3 * 8 * 4];
    let decoded = decode_browser_onnx_output(
        &rgba,
        &raw,
        BrowserOnnxOutputRequest {
            plan,
            tile_index: Some(0),
            output_shape: Vec::new(),
        },
    )
    .unwrap();
    assert_eq!((decoded.width, decoded.height, decoded.dst_x, decoded.dst_y), (8, 4, Some(0), Some(0)));
    assert_eq!(decoded.rgba.unwrap()[0..4], [255, 255, 255, 255]);
}

#[test]
fn browser_detection_and_segmentation_use_rust_output_mapping() {
    let detection_plan = plan_browser_onnx_task(BrowserOnnxTaskPlanRequest {
        model_id: Some("yolov8n".to_owned()),
        task: Some(OnnxTask::Detection),
        source_width: 640,
        source_height: 640,
        target_size: Some(640),
        tile_size: None,
        params: json!({}),
        input_shape: Vec::new(),
        output_shape: Vec::new(),
    })
    .unwrap();
    let mut raw = vec![0.0; 84 * 8400];
    raw[0] = 320.0;
    raw[8400] = 320.0;
    raw[8400 * 2] = 320.0;
    raw[8400 * 3] = 160.0;
    raw[8400 * 4] = 0.9;
    let decoded = decode_browser_onnx_output(
        &[0; 640 * 640 * 4],
        &raw,
        BrowserOnnxOutputRequest {
            plan: detection_plan,
            tile_index: None,
            output_shape: Vec::new(),
        },
    )
    .unwrap();
    assert_eq!(decoded.data.unwrap()[0]["classId"], 0);

    let segmentation_plan = plan_browser_onnx_task(BrowserOnnxTaskPlanRequest {
        model_id: Some("yolo26n-sem".to_owned()),
        task: Some(OnnxTask::Segmentation),
        source_width: 2,
        source_height: 2,
        target_size: Some(2),
        tile_size: None,
        params: json!({}),
        input_shape: Vec::new(),
        output_shape: Vec::new(),
    })
    .unwrap();
    let mut mask = vec![0.0; 19 * 4];
    mask[1] = 2.0;
    let segmentation = decode_browser_onnx_output(
        &[0; 16],
        &mask,
        BrowserOnnxOutputRequest {
            plan: segmentation_plan,
            tile_index: None,
            output_shape: Vec::new(),
        },
    )
    .unwrap();
    assert_eq!((segmentation.width, segmentation.height), (2, 2));
    assert_eq!(segmentation.rgba.unwrap().len(), 16);
}

#[test]
fn browser_completion_helper_preserves_launch_stamp_and_payload_policy() {
    let completion = build_browser_onnx_completion(BrowserOnnxCompletionRequest {
        node_id: "onnx".to_owned(),
        graph_revision: 7,
        node_generation: 3,
        input_stamp: open_quartz_execution::runtime::FrameStamp {
            epoch: 2,
            frame: 11,
            timeline_ns: 123,
            deadline_ns: 456,
        },
        data: json!([{ "bbox": [0.0, 0.0, 1.0, 1.0] }]),
        outputs: vec![
            open_quartz_execution::onnx::BrowserOnnxCompletionPort {
                id: "overlay".to_owned(),
                data_type: open_quartz_schema::DataType::Sampler2d,
            },
            open_quartz_execution::onnx::BrowserOnnxCompletionPort {
                id: "detections".to_owned(),
                data_type: open_quartz_schema::DataType::Roi,
            },
        ],
    });
    assert_eq!(completion.graph_revision, 7);
    assert_eq!(completion.input_stamp.timeline_ns, 123);
    assert_eq!(completion.content_stamp.timeline_ns, 123);
    assert!(matches!(completion.outputs[0].1, open_quartz_execution::runtime::OutputPayload::Resource { handle: 3 }));
    assert!(matches!(completion.outputs[1].1, open_quartz_execution::runtime::OutputPayload::Json(_)));
}

#[test]
fn segmentation_rejects_invalid_and_boundary_tensors() {
    assert!(decode_segmentation_output(&[0.0; 18], 1, 1, 1.0, 0.0, 0.0).is_err());
    assert!(resize_mask_nearest(&[0, 1, 2], 2, 2, 2, 2).is_err());
    assert!(letterbox_preprocess(&[0; 4], 1, 1, 0).is_err());
    assert!(rgba_to_chw(&[0; 3], 1, 1).is_err());
}
