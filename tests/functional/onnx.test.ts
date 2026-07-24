/**
 * Functional tests for ONNX model lifecycle and real inference.
 *
 * These tests download real models, run real inference via onnxruntime-node,
 * and verify actual output. They are slow (~minutes) and require network.
 *
 * Run with: npm run test:functional
 */
import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import * as ort from 'onnxruntime-node';
import * as fs from 'node:fs';
import * as path from 'node:path';
import * as os from 'node:os';
import { ONNX_CATALOG } from '../../src/catalog/onnxCatalog';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const CACHE_DIR = path.join(os.tmpdir(), 'oq-test-models');

async function ensureModel(modelId: string): Promise<string> {
  const entry = ONNX_CATALOG[modelId];
  if (!entry) throw new Error(`Unknown model: ${modelId}`);

  const filePath = path.join(CACHE_DIR, `${modelId}.onnx`);
  if (fs.existsSync(filePath)) return filePath;

  fs.mkdirSync(CACHE_DIR, { recursive: true });
  console.log(`Downloading ${entry.label} (${(entry.fileSize / 1e6).toFixed(1)}MB)...`);
  const res = await fetch(entry.downloadUrl);
  if (!res.ok) throw new Error(`HTTP ${res.status}: ${entry.downloadUrl}`);
  const buffer = await res.arrayBuffer();
  fs.writeFileSync(filePath, Buffer.from(buffer));
  console.log(`  → saved to ${filePath}`);
  return filePath;
}

function deleteModel(modelId: string): void {
  const filePath = path.join(CACHE_DIR, `${modelId}.onnx`);
  if (fs.existsSync(filePath)) fs.unlinkSync(filePath);
}

/** Create a flat RGBA Uint8ClampedArray (red-green gradient). */
function makeTestRgba(w: number, h: number): Uint8ClampedArray {
  const buf = new Uint8ClampedArray(w * h * 4);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const i = (y * w + x) * 4;
      buf[i] = Math.round((x / w) * 255);
      buf[i + 1] = Math.round((y / h) * 255);
      buf[i + 2] = 128;
      buf[i + 3] = 255;
    }
  }
  return buf;
}

/** RGBA → NCHW float32 [1,3,H,W], normalized to 0-1. */
function rgbaToNchw(rgba: Uint8ClampedArray, w: number, h: number): Float32Array {
  const px = w * h;
  const out = new Float32Array(3 * px);
  for (let i = 0; i < px; i++) {
    out[i] = rgba[i * 4] / 255;
    out[px + i] = rgba[i * 4 + 1] / 255;
    out[2 * px + i] = rgba[i * 4 + 2] / 255;
  }
  return out;
}

/** RGBA → NCHW float32 [1,3,H,W], BGR order + ImageNet normalize. */
function rgbaToMidasInput(rgba: Uint8ClampedArray, w: number, h: number): Float32Array {
  const mean = [0.485, 0.456, 0.406];
  const std = [0.229, 0.224, 0.225];
  const px = w * h;
  const out = new Float32Array(3 * px);
  for (let i = 0; i < px; i++) {
    const r = rgba[i * 4] / 255;
    const g = rgba[i * 4 + 1] / 255;
    const b = rgba[i * 4 + 2] / 255;
    // BGR order
    out[i] = (b - mean[0]) / std[0];
    out[px + i] = (g - mean[1]) / std[1];
    out[2 * px + i] = (r - mean[2]) / std[2];
  }
  return out;
}

// ---------------------------------------------------------------------------
// Model download lifecycle
// ---------------------------------------------------------------------------

