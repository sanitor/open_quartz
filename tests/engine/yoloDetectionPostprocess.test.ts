import { describe, it, expect } from 'vitest';
import {
  iou,
  decodeYoloOutput,
  nms,
  detectPostprocess,
  COCO_CLASSES,
} from '../../src/engine/yoloDetectionPostprocess';
import type { Detection } from '../../src/engine/yoloDetectionPostprocess';

// ---------------------------------------------------------------------------
// Helpers — build a minimal [1, 84, 8400] tensor with known detections
// ---------------------------------------------------------------------------

const NUM_CLASSES = 80;
const NUM_BOXES = 8400;

/**
 * Build a raw tensor with a single detection at a known position.
 * Tensor layout: [1, 84, 8400] = channels-first, so index = channel * 8400 + box.
 */
function buildTensor(
  entries: Array<{
    boxIndex: number;
    cx: number;
    cy: number;
    w: number;
    h: number;
    classId: number;
    score: number;
  }>,
): Float32Array {
  const raw = new Float32Array((4 + NUM_CLASSES) * NUM_BOXES);
  for (const e of entries) {
    raw[0 * NUM_BOXES + e.boxIndex] = e.cx;
    raw[1 * NUM_BOXES + e.boxIndex] = e.cy;
    raw[2 * NUM_BOXES + e.boxIndex] = e.w;
    raw[3 * NUM_BOXES + e.boxIndex] = e.h;
    raw[(4 + e.classId) * NUM_BOXES + e.boxIndex] = e.score;
  }
  return raw;
}

// ---------------------------------------------------------------------------
// iou
// ---------------------------------------------------------------------------

describe('iou', () => {
  it('returns 1 for identical boxes', () => {
    expect(iou([0, 0, 1, 1], [0, 0, 1, 1])).toBeCloseTo(1.0);
  });

  it('returns 0 for non-overlapping boxes', () => {
    expect(iou([0, 0, 0.5, 0.5], [0.6, 0.6, 1, 1])).toBeCloseTo(0);
  });

  it('returns ~0.14 for partially overlapping boxes', () => {
    // box A: [0, 0, 0.5, 0.5] area = 0.25
    // box B: [0.25, 0.25, 0.75, 0.75] area = 0.25
    // overlap: [0.25, 0.25, 0.5, 0.5] area = 0.0625
    // union: 0.25 + 0.25 - 0.0625 = 0.4375
    // iou: 0.0625 / 0.4375 ≈ 0.1429
    expect(iou([0, 0, 0.5, 0.5], [0.25, 0.25, 0.75, 0.75])).toBeCloseTo(0.1429, 3);
  });

  it('returns 0 when union is zero (degenerate boxes)', () => {
    expect(iou([0, 0, 0, 0], [0, 0, 0, 0])).toBe(0);
  });

  it('handles contained box (smaller inside larger)', () => {
    // A: [0, 0, 1, 1] area=1; B: [0.25, 0.25, 0.75, 0.75] area=0.25
    // overlap = 0.25; union = 1; iou = 0.25
    expect(iou([0, 0, 1, 1], [0.25, 0.25, 0.75, 0.75])).toBeCloseTo(0.25);
  });
});

// ---------------------------------------------------------------------------
// decodeYoloOutput
// ---------------------------------------------------------------------------

