// onnxInference.ts — Generic TypeScript ORT inference session.
//
// Unlike the Rust wasm path (onnxSession.ts / YoloDetectorWasm), this runs
// directly against `globalThis.ort` (onnxruntime-web loaded via <script>).
// Used for non-YOLO tasks: super-resolution, background removal, depth, etc.

import type * as OrtModule from 'onnxruntime-web';
import type { OnnxTask } from './onnxCatalog';

declare const ort: typeof OrtModule;

// ---------------------------------------------------------------------------
// Session wrapper
// ---------------------------------------------------------------------------

export class OnnxInferenceSession {
  private session: OrtModule.InferenceSession | null = null;
  private _inputNames: string[] = [];
  private _outputNames: string[] = [];

  get inputNames(): readonly string[] { return this._inputNames; }
  get outputNames(): readonly string[] { return this._outputNames; }

  /** Load a model from an ArrayBuffer (already downloaded by modelManager). */
  async loadFromBuffer(buffer: ArrayBuffer): Promise<void> {
    await ensureOrtLoaded();
    this.session = await ort.InferenceSession.create(buffer, {
      executionProviders: ['webgpu', 'wasm'],
    });
    this._inputNames = [...this.session.inputNames];
    this._outputNames = [...this.session.outputNames];
  }

  /** Load a model from a URL (blob URL or network URL). */
  async loadFromUrl(url: string): Promise<void> {
    await ensureOrtLoaded();
    this.session = await ort.InferenceSession.create(url, {
      executionProviders: ['webgpu', 'wasm'],
    });
    this._inputNames = [...this.session.inputNames];
    this._outputNames = [...this.session.outputNames];
  }

  /** Run inference with a single float32 input tensor. */
  async run(input: Float32Array, shape: number[]): Promise<Float32Array> {
    if (!this.session) throw new Error('OnnxInferenceSession not loaded');
    const tensor = new ort.Tensor('float32', input, shape);
    const feeds: Record<string, OrtModule.Tensor> = { [this._inputNames[0]]: tensor };
    const results = await this.session.run(feeds);
    const output = results[this._outputNames[0]];
    return output.data as Float32Array;
  }

  /** Run inference returning full result map (for multi-output models). */
  async runFull(
    feeds: Record<string, { data: Float32Array; shape: number[] }>,
  ): Promise<Record<string, { data: Float32Array; dims: readonly number[] }>> {
    if (!this.session) throw new Error('OnnxInferenceSession not loaded');
    const ortFeeds: Record<string, OrtModule.Tensor> = {};
    for (const [name, { data, shape }] of Object.entries(feeds)) {
      ortFeeds[name] = new ort.Tensor('float32', data, shape);
    }
    const results = await this.session.run(ortFeeds);
    const out: Record<string, { data: Float32Array; dims: readonly number[] }> = {};
    for (const [name, tensor] of Object.entries(results)) {
      out[name] = { data: tensor.data as Float32Array, dims: tensor.dims };
    }
    return out;
  }

  dispose(): void {
    this.session?.release();
    this.session = null;
    this._inputNames = [];
    this._outputNames = [];
  }
}

// ---------------------------------------------------------------------------
// ORT loading helper (same pattern as ort_bridge's _ensureOrtLoaded)
// ---------------------------------------------------------------------------

let ortLoadPromise: Promise<void> | null = null;

function ensureOrtLoaded(): Promise<void> {
  if (typeof globalThis.ort !== 'undefined') return Promise.resolve();
  if (ortLoadPromise) return ortLoadPromise;
  ortLoadPromise = new Promise<void>((resolve, reject) => {
    const s = document.createElement('script');
    s.src = '/ort/ort.min.js';
    s.onload = () => {
      if (typeof globalThis.ort === 'undefined') {
        reject(new Error('onnxruntime-web loaded but globalThis.ort is undefined'));
      } else {
        (globalThis.ort as typeof OrtModule).env.wasm.wasmPaths = '/ort/';
        (globalThis.ort as typeof OrtModule).env.wasm.numThreads = 1;
        resolve();
      }
    };
    s.onerror = () => reject(new Error('Failed to load /ort/ort.min.js'));
    document.head.appendChild(s);
  });
  return ortLoadPromise;
}

// ---------------------------------------------------------------------------
// Super-Resolution pre/post processing
// ---------------------------------------------------------------------------

