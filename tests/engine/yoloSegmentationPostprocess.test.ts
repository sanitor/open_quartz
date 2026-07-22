import { describe, it, expect } from 'vitest';
import {
  decodeSegmentationOutput,
  resizeMaskNearest,
  maskToRgba,
  segmentPostprocess,
  CITYSCAPES_PALETTE,
  CITYSCAPES_CLASSES,
} from '../../src/engine/onnx/yoloSegmentationPostprocess';

// ---------------------------------------------------------------------------
// Helpers — build a [1, C, H, W] logits tensor
// ---------------------------------------------------------------------------

/**
 * Build a flat Float32Array for a [1, numClasses, H, W] tensor (NCHW).
 * `classMap[y][x]` is the winning class for that pixel.
 */
function buildSegTensor(
  numClasses: number,
  height: number,
  width: number,
  classMap: number[][],
): Float32Array {
  const raw = new Float32Array(numClasses * height * width);
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const winClass = classMap[y][x];
      // Set the winning class logit high, others at 0
      raw[winClass * height * width + y * width + x] = 10.0;
    }
  }
  return raw;
}

// ---------------------------------------------------------------------------
// decodeSegmentationOutput
// ---------------------------------------------------------------------------

describe('decodeSegmentationOutput', () => {
  it('performs argmax correctly on a small tensor', () => {
    // 4×4 image, 19 classes, no letterbox (scale=1, pad=0)
    const classMap = [
      [0, 1, 2, 3],
      [4, 5, 6, 7],
      [8, 9, 10, 11],
      [12, 13, 14, 15],
    ];
    const raw = buildSegTensor(19, 4, 4, classMap);
    const result = decodeSegmentationOutput(raw, 4, 4, 1, 0, 0);

    expect(result.numClasses).toBe(19);
    expect(result.maskW).toBe(4);
    expect(result.maskH).toBe(4);
    // Check a few pixels
    expect(result.classMap[0 * 4 + 0]).toBe(0);
    expect(result.classMap[0 * 4 + 3]).toBe(3);
    expect(result.classMap[1 * 4 + 1]).toBe(5);
    expect(result.classMap[3 * 4 + 3]).toBe(15);
  });

  it('returns correct dimensions', () => {
    const classMap = Array.from({ length: 8 }, () => Array.from({ length: 8 }, () => 0));
    const raw = buildSegTensor(19, 8, 8, classMap);
    const result = decodeSegmentationOutput(raw, 8, 8, 1, 0, 0);
    expect(result.maskW).toBe(8);
    expect(result.maskH).toBe(8);
    expect(result.classMap.length).toBe(64);
  });

  it('selects highest logit among competing classes', () => {
    const numClasses = 19;
    const H = 2;
    const W = 2;
    const raw = new Float32Array(numClasses * H * W);
    // Pixel (0,0): class 5 wins with logit 3.0
    raw[5 * H * W + 0 * W + 0] = 3.0;
    raw[3 * H * W + 0 * W + 0] = 1.0;
    // Pixel (1,1): class 18 wins with logit 5.0
    raw[18 * H * W + 1 * W + 1] = 5.0;
    raw[0 * H * W + 1 * W + 1] = 2.0;

    const result = decodeSegmentationOutput(raw, 2, 2, 1, 0, 0);
    expect(result.classMap[0]).toBe(5);
    expect(result.classMap[3]).toBe(18);
  });
});

// ---------------------------------------------------------------------------
// resizeMaskNearest
// ---------------------------------------------------------------------------

describe('resizeMaskNearest', () => {
  it('identity resize (same size)', () => {
    const mask = new Uint8Array([0, 1, 2, 3]);
    const result = resizeMaskNearest(mask, 2, 2, 2, 2);
    expect(result).toEqual(new Uint8Array([0, 1, 2, 3]));
  });

  it('upscales 2×2 to 4×4', () => {
    const mask = new Uint8Array([0, 1, 2, 3]);
    const result = resizeMaskNearest(mask, 2, 2, 4, 4);
    expect(result.length).toBe(16);
    // Top-left quadrant should all be class 0
    expect(result[0]).toBe(0);
    expect(result[1]).toBe(0);
    expect(result[4]).toBe(0);
    expect(result[5]).toBe(0);
    // Top-right quadrant should be class 1
    expect(result[2]).toBe(1);
    expect(result[3]).toBe(1);
    // Bottom-right should be class 3
    expect(result[15]).toBe(3);
  });

  it('downscales 4×4 to 2×2', () => {
    const mask = new Uint8Array([
      0, 0, 1, 1,
      0, 0, 1, 1,
      2, 2, 3, 3,
      2, 2, 3, 3,
    ]);
    const result = resizeMaskNearest(mask, 4, 4, 2, 2);
    expect(result).toEqual(new Uint8Array([0, 1, 2, 3]));
  });

  it('handles non-square resize', () => {
    const mask = new Uint8Array([0, 1, 2, 3, 4, 5]);
    const result = resizeMaskNearest(mask, 3, 2, 6, 4);
    expect(result.length).toBe(24);
    // First row pixel 0 maps to source (0,0) = 0
    expect(result[0]).toBe(0);
  });
});

