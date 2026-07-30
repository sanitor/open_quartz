use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detection {
    pub bbox: [f32; 4],
    pub score: f32,
    pub class_id: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentationMask {
    pub class_map: Vec<u8>,
    pub mask_width: usize,
    pub mask_height: usize,
    pub num_classes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentationResult {
    pub mask_rgba: Vec<u8>,
    pub mask_width: usize,
    pub mask_height: usize,
    pub class_counts: Vec<usize>,
    pub num_classes: usize,
}

pub const COCO_CLASSES: [&str; 80] = [
    "person",
    "bicycle",
    "car",
    "motorcycle",
    "airplane",
    "bus",
    "train",
    "truck",
    "boat",
    "traffic light",
    "fire hydrant",
    "stop sign",
    "parking meter",
    "bench",
    "bird",
    "cat",
    "dog",
    "horse",
    "sheep",
    "cow",
    "elephant",
    "bear",
    "zebra",
    "giraffe",
    "backpack",
    "umbrella",
    "handbag",
    "tie",
    "suitcase",
    "frisbee",
    "skis",
    "snowboard",
    "sports ball",
    "kite",
    "baseball bat",
    "baseball glove",
    "skateboard",
    "surfboard",
    "tennis racket",
    "bottle",
    "wine glass",
    "cup",
    "fork",
    "knife",
    "spoon",
    "bowl",
    "banana",
    "apple",
    "sandwich",
    "orange",
    "broccoli",
    "carrot",
    "hot dog",
    "pizza",
    "donut",
    "cake",
    "chair",
    "couch",
    "potted plant",
    "bed",
    "dining table",
    "toilet",
    "tv",
    "laptop",
    "mouse",
    "remote",
    "keyboard",
    "cell phone",
    "microwave",
    "oven",
    "toaster",
    "sink",
    "refrigerator",
    "book",
    "clock",
    "vase",
    "scissors",
    "teddy bear",
    "hair drier",
    "toothbrush",
];

pub const CITYSCAPES_CLASSES: [&str; 19] = [
    "road",
    "sidewalk",
    "building",
    "wall",
    "fence",
    "pole",
    "traffic light",
    "traffic sign",
    "vegetation",
    "terrain",
    "sky",
    "person",
    "rider",
    "car",
    "truck",
    "bus",
    "train",
    "motorcycle",
    "bicycle",
];

pub const CITYSCAPES_PALETTE: [[u8; 4]; 19] = [
    [128, 64, 128, 255],
    [244, 35, 232, 255],
    [70, 70, 70, 255],
    [102, 102, 156, 255],
    [190, 153, 153, 255],
    [153, 153, 153, 255],
    [250, 170, 30, 255],
    [220, 220, 0, 255],
    [107, 142, 35, 255],
    [152, 251, 152, 255],
    [70, 130, 180, 255],
    [220, 20, 60, 255],
    [255, 0, 0, 255],
    [0, 0, 142, 255],
    [0, 0, 70, 255],
    [0, 60, 100, 255],
    [0, 80, 100, 255],
    [0, 0, 230, 255],
    [119, 11, 32, 255],
];

pub fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let x1 = a[0].max(b[0]);
    let y1 = a[1].max(b[1]);
    let x2 = a[2].min(b[2]);
    let y2 = a[3].min(b[3]);
    let intersection = (x2 - x1).max(0.0) * (y2 - y1).max(0.0);
    let area_a = (a[2] - a[0]) * (a[3] - a[1]);
    let area_b = (b[2] - b[0]) * (b[3] - b[1]);
    let union = area_a + area_b - intersection;
    if union <= 0.0 {
        0.0
    } else {
        intersection / union
    }
}

pub fn decode_yolo_output(
    raw: &[f32],
    source_width: u32,
    source_height: u32,
    scale: f32,
    pad_x: f32,
    pad_y: f32,
    score_threshold: f32,
) -> Vec<Detection> {
    const NUM_CLASSES: usize = 80;
    const NUM_BOXES: usize = 8400;
    if raw.len() < (4 + NUM_CLASSES) * NUM_BOXES || scale <= 0.0 {
        return Vec::new();
    }
    let inverse_width = 1.0 / (source_width as f32 * scale);
    let inverse_height = 1.0 / (source_height as f32 * scale);
    let mut detections = Vec::new();
    for index in 0..NUM_BOXES {
        let center_x = raw[index];
        let center_y = raw[NUM_BOXES + index];
        let width = raw[NUM_BOXES * 2 + index];
        let height = raw[NUM_BOXES * 3 + index];
        let mut score = 0.0;
        let mut class_id = 0;
        for class in 0..NUM_CLASSES {
            let candidate = raw[(4 + class) * NUM_BOXES + index];
            if candidate > score {
                score = candidate;
                class_id = class;
            }
        }
        if score < score_threshold {
            continue;
        }
        detections.push(Detection {
            bbox: [
                ((center_x - width / 2.0 - pad_x) * inverse_width).clamp(0.0, 1.0),
                ((center_y - height / 2.0 - pad_y) * inverse_height).clamp(0.0, 1.0),
                ((center_x + width / 2.0 - pad_x) * inverse_width).clamp(0.0, 1.0),
                ((center_y + height / 2.0 - pad_y) * inverse_height).clamp(0.0, 1.0),
            ],
            score,
            class_id,
        });
    }
    detections
}

pub fn nms(detections: &[Detection], iou_threshold: f32) -> Vec<Detection> {
    let mut sorted = detections.to_vec();
    sorted.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut suppressed = vec![false; sorted.len()];
    let mut kept = Vec::new();
    for index in 0..sorted.len() {
        if suppressed[index] {
            continue;
        }
        kept.push(sorted[index].clone());
        for candidate in index + 1..sorted.len() {
            if !suppressed[candidate]
                && iou(&sorted[index].bbox, &sorted[candidate].bbox) > iou_threshold
            {
                suppressed[candidate] = true;
            }
        }
    }
    kept
}

#[allow(clippy::too_many_arguments)]
pub fn detect_postprocess(
    raw: &[f32],
    source_width: u32,
    source_height: u32,
    scale: f32,
    pad_x: f32,
    pad_y: f32,
    score_threshold: f32,
    iou_threshold: f32,
) -> Vec<Detection> {
    nms(
        &decode_yolo_output(
            raw,
            source_width,
            source_height,
            scale,
            pad_x,
            pad_y,
            score_threshold,
        ),
        iou_threshold,
    )
}

pub fn decode_segmentation_output(
    raw: &[f32],
    source_width: usize,
    source_height: usize,
    scale: f32,
    pad_x: f32,
    pad_y: f32,
) -> Result<SegmentationMask, String> {
    const NUM_CLASSES: usize = 19;
    if raw.is_empty() || raw.len() % NUM_CLASSES != 0 {
        return Err("segmentation tensor is not divisible by 19 classes".to_owned());
    }
    let spatial = raw.len() / NUM_CLASSES;
    let output_height = (spatial as f64).sqrt().round() as usize;
    let output_width = spatial / output_height.max(1);
    if output_width * output_height != spatial {
        return Err("segmentation tensor spatial dimensions are not rectangular".to_owned());
    }
    let model_width = source_width as f32 * scale + 2.0 * pad_x;
    let model_height = source_height as f32 * scale + 2.0 * pad_y;
    let crop_x = (pad_x * output_width as f32 / model_width).round() as usize;
    let crop_y = (pad_y * output_height as f32 / model_height).round() as usize;
    let crop_width =
        (source_width as f32 * scale * output_width as f32 / model_width).round() as usize;
    let crop_height =
        (source_height as f32 * scale * output_height as f32 / model_height).round() as usize;
    let mask_width = crop_width.max(1);
    let mask_height = crop_height.max(1);
    if crop_x + mask_width > output_width || crop_y + mask_height > output_height {
        return Err("segmentation crop exceeds tensor bounds".to_owned());
    }
    let mut class_map = vec![0; mask_width * mask_height];
    for y in 0..mask_height {
        for x in 0..mask_width {
            let source_x = x + crop_x;
            let source_y = y + crop_y;
            let mut max_value = f32::NEG_INFINITY;
            let mut max_class = 0;
            for class in 0..NUM_CLASSES {
                let value = raw[class * spatial + source_y * output_width + source_x];
                if value > max_value {
                    max_value = value;
                    max_class = class;
                }
            }
            class_map[y * mask_width + x] = max_class as u8;
        }
    }
    Ok(SegmentationMask {
        class_map,
        mask_width,
        mask_height,
        num_classes: NUM_CLASSES,
    })
}

pub fn resize_mask_nearest(
    mask: &[u8],
    source_width: usize,
    source_height: usize,
    target_width: usize,
    target_height: usize,
) -> Result<Vec<u8>, String> {
    if source_width == 0 || source_height == 0 || mask.len() != source_width * source_height {
        return Err("mask dimensions do not match input length".to_owned());
    }
    let mut output = vec![0; target_width * target_height];
    for y in 0..target_height {
        let source_y = (y * source_height / target_height.max(1)).min(source_height - 1);
        for x in 0..target_width {
            let source_x = (x * source_width / target_width.max(1)).min(source_width - 1);
            output[y * target_width + x] = mask[source_y * source_width + source_x];
        }
    }
    Ok(output)
}

pub fn mask_to_rgba(class_map: &[u8], palette: &[[u8; 4]]) -> Vec<u8> {
    let mut rgba = vec![0; class_map.len() * 4];
    for (index, class) in class_map.iter().copied().enumerate() {
        let color = palette
            .get(class as usize)
            .copied()
            .unwrap_or([0, 0, 0, 255]);
        rgba[index * 4..index * 4 + 4].copy_from_slice(&color);
    }
    rgba
}

pub fn segment_postprocess(
    raw: &[f32],
    source_width: usize,
    source_height: usize,
    scale: f32,
    pad_x: f32,
    pad_y: f32,
) -> Result<SegmentationResult, String> {
    let decoded =
        decode_segmentation_output(raw, source_width, source_height, scale, pad_x, pad_y)?;
    let resized = resize_mask_nearest(
        &decoded.class_map,
        decoded.mask_width,
        decoded.mask_height,
        source_width,
        source_height,
    )?;
    let mut class_counts = vec![0; decoded.num_classes];
    for class in &resized {
        if let Some(count) = class_counts.get_mut(*class as usize) {
            *count += 1;
        }
    }
    Ok(SegmentationResult {
        mask_rgba: mask_to_rgba(&resized, &CITYSCAPES_PALETTE),
        mask_width: source_width,
        mask_height: source_height,
        class_counts,
        num_classes: decoded.num_classes,
    })
}