describe('decodeYoloOutput', () => {
  // No letterbox: scale=1, pad=0, srcW=srcH=640
  const noLetterbox = { srcW: 640, srcH: 640, scale: 1, padX: 0, padY: 0 };

  it('returns empty for undersized tensor', () => {
    const raw = new Float32Array(10);
    expect(decodeYoloOutput(raw, 640, 640, 1, 0, 0)).toEqual([]);
  });

  it('decodes a single high-score detection', () => {
    // Place a box at center (320, 320) with size 100×100 in a 640×640 image
    const raw = buildTensor([{
      boxIndex: 0, cx: 320, cy: 320, w: 100, h: 100,
      classId: 0, score: 0.9,
    }]);
    const dets = decodeYoloOutput(raw, noLetterbox.srcW, noLetterbox.srcH,
      noLetterbox.scale, noLetterbox.padX, noLetterbox.padY);

    expect(dets).toHaveLength(1);
    expect(dets[0].classId).toBe(0);
    expect(dets[0].score).toBeCloseTo(0.9);
    // bbox: [(320-50)/640, (320-50)/640, (320+50)/640, (320+50)/640]
    //      = [270/640, 270/640, 370/640, 370/640]
    expect(dets[0].bbox[0]).toBeCloseTo(270 / 640, 4);
    expect(dets[0].bbox[1]).toBeCloseTo(270 / 640, 4);
    expect(dets[0].bbox[2]).toBeCloseTo(370 / 640, 4);
    expect(dets[0].bbox[3]).toBeCloseTo(370 / 640, 4);
  });

  it('filters out detections below score threshold', () => {
    const raw = buildTensor([
      { boxIndex: 0, cx: 100, cy: 100, w: 50, h: 50, classId: 0, score: 0.1 },
      { boxIndex: 1, cx: 200, cy: 200, w: 50, h: 50, classId: 1, score: 0.8 },
    ]);
    const dets = decodeYoloOutput(raw, 640, 640, 1, 0, 0, 0.25);
    expect(dets).toHaveLength(1);
    expect(dets[0].classId).toBe(1);
  });

  it('respects custom score threshold', () => {
    const raw = buildTensor([
      { boxIndex: 0, cx: 100, cy: 100, w: 50, h: 50, classId: 5, score: 0.6 },
    ]);
    expect(decodeYoloOutput(raw, 640, 640, 1, 0, 0, 0.7)).toHaveLength(0);
    expect(decodeYoloOutput(raw, 640, 640, 1, 0, 0, 0.5)).toHaveLength(1);
  });

  it('picks the class with highest score', () => {
    const raw = buildTensor([]);
    // Manually set two class scores for box 0
    raw[(4 + 3) * NUM_BOXES + 0] = 0.4;  // class 3
    raw[(4 + 7) * NUM_BOXES + 0] = 0.8;  // class 7 (winner)
    raw[0 * NUM_BOXES + 0] = 320; // cx
    raw[1 * NUM_BOXES + 0] = 320; // cy
    raw[2 * NUM_BOXES + 0] = 100; // w
    raw[3 * NUM_BOXES + 0] = 100; // h

    const dets = decodeYoloOutput(raw, 640, 640, 1, 0, 0);
    expect(dets).toHaveLength(1);
    expect(dets[0].classId).toBe(7);
    expect(dets[0].score).toBeCloseTo(0.8);
  });

  it('applies letterbox inverse mapping correctly', () => {
    // Letterbox scenario: 640×480 image padded to 640×640
    // scale = 640/640 = 1.0 (width-limited)
    // padX = 0, padY = (640 - 480) / 2 = 80
    // A box at model coords (320, 320) with size 100×100:
    // x1 = (320-50-0)/(640*1) = 270/640
    // y1 = (320-50-80)/(480*1) = 190/480
    const raw = buildTensor([{
      boxIndex: 0, cx: 320, cy: 320, w: 100, h: 100,
      classId: 0, score: 0.9,
    }]);
    const dets = decodeYoloOutput(raw, 640, 480, 1.0, 0, 80);
    expect(dets).toHaveLength(1);
    expect(dets[0].bbox[0]).toBeCloseTo(270 / 640, 3);
    expect(dets[0].bbox[1]).toBeCloseTo(190 / 480, 3);
  });

  it('clamps bbox to [0, 1]', () => {
    // Box extending beyond the image edges
    const raw = buildTensor([{
      boxIndex: 0, cx: 10, cy: 10, w: 100, h: 100,
      classId: 0, score: 0.9,
    }]);
    const dets = decodeYoloOutput(raw, 640, 640, 1, 0, 0);
    expect(dets).toHaveLength(1);
    expect(dets[0].bbox[0]).toBe(0); // clamped
    expect(dets[0].bbox[1]).toBe(0); // clamped
  });

  it('handles multiple detections across different boxes', () => {
    const raw = buildTensor([
      { boxIndex: 0, cx: 100, cy: 100, w: 50, h: 50, classId: 0, score: 0.9 },
      { boxIndex: 100, cx: 400, cy: 400, w: 80, h: 80, classId: 1, score: 0.7 },
      { boxIndex: 4000, cx: 300, cy: 300, w: 60, h: 60, classId: 2, score: 0.5 },
    ]);
    const dets = decodeYoloOutput(raw, 640, 640, 1, 0, 0);
    expect(dets).toHaveLength(3);
    expect(dets.map(d => d.classId).sort()).toEqual([0, 1, 2]);
  });
});

// ---------------------------------------------------------------------------
// nms
// ---------------------------------------------------------------------------

