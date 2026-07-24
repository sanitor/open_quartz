import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

// ---------------------------------------------------------------------------
// Integration test: detection overlay pipeline
//
// Tests the data-flow contract that WebGPUExecutionEngine.runTaskInference
// depends on — without requiring a real canvas 2D context.
//
// The bugs that caused "all black" YOLO output were:
//   1. Detection uses classId, overlay expects class_id
//   2. drawDetectionOverlay signature: (sourceCanvas, width, height, detections)
//   3. Field name mapping between Detection and OnnxDetection
// ---------------------------------------------------------------------------

import { drawDetectionOverlay } from '../../src/engine/onnx/overlay';
import { detectPostprocess } from '../../src/engine/onnx/yoloDetectionPostprocess';
import { COCO_CLASSES } from '../../src/engine/onnx/yoloDetectionPostprocess';
import type { Detection } from '../../src/engine/onnx/yoloDetectionPostprocess';
import type { OnnxDetection } from '../../src/engine/onnx/overlay';

// ---------------------------------------------------------------------------
// Canvas mock — minimal stub for jsdom
// ---------------------------------------------------------------------------

interface Mock2DContext {
  drawImage: ReturnType<typeof vi.fn>;
  strokeRect: ReturnType<typeof vi.fn>;
  fillRect: ReturnType<typeof vi.fn>;
  fillText: ReturnType<typeof vi.fn>;
  getImageData: ReturnType<typeof vi.fn>;
  putImageData: ReturnType<typeof vi.fn>;
  measureText: ReturnType<typeof vi.fn>;
}

function makeMockCtx(returnRgba: Uint8ClampedArray): Mock2DContext {
  return {
    drawImage: vi.fn(),
    strokeRect: vi.fn(),
    fillRect: vi.fn(),
    fillText: vi.fn(),
    getImageData: vi.fn(() => ({ data: returnRgba })),
    putImageData: vi.fn(),
    measureText: vi.fn(() => ({ width: 50 })),
  };
}

function makeMockCanvas(w: number, h: number, returnRgba: Uint8ClampedArray): HTMLCanvasElement {
  const ctx = makeMockCtx(returnRgba);
  const canvas = {
    width: w,
    height: h,
    getContext: vi.fn(() => ctx),
    toDataURL: vi.fn(() => 'data:image/png;base64,mock'),
  } as unknown as HTMLCanvasElement;
  return canvas;
}

beforeEach(() => {
  // Stub document.createElement('canvas') to return our mock
  vi.spyOn(document, 'createElement').mockImplementation((tag: string) => {
    if (tag === 'canvas') {
      return makeMockCanvas(1, 1, new Uint8ClampedArray(4)) as unknown as HTMLElement;
    }
    return {} as HTMLElement;
  });
});

afterEach(() => {
  vi.restoreAllMocks();
});

// ---------------------------------------------------------------------------
// Helpers — mirror the exact code in runTaskInference
// ---------------------------------------------------------------------------

function makeYoloRawOutput(
  detections: Array<{ cx: number; cy: number; w: number; h: number; classId: number; score: number }>,
  numBoxes = 8400,
): Float32Array {
  const numClasses = 80;
  const data = new Float32Array((4 + numClasses) * numBoxes);
  detections.forEach((det, i) => {
    data[0 * numBoxes + i] = det.cx;
    data[1 * numBoxes + i] = det.cy;
    data[2 * numBoxes + i] = det.w;
    data[3 * numBoxes + i] = det.h;
    data[(4 + det.classId) * numBoxes + i] = det.score;
  });
  return data;
}

/** The exact field mapping that runTaskInference uses. */
function mapDetectionsForOverlay(detections: Detection[]): OnnxDetection[] {
  return detections.map((d) => ({
    bbox: d.bbox,
    score: d.score,
    class_id: d.classId,
    class_name: d.classId < COCO_CLASSES.length ? COCO_CLASSES[d.classId] : `class_${d.classId}`,
  }));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('detection overlay field mapping', () => {
  it('maps Detection.classId → OnnxDetection.class_id correctly', () => {
    const detections: Detection[] = [
      { bbox: [0.1, 0.2, 0.3, 0.4], score: 0.9, classId: 0 },
      { bbox: [0.5, 0.5, 0.8, 0.9], score: 0.7, classId: 15 },
    ];

    const mapped = mapDetectionsForOverlay(detections);

    expect(mapped[0].class_id).toBe(0);
    expect(mapped[0].class_name).toBe('person');
    expect(mapped[1].class_id).toBe(15);
    expect(mapped[1].class_name).toBeDefined();
    // bbox and score pass through unchanged
    expect(mapped[0].bbox).toEqual([0.1, 0.2, 0.3, 0.4]);
    expect(mapped[0].score).toBe(0.9);
  });

  it('detectPostprocess + field mapping produces valid OnnxDetection array', () => {
    const w = 640, h = 640;
    const raw = makeYoloRawOutput([
      { cx: 320, cy: 320, w: 200, h: 300, classId: 0, score: 0.9 },
    ]);

    const detections = detectPostprocess(raw, w, h, 1, 0, 0, 0.25, 0.45);
    expect(detections).toHaveLength(1);

    const mapped = mapDetectionsForOverlay(detections);
    expect(mapped).toHaveLength(1);
    expect(mapped[0].class_id).toBe(0);
    expect(mapped[0].class_name).toBe('person');
    expect(mapped[0].score).toBeGreaterThan(0.25);
    expect(mapped[0].bbox[0]).toBeGreaterThanOrEqual(0);
    expect(mapped[0].bbox[2]).toBeLessThanOrEqual(1);
  });
});

describe('drawDetectionOverlay call contract', () => {
  it('is called with (sourceCanvas, width, height, detections) — correct arg order', () => {
    const srcCanvas = makeMockCanvas(640, 640, new Uint8ClampedArray(640 * 640 * 4));
    const detections: OnnxDetection[] = [
      { bbox: [0.1, 0.1, 0.4, 0.4], score: 0.9, class_id: 0, class_name: 'person' },
    ];

    const result = drawDetectionOverlay(srcCanvas, 640, 640, detections);

    // Verify it returns { canvas, dataUrl }
    expect(result).toHaveProperty('canvas');
    expect(result).toHaveProperty('dataUrl');
  });

  it('accepts empty detections array without error', () => {
    const srcCanvas = makeMockCanvas(640, 640, new Uint8ClampedArray(640 * 640 * 4));

    const result = drawDetectionOverlay(srcCanvas, 640, 640, []);

    expect(result).toHaveProperty('canvas');
  });

  it('detectPostprocess with all-zero raw output returns empty (no crash)', () => {
    const raw = new Float32Array(84 * 8400);
    const detections = detectPostprocess(raw, 640, 640, 1, 0, 0, 0.25, 0.45);
    expect(detections).toHaveLength(0);
  });
});