/**
 * Pre-process an RGBA pixel buffer for the Sub-pixel CNN SR model.
 *
 * 1. Convert RGB → YCbCr
 * 2. Extract Y channel
 * 3. Normalize to [0, 1]
 * 4. Return { yTensor, cbData, crData } for post-processing
 */
export function srPreprocess(
  rgba: Uint8ClampedArray,
  width: number,
  height: number,
): { yTensor: Float32Array; yShape: number[]; cbData: Float32Array; crData: Float32Array } {
  const pixelCount = width * height;
  const yData = new Float32Array(pixelCount);
  const cbData = new Float32Array(pixelCount);
  const crData = new Float32Array(pixelCount);

  for (let i = 0; i < pixelCount; i++) {
    const r = rgba[i * 4] / 255;
    const g = rgba[i * 4 + 1] / 255;
    const b = rgba[i * 4 + 2] / 255;

    // RGB → YCbCr (ITU-R BT.601)
    yData[i] = 0.299 * r + 0.587 * g + 0.114 * b;
    cbData[i] = -0.169 * r - 0.331 * g + 0.500 * b + 0.5;
    crData[i] = 0.500 * r - 0.419 * g - 0.081 * b + 0.5;
  }

  return {
    yTensor: yData,
    yShape: [1, 1, height, width],  // NCHW
    cbData,
    crData,
  };
}

/**
 * Post-process SR model output back to RGBA.
 *
 * 1. Take upscaled Y channel from model output
 * 2. Bicubic-upscale Cb/Cr to match (done as nearest for simplicity in MVP)
 * 3. YCbCr → RGB
 * 4. Return RGBA Uint8ClampedArray
 */
export function srPostprocess(
  yUpscaled: Float32Array,
  outWidth: number,
  outHeight: number,
  cbData: Float32Array,
  crData: Float32Array,
  inWidth: number,
  inHeight: number,
): Uint8ClampedArray {
  const outPixels = outWidth * outHeight;
  const rgba = new Uint8ClampedArray(outPixels * 4);

  for (let oy = 0; oy < outHeight; oy++) {
    for (let ox = 0; ox < outWidth; ox++) {
      const outIdx = oy * outWidth + ox;

      // Y from upscaled output
      const y = yUpscaled[outIdx];

      // Cb/Cr from nearest-neighbor upscale of original
      const srcX = Math.min(Math.floor(ox * inWidth / outWidth), inWidth - 1);
      const srcY = Math.min(Math.floor(oy * inHeight / outHeight), inHeight - 1);
      const srcIdx = srcY * inWidth + srcX;
      const cb = cbData[srcIdx] - 0.5;
      const cr = crData[srcIdx] - 0.5;

      // YCbCr → RGB (ITU-R BT.601)
      const r = y + 1.402 * cr;
      const g = y - 0.344 * cb - 0.714 * cr;
      const b = y + 1.772 * cb;

      rgba[outIdx * 4] = Math.max(0, Math.min(255, Math.round(r * 255)));
      rgba[outIdx * 4 + 1] = Math.max(0, Math.min(255, Math.round(g * 255)));
      rgba[outIdx * 4 + 2] = Math.max(0, Math.min(255, Math.round(b * 255)));
      rgba[outIdx * 4 + 3] = 255;
    }
  }

  return rgba;
}

// ---------------------------------------------------------------------------
// Task-specific inference runners
// ---------------------------------------------------------------------------

/**
 * Run super-resolution on an RGBA pixel buffer.
 *
 * Supports two model types:
 * - **Real-ESRGAN** (realesr-general-x4v3): RGB in/out, dynamic input size, 4× scale.
 *   Input: [1,3,H,W] float32 0-1. Output: [1,3,H*4,W*4].
 * - **Sub-pixel CNN**: Y-channel only, fixed 224×224 input, 3× scale (legacy).
 *
 * The `modelType` parameter selects the pipeline. Default is `'rgb'` (Real-ESRGAN).
 */
export async function runSuperResolution(
  session: OnnxInferenceSession,
  rgba: Uint8ClampedArray,
  width: number,
  height: number,
  scale: number,
  modelType: 'rgb' | 'ycbcr' = 'rgb',
): Promise<{ rgba: Uint8ClampedArray; width: number; height: number }> {
  if (modelType === 'ycbcr') {
    return runSrYCbCr(session, rgba, width, height, scale);
  }
  return runSrRgb(session, rgba, width, height, scale);
}