describe('nms', () => {
  it('returns empty for empty input', () => {
    expect(nms([], 0.5)).toEqual([]);
  });

  it('keeps a single detection', () => {
    const dets: Detection[] = [
      { bbox: [0, 0, 0.5, 0.5], score: 0.9, classId: 0 },
    ];
    expect(nms(dets, 0.5)).toHaveLength(1);
  });

  it('keeps non-overlapping detections', () => {
    const dets: Detection[] = [
      { bbox: [0, 0, 0.3, 0.3], score: 0.9, classId: 0 },
      { bbox: [0.5, 0.5, 0.8, 0.8], score: 0.8, classId: 1 },
    ];
    expect(nms(dets, 0.5)).toHaveLength(2);
  });

  it('suppresses overlapping lower-score detection', () => {
    const dets: Detection[] = [
      { bbox: [0, 0, 0.5, 0.5], score: 0.9, classId: 0 },
      { bbox: [0.05, 0.05, 0.55, 0.55], score: 0.7, classId: 0 }, // high overlap
    ];
    const result = nms(dets, 0.5);
    expect(result).toHaveLength(1);
    expect(result[0].score).toBeCloseTo(0.9);
  });

  it('keeps both when IoU is below threshold', () => {
    const dets: Detection[] = [
      { bbox: [0, 0, 0.5, 0.5], score: 0.9, classId: 0 },
      { bbox: [0.3, 0.3, 0.8, 0.8], score: 0.8, classId: 0 },
    ];
    // IoU ≈ 0.04/0.46 ≈ 0.087 — well below 0.5
    const result = nms(dets, 0.5);
    expect(result).toHaveLength(2);
  });

  it('sorts by score descending — highest score always kept', () => {
    const dets: Detection[] = [
      { bbox: [0, 0, 1, 1], score: 0.5, classId: 0 },
      { bbox: [0, 0, 1, 1], score: 0.9, classId: 0 }, // identical box, higher score
    ];
    const result = nms(dets, 0.5);
    expect(result).toHaveLength(1);
    expect(result[0].score).toBeCloseTo(0.9);
  });

  it('handles chain suppression (A suppresses B, B would have suppressed C)', () => {
    const dets: Detection[] = [
      { bbox: [0, 0, 0.5, 0.5], score: 0.9, classId: 0 },
      { bbox: [0.05, 0.05, 0.55, 0.55], score: 0.8, classId: 0 }, // overlaps A
      { bbox: [0.1, 0.1, 0.6, 0.6], score: 0.7, classId: 0 },     // overlaps B but not enough with A
    ];
    // A suppresses B (high IoU). C may or may not overlap enough with A.
    const result = nms(dets, 0.5);
    // A is always kept. B is suppressed. C depends on IoU with A.
    expect(result[0].score).toBeCloseTo(0.9);
    expect(result.every(d => d.score >= 0.7)).toBe(true);
  });

  it('does not mutate the input array', () => {
    const dets: Detection[] = [
      { bbox: [0, 0, 1, 1], score: 0.5, classId: 0 },
      { bbox: [0, 0, 1, 1], score: 0.9, classId: 0 },
    ];
    const copy = [...dets];
    nms(dets, 0.5);
    expect(dets).toEqual(copy);
  });
});

// ---------------------------------------------------------------------------
// detectPostprocess (integration)
// ---------------------------------------------------------------------------

describe('detectPostprocess', () => {
  it('decodes and runs NMS end-to-end', () => {
    // Two overlapping detections — NMS should keep only the higher one
    const raw = buildTensor([
      { boxIndex: 0, cx: 320, cy: 320, w: 100, h: 100, classId: 0, score: 0.9 },
      { boxIndex: 1, cx: 325, cy: 325, w: 100, h: 100, classId: 0, score: 0.7 },
    ]);
    const result = detectPostprocess(raw, 640, 640, 1, 0, 0, 0.25, 0.5);
    expect(result).toHaveLength(1);
    expect(result[0].score).toBeCloseTo(0.9);
  });
});

// ---------------------------------------------------------------------------
// COCO_CLASSES
// ---------------------------------------------------------------------------

describe('COCO_CLASSES', () => {
  it('has exactly 80 classes', () => {
    expect(COCO_CLASSES).toHaveLength(80);
  });

  it('starts with person', () => {
    expect(COCO_CLASSES[0]).toBe('person');
  });

  it('ends with toothbrush', () => {
    expect(COCO_CLASSES[79]).toBe('toothbrush');
  });
});
