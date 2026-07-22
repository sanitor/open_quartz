/**
 * Pure TypeScript YOLO detection post-processing.
 *
 * Replaces the Rust WASM `rimeflow-yolov8n::postprocess` module.
 * Decodes raw YOLOv8 output tensor, applies letterbox inverse mapping,
 * score filtering, and Non-Maximum Suppression (NMS).
 */

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface Detection {
  bbox: [number, number, number, number]; // x1, y1, x2, y2 normalized 0..1
  score: number;
  classId: number;
}

export const COCO_CLASSES: readonly string[] = [
  'person', 'bicycle', 'car', 'motorcycle', 'airplane', 'bus', 'train', 'truck',
  'boat', 'traffic light', 'fire hydrant', 'stop sign', 'parking meter', 'bench',
  'bird', 'cat', 'dog', 'horse', 'sheep', 'cow', 'elephant', 'bear', 'zebra',
  'giraffe', 'backpack', 'umbrella', 'handbag', 'tie', 'suitcase', 'frisbee',
  'skis', 'snowboard', 'sports ball', 'kite', 'baseball bat', 'baseball glove',
  'skateboard', 'surfboard', 'tennis racket', 'bottle', 'wine glass', 'cup',
  'fork', 'knife', 'spoon', 'bowl', 'banana', 'apple', 'sandwich', 'orange',
  'broccoli', 'carrot', 'hot dog', 'pizza', 'donut', 'cake', 'chair', 'couch',
  'potted plant', 'bed', 'dining table', 'toilet', 'tv', 'laptop', 'mouse',
  'remote', 'keyboard', 'cell phone', 'microwave', 'oven', 'toaster', 'sink',
  'refrigerator', 'book', 'clock', 'vase', 'scissors', 'teddy bear', 'hair drier',
  'toothbrush',
];

// ---------------------------------------------------------------------------
// IoU
// ---------------------------------------------------------------------------

/** Intersection-over-Union for two [x1, y1, x2, y2] boxes. */
export function iou(
  a: readonly [number, number, number, number],
  b: readonly [number, number, number, number],
): number {
  const x1 = Math.max(a[0], b[0]);
  const y1 = Math.max(a[1], b[1]);
  const x2 = Math.min(a[2], b[2]);
  const y2 = Math.min(a[3], b[3]);
  const inter = Math.max(0, x2 - x1) * Math.max(0, y2 - y1);
  const areaA = (a[2] - a[0]) * (a[3] - a[1]);
  const areaB = (b[2] - b[0]) * (b[3] - b[1]);
  const union = areaA + areaB - inter;
  return union <= 0 ? 0 : inter / union;
}

// ---------------------------------------------------------------------------
// Decode
// ---------------------------------------------------------------------------

/**
 * Decode raw YOLOv8 output tensor into Detection objects.
 *
 * @param raw       Float32 tensor data, shape [1, 84, 8400] flattened.
 *                  84 = 4 (cx, cy, w, h) + 80 (class scores).
 * @param srcW      Original source image width (before letterbox).
 * @param srcH      Original source image height (before letterbox).
 * @param scale     Letterbox scale factor.
 * @param padX      Letterbox horizontal padding (pixels in model space).
 * @param padY      Letterbox vertical padding (pixels in model space).
 * @param scoreThreshold  Minimum score to keep (default 0.25).
 */
export function decodeYoloOutput(
  raw: Float32Array,
  srcW: number,
  srcH: number,
  scale: number,
  padX: number,
  padY: number,
  scoreThreshold = 0.25,
): Detection[] {
  const numClasses = 80;
  const numBoxes = 8400;
  const expected = (4 + numClasses) * numBoxes;
  if (raw.length < expected) return [];

  const detections: Detection[] = [];
  const invScaleW = 1 / (srcW * scale);
  const invScaleH = 1 / (srcH * scale);

  for (let i = 0; i < numBoxes; i++) {
    const cx = raw[0 * numBoxes + i];
    const cy = raw[1 * numBoxes + i];
    const w = raw[2 * numBoxes + i];
    const h = raw[3 * numBoxes + i];

    // Find max class score
    let maxScore = 0;
    let maxClass = 0;
    for (let c = 0; c < numClasses; c++) {
      const score = raw[(4 + c) * numBoxes + i];
      if (score > maxScore) {
        maxScore = score;
        maxClass = c;
      }
    }

    if (maxScore < scoreThreshold) continue;

    // Letterbox inverse: model coords → normalized source coords
    const x1 = Math.max(0, Math.min(1, ((cx - w / 2) - padX) * invScaleW));
    const y1 = Math.max(0, Math.min(1, ((cy - h / 2) - padY) * invScaleH));
    const x2 = Math.max(0, Math.min(1, ((cx + w / 2) - padX) * invScaleW));
    const y2 = Math.max(0, Math.min(1, ((cy + h / 2) - padY) * invScaleH));

    detections.push({ bbox: [x1, y1, x2, y2], score: maxScore, classId: maxClass });
  }

  return detections;
}

// ---------------------------------------------------------------------------
// NMS
// ---------------------------------------------------------------------------

/**
 * Non-Maximum Suppression — remove overlapping detections.
 *
 * @param detections  Input detections (will not be mutated).
 * @param iouThreshold  IoU threshold above which a lower-score box is suppressed.
 */
export function nms(detections: readonly Detection[], iouThreshold: number): Detection[] {
  // Sort by score descending
  const sorted = detections.slice().sort((a, b) => b.score - a.score);
  const suppressed = new Uint8Array(sorted.length); // 0 = keep, 1 = suppressed
  const keep: Detection[] = [];

  for (let i = 0; i < sorted.length; i++) {
    if (suppressed[i]) continue;
    keep.push(sorted[i]);
    for (let j = i + 1; j < sorted.length; j++) {
      if (suppressed[j]) continue;
      if (iou(sorted[i].bbox, sorted[j].bbox) > iouThreshold) {
        suppressed[j] = 1;
      }
    }
  }

  return keep;
}

/**
 * Full detection pipeline: decode + NMS.
 */
export function detectPostprocess(
  raw: Float32Array,
  srcW: number,
  srcH: number,
  scale: number,
  padX: number,
  padY: number,
  scoreThreshold = 0.25,
  iouThreshold = 0.45,
): Detection[] {
  const decoded = decodeYoloOutput(raw, srcW, srcH, scale, padX, padY, scoreThreshold);
  return nms(decoded, iouThreshold);
}