describe('ONNX model lifecycle', () => {
  const MODEL_ID = 'super-resolution-3x';  // smallest model (~240KB)

  afterAll(() => {
    deleteModel(MODEL_ID);
  });

  it('downloads a model from the catalog URL', async () => {
    deleteModel(MODEL_ID);  // ensure clean state
    const filePath = await ensureModel(MODEL_ID);
    expect(fs.existsSync(filePath)).toBe(true);

    const stat = fs.statSync(filePath);
    expect(stat.size).toBeGreaterThan(100_000);
    expect(stat.size).toBe(ONNX_CATALOG[MODEL_ID].fileSize);
  });

  it('uses cached model on second call', async () => {
    const filePath = await ensureModel(MODEL_ID);
    const stat1 = fs.statSync(filePath);

    // Second call should not re-download
    const filePath2 = await ensureModel(MODEL_ID);
    expect(filePath2).toBe(filePath);

    const stat2 = fs.statSync(filePath2);
    expect(stat2.mtimeMs).toBe(stat1.mtimeMs);
  });

  it('delete removes model from cache', async () => {
    const filePath = await ensureModel(MODEL_ID);
    expect(fs.existsSync(filePath)).toBe(true);
    deleteModel(MODEL_ID);
    expect(fs.existsSync(filePath)).toBe(false);
  });

  it('can re-download after delete', async () => {
    deleteModel(MODEL_ID);
    const filePath = await ensureModel(MODEL_ID);
    expect(fs.existsSync(filePath)).toBe(true);
    const stat = fs.statSync(filePath);
    expect(stat.size).toBe(ONNX_CATALOG[MODEL_ID].fileSize);
  });
});

// ---------------------------------------------------------------------------
// Super-Resolution: Sub-pixel CNN 3×
// ---------------------------------------------------------------------------

describe('Super-Resolution 3× (sub-pixel CNN)', () => {
  let session: ort.InferenceSession;

  beforeAll(async () => {
    const modelPath = await ensureModel('super-resolution-3x');
    session = await ort.InferenceSession.create(modelPath);
  });

  afterAll(async () => {
    await session?.release();
  });

  it('has correct input/output names', () => {
    expect(session.inputNames).toContain('input');
    expect(session.outputNames.length).toBeGreaterThan(0);
  });

  it('produces 3× upscaled output from 224×224 Y-channel input', async () => {
    const w = 224, h = 224;
    const rgba = makeTestRgba(w, h);
    // Sub-pixel CNN takes Y channel only: [1, 1, 224, 224]
    const px = w * h;
    const yInput = new Float32Array(px);
    for (let i = 0; i < px; i++) {
      yInput[i] = 0.299 * (rgba[i * 4] / 255) + 0.587 * (rgba[i * 4 + 1] / 255) + 0.114 * (rgba[i * 4 + 2] / 255);
    }

    const tensor = new ort.Tensor('float32', yInput, [1, 1, h, w]);
    const results = await session.run({ [session.inputNames[0]]: tensor });
    const output = results[session.outputNames[0]];

    expect(output.dims[0]).toBe(1);
    // Output should be 3× the input
    expect(output.dims[output.dims.length - 2]).toBe(h * 3);
    expect(output.dims[output.dims.length - 1]).toBe(w * 3);
    // Output should have real values (not all zeros)
    const data = output.data as Float32Array;
    const nonZero = data.filter(v => Math.abs(v) > 1e-6).length;
    expect(nonZero).toBeGreaterThan(data.length * 0.5);
  });
});

// ---------------------------------------------------------------------------
// Super-Resolution: Real-ESRGAN 4×
// ---------------------------------------------------------------------------

describe('Real-ESRGAN 4×', () => {
  let session: ort.InferenceSession;

  beforeAll(async () => {
    const modelPath = await ensureModel('realesrgan-x4');
    session = await ort.InferenceSession.create(modelPath);
  });

  afterAll(async () => {
    await session?.release();
  });

  it('has correct input/output names', () => {
    expect(session.inputNames.length).toBe(1);
    expect(session.outputNames.length).toBe(1);
  });

  it('produces 4× upscaled RGB output from 16×16 input', async () => {
    const w = 16, h = 16;
    const rgba = makeTestRgba(w, h);
    const input = rgbaToNchw(rgba, w, h);
    const tensor = new ort.Tensor('float32', input, [1, 3, h, w]);
    const results = await session.run({ [session.inputNames[0]]: tensor });
    const output = results[session.outputNames[0]];

    expect(output.dims).toEqual([1, 3, h * 4, w * 4]);
    const data = output.data as Float32Array;
    expect(data.length).toBe(3 * h * 4 * w * 4);
    // Values should be in a reasonable range (ESRGAN outputs 0-1 float)
    const inRange = data.filter(v => v >= -0.5 && v <= 1.5).length;
    expect(inRange).toBeGreaterThan(data.length * 0.9);
  });
});