// ---------------------------------------------------------------------------
// maskToRgba
// ---------------------------------------------------------------------------

describe('maskToRgba', () => {
  it('maps class indices to palette colors', () => {
    const classMap = new Uint8Array([0, 1, 2]);
    const rgba = maskToRgba(classMap, 3, 1);
    // Class 0 = road = [128, 64, 128, 255]
    expect(rgba[0]).toBe(128);
    expect(rgba[1]).toBe(64);
    expect(rgba[2]).toBe(128);
    expect(rgba[3]).toBe(255);
    // Class 1 = sidewalk = [244, 35, 232, 255]
    expect(rgba[4]).toBe(244);
    expect(rgba[5]).toBe(35);
  });

  it('defaults to black for out-of-palette class', () => {
    const classMap = new Uint8Array([255]); // beyond palette
    const rgba = maskToRgba(classMap, 1, 1);
    expect(rgba[0]).toBe(0);
    expect(rgba[1]).toBe(0);
    expect(rgba[2]).toBe(0);
    expect(rgba[3]).toBe(255);
  });

  it('produces correct output length', () => {
    const classMap = new Uint8Array(10 * 10);
    const rgba = maskToRgba(classMap, 10, 10);
    expect(rgba.length).toBe(400);
  });

  it('accepts custom palette', () => {
    const palette: [number, number, number, number][] = [[255, 0, 0, 255]];
    const classMap = new Uint8Array([0]);
    const rgba = maskToRgba(classMap, 1, 1, palette);
    expect(rgba[0]).toBe(255);
    expect(rgba[1]).toBe(0);
    expect(rgba[2]).toBe(0);
  });
});

// ---------------------------------------------------------------------------
// segmentPostprocess (integration)
// ---------------------------------------------------------------------------

describe('segmentPostprocess', () => {
  it('end-to-end: decode + resize + colorize + class counts', () => {
    // 4×4 image, all pixels class 0 (road)
    const classMap = Array.from({ length: 4 }, () => [0, 0, 0, 0]);
    const raw = buildSegTensor(19, 4, 4, classMap);
    const result = segmentPostprocess(raw, 4, 4, 1, 0, 0);

    expect(result.maskW).toBe(4);
    expect(result.maskH).toBe(4);
    expect(result.numClasses).toBe(19);
    expect(result.maskRgba.length).toBe(4 * 4 * 4);
    expect(result.classCounts[0]).toBe(16); // all pixels are class 0
    expect(result.classCounts.slice(1).every(c => c === 0)).toBe(true);

    // Check first pixel is road color
    expect(result.maskRgba[0]).toBe(128);
    expect(result.maskRgba[1]).toBe(64);
    expect(result.maskRgba[2]).toBe(128);
  });

  it('counts multiple classes correctly', () => {
    const classMap = [
      [0, 0, 1, 1],
      [0, 0, 1, 1],
      [11, 11, 13, 13],
      [11, 11, 13, 13],
    ];
    const raw = buildSegTensor(19, 4, 4, classMap);
    const result = segmentPostprocess(raw, 4, 4, 1, 0, 0);

    expect(result.classCounts[0]).toBe(4);  // road
    expect(result.classCounts[1]).toBe(4);  // sidewalk
    expect(result.classCounts[11]).toBe(4); // person
    expect(result.classCounts[13]).toBe(4); // car
  });
});

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

describe('CITYSCAPES_CLASSES', () => {
  it('has 19 classes', () => {
    expect(CITYSCAPES_CLASSES).toHaveLength(19);
  });

  it('starts with road', () => {
    expect(CITYSCAPES_CLASSES[0]).toBe('road');
  });
});

describe('CITYSCAPES_PALETTE', () => {
  it('has 19 colors', () => {
    expect(CITYSCAPES_PALETTE).toHaveLength(19);
  });

  it('each color has 4 components (RGBA)', () => {
    for (const color of CITYSCAPES_PALETTE) {
      expect(color).toHaveLength(4);
    }
  });

  it('all alpha values are 255', () => {
    for (const color of CITYSCAPES_PALETTE) {
      expect(color[3]).toBe(255);
    }
  });
});
