use open_quartz::onnx::{
    apply_alpha_mask, decode_segmentation_output, decode_yolo_output, iou, letterbox_preprocess,
    mask_to_rgba, nms, resize_mask_nearest, rgba_to_chw, segment_postprocess, Detection,
    CITYSCAPES_PALETTE, COCO_CLASSES,
};

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