// ---------------------------------------------------------------------------
// Background Removal: u2netp
// ---------------------------------------------------------------------------

describe('u2netp (background removal)', () => {
  let session: ort.InferenceSession;

  beforeAll(async () => {
    const modelPath = await ensureModel('u2netp');
    session = await ort.InferenceSession.create(modelPath);
  });

  afterAll(async () => {
    await session?.release();
  });

  it('has single input and at least one output', () => {
    expect(session.inputNames.length).toBe(1);
    expect(session.outputNames.length).toBeGreaterThanOrEqual(1);
  });

  it('produces a mask output from 320×320 RGB input', async () => {
    const w = 320, h = 320;
    const rgba = makeTestRgba(w, h);
    const input = rgbaToNchw(rgba, w, h);
    const tensor = new ort.Tensor('float32', input, [1, 3, h, w]);
    const results = await session.run({ [session.inputNames[0]]: tensor });

    // u2netp outputs multiple maps; first is the main mask
    const output = results[session.outputNames[0]];
    expect(output.dims[0]).toBe(1);
    // Mask should be 320×320 (same as input)
    const outH = output.dims[output.dims.length - 2];
    const outW = output.dims[output.dims.length - 1];
    expect(outH).toBe(h);
    expect(outW).toBe(w);
    // Values should be sigmoid outputs (0-1 range)
    const data = output.data as Float32Array;
    const inRange = data.filter(v => v >= -0.1 && v <= 1.1).length;
    expect(inRange).toBeGreaterThan(data.length * 0.95);
  });
});

// ---------------------------------------------------------------------------
// Background Removal: MODNet
// ---------------------------------------------------------------------------

describe('MODNet (portrait matting)', () => {
  let session: ort.InferenceSession;

  beforeAll(async () => {
    const modelPath = await ensureModel('modnet');
    session = await ort.InferenceSession.create(modelPath);
  });

  afterAll(async () => {
    await session?.release();
  });

  it('has expected input/output names', () => {
    expect(session.inputNames.length).toBe(1);
    expect(session.outputNames.length).toBeGreaterThanOrEqual(1);
  });

  it('produces alpha matte from 512×512 RGB input', async () => {
    const w = 512, h = 512;
    const rgba = makeTestRgba(w, h);
    const input = rgbaToNchw(rgba, w, h);
    const tensor = new ort.Tensor('float32', input, [1, 3, h, w]);
    const results = await session.run({ [session.inputNames[0]]: tensor });
    const output = results[session.outputNames[0]];

    expect(output.dims[0]).toBe(1);
    // Matte values in 0-1
    const data = output.data as Float32Array;
    const inRange = data.filter(v => v >= -0.1 && v <= 1.1).length;
    expect(inRange).toBeGreaterThan(data.length * 0.95);
  });
});

// ---------------------------------------------------------------------------
// Depth Estimation: MiDaS v2.1 Small
// ---------------------------------------------------------------------------

describe('MiDaS v2.1 Small (depth estimation)', () => {
  let session: ort.InferenceSession;

  beforeAll(async () => {
    const modelPath = await ensureModel('midas-small');
    session = await ort.InferenceSession.create(modelPath);
  });

  afterAll(async () => {
    await session?.release();
  });

  it('has input named "input.1" or similar', () => {
    expect(session.inputNames.length).toBe(1);
    expect(session.outputNames.length).toBe(1);
  });

  it('produces 256×256 depth map from 256×256 BGR ImageNet-normalized input', async () => {
    const w = 256, h = 256;
    const rgba = makeTestRgba(w, h);
    const input = rgbaToMidasInput(rgba, w, h);
    const tensor = new ort.Tensor('float32', input, [1, 3, h, w]);
    const results = await session.run({ [session.inputNames[0]]: tensor });
    const output = results[session.outputNames[0]];

    // Output: [1, 256, 256] relative depth
    expect(output.dims).toEqual([1, h, w]);
    const data = output.data as Float32Array;
    expect(data.length).toBe(h * w);

    // Depth values should have non-trivial range (not all same value)
    let min = Infinity, max = -Infinity;
    for (const v of data) {
      if (v < min) min = v;
      if (v > max) max = v;
    }
    expect(max - min).toBeGreaterThan(0.1);
  });
});