/** Real-ESRGAN pipeline: RGB [1,3,H,W] → [1,3,H*s,W*s] */
async function runSrRgb(
  session: OnnxInferenceSession,
  rgba: Uint8ClampedArray,
  width: number,
  height: number,
  scale: number,
): Promise<{ rgba: Uint8ClampedArray; width: number; height: number }> {
  // Limit input size to keep output within WebGPU buffer limits.
  // Max long side 256 → 4× output = 1024px, ~12MB tensor (safe).
  const MAX_SIDE = 256;
  let inW = width;
  let inH = height;
  let inputRgba = rgba;

  if (inW > MAX_SIDE || inH > MAX_SIDE) {
    const ratio = Math.min(MAX_SIDE / inW, MAX_SIDE / inH);
    inW = Math.round(inW * ratio);
    inH = Math.round(inH * ratio);
    // Resize via canvas
    const srcCanvas = document.createElement('canvas');
    srcCanvas.width = width;
    srcCanvas.height = height;
    const srcCtx = srcCanvas.getContext('2d')!;
    srcCtx.putImageData(new ImageData(rgba, width, height), 0, 0);
    const smallCanvas = document.createElement('canvas');
    smallCanvas.width = inW;
    smallCanvas.height = inH;
    const smallCtx = smallCanvas.getContext('2d')!;
    smallCtx.drawImage(srcCanvas, 0, 0, inW, inH);
    inputRgba = smallCtx.getImageData(0, 0, inW, inH).data;
  }

  const pixelCount = inW * inH;

  // RGBA → NCHW float32 [1,3,H,W], normalized 0-1
  const input = new Float32Array(3 * pixelCount);
  const hw = pixelCount;
  for (let i = 0; i < pixelCount; i++) {
    input[i]          = inputRgba[i * 4]     / 255;  // R
    input[hw + i]     = inputRgba[i * 4 + 1] / 255;  // G
    input[2 * hw + i] = inputRgba[i * 4 + 2] / 255;  // B
  }

  const output = await session.run(input, [1, 3, inH, inW]);

  // Output: NCHW [1,3,H*s,W*s] → RGBA
  const outW = inW * scale;
  const outH = inH * scale;
  const outPixels = outW * outH;
  const outRgba = new Uint8ClampedArray(outPixels * 4);
  for (let i = 0; i < outPixels; i++) {
    outRgba[i * 4]     = Math.max(0, Math.min(255, Math.round(output[i]              * 255)));
    outRgba[i * 4 + 1] = Math.max(0, Math.min(255, Math.round(output[outPixels + i]  * 255)));
    outRgba[i * 4 + 2] = Math.max(0, Math.min(255, Math.round(output[2*outPixels + i]* 255)));
    outRgba[i * 4 + 3] = 255;
  }

  return { rgba: outRgba, width: outW, height: outH };
}

/** Sub-pixel CNN legacy pipeline: YCbCr, fixed 224 input */
async function runSrYCbCr(
  session: OnnxInferenceSession,
  rgba: Uint8ClampedArray,
  width: number,
  height: number,
  scale: number,
): Promise<{ rgba: Uint8ClampedArray; width: number; height: number }> {
  const MODEL_INPUT = 224;
  const srcCanvas = document.createElement('canvas');
  srcCanvas.width = width;
  srcCanvas.height = height;
  const srcCtx = srcCanvas.getContext('2d')!;
  srcCtx.putImageData(new ImageData(rgba, width, height), 0, 0);

  const resizedCanvas = document.createElement('canvas');
  resizedCanvas.width = MODEL_INPUT;
  resizedCanvas.height = MODEL_INPUT;
  const resizedCtx = resizedCanvas.getContext('2d')!;
  resizedCtx.drawImage(srcCanvas, 0, 0, MODEL_INPUT, MODEL_INPUT);
  const resizedData = resizedCtx.getImageData(0, 0, MODEL_INPUT, MODEL_INPUT);

  const { yTensor, yShape, cbData, crData } = srPreprocess(
    resizedData.data, MODEL_INPUT, MODEL_INPUT,
  );
  const yUpscaled = await session.run(yTensor, yShape);
  const outWidth = MODEL_INPUT * scale;
  const outHeight = MODEL_INPUT * scale;
  const outRgba = srPostprocess(
    yUpscaled, outWidth, outHeight, cbData, crData, MODEL_INPUT, MODEL_INPUT,
  );
  return { rgba: outRgba, width: outWidth, height: outHeight };
}