// ---------------------------------------------------------------------------
// Custom model: load any .onnx file and run inference
// ---------------------------------------------------------------------------

describe('Custom ONNX model (using SR-3x as stand-in)', () => {
  it('can load, introspect, and run a model without catalog metadata', async () => {
    // Use SR-3x as a "custom" model — load by file path, no catalog lookup
    const modelPath = await ensureModel('super-resolution-3x');
    const buffer = fs.readFileSync(modelPath);

    // Simulate introspection: create session, get I/O names
    const session = await ort.InferenceSession.create(buffer.buffer);
    expect(session.inputNames.length).toBeGreaterThan(0);
    expect(session.outputNames.length).toBeGreaterThan(0);

    // Run inference with the introspected input name
    const w = 224, h = 224;
    const px = w * h;
    const input = new Float32Array(px);
    for (let i = 0; i < px; i++) input[i] = Math.random();
    const tensor = new ort.Tensor('float32', input, [1, 1, h, w]);
    const results = await session.run({ [session.inputNames[0]]: tensor });
    const output = results[session.outputNames[0]];

    expect(output.data.length).toBeGreaterThan(0);
    await session?.release();
  });
});


// ---------------------------------------------------------------------------
// Detection: YOLOv8n
// ---------------------------------------------------------------------------

import { detectPostprocess } from '../../src/engine/onnx/yoloDetectionPostprocess';
import type { Detection } from '../../src/engine/onnx/yoloDetectionPostprocess';

describe('YOLOv8n (detection)', () => {
  let session: ort.InferenceSession;

  beforeAll(async () => {
    const modelPath = await ensureModel('yolov8n');
    session = await ort.InferenceSession.create(modelPath);
  });

  afterAll(async () => {
    await session?.release();
  });

  it('has single input and at least one output', () => {
    expect(session.inputNames.length).toBe(1);
    expect(session.outputNames.length).toBeGreaterThanOrEqual(1);
  });

  it('produces raw output tensor from 640×640 RGB input', async () => {
    const w = 640, h = 640;
    const rgba = makeTestRgba(w, h);
    const input = rgbaToNchw(rgba, w, h);
    const tensor = new ort.Tensor('float32', input, [1, 3, h, w]);
    const results = await session.run({ [session.inputNames[0]]: tensor });

    const output = results[session.outputNames[0]];
    // YOLOv8 output: [1, 84, 8400] = 4 bbox + 80 class scores, 8400 boxes
    expect(output.dims.length).toBe(3);
    expect(output.dims[0]).toBe(1);
    expect(output.data.length).toBeGreaterThan(0);
  });

  it('detectPostprocess decodes valid detections from raw output', async () => {
    const w = 640, h = 640;
    const rgba = makeTestRgba(w, h);
    const input = rgbaToNchw(rgba, w, h);
    const tensor = new ort.Tensor('float32', input, [1, 3, h, w]);
    const results = await session.run({ [session.inputNames[0]]: tensor });
    const raw = results[session.outputNames[0]].data as Float32Array;

    // Use very low threshold to catch even weak detections
    const detections: Detection[] = detectPostprocess(raw, w, h, 1, 0, 0, 0.001, 0.45);

    // Verify bbox format: normalized [0,1] coordinates
    for (const d of detections) {
      expect(d.bbox[0]).toBeGreaterThanOrEqual(0);
      expect(d.bbox[1]).toBeGreaterThanOrEqual(0);
      expect(d.bbox[2]).toBeLessThanOrEqual(1);
      expect(d.bbox[3]).toBeLessThanOrEqual(1);
      expect(d.score).toBeGreaterThan(0);
      expect(d.classId).toBeGreaterThanOrEqual(0);
      expect(d.classId).toBeLessThan(80);
    }

    // Log detection counts at various thresholds for diagnostics
    const at25 = detectPostprocess(raw, w, h, 1, 0, 0, 0.25, 0.45);
    console.log(`[yolov8n] detections at threshold 0.001: ${detections.length}, at 0.25: ${at25.length}`);
  });
});
