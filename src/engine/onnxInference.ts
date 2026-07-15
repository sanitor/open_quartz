// onnxInference.ts — Generic TypeScript ORT inference session.
//
// Unlike the Rust wasm path (onnxSession.ts / YoloDetectorWasm), this runs
// directly against `globalThis.ort` (onnxruntime-web loaded via <script>).
// Used for non-YOLO tasks: super-resolution, background removal, depth, etc.

import type * as OrtModule from 'onnxruntime-web';

// onnxruntime-web is loaded at runtime via a <script> tag as `globalThis.ort`.
// Module-local `ort` is guaranteed non-undefined after `ensureOrtLoaded()`;
// the global augmentation types the existence checks in that function.
declare const ort: typeof OrtModule;
declare global {
  // eslint-disable-next-line no-var
  var ort: typeof OrtModule | undefined;
}

// ---------------------------------------------------------------------------
// Session wrapper
// ---------------------------------------------------------------------------

export class OnnxInferenceSession {
  private session: OrtModule.InferenceSession | null = null;
  private _inputNames: string[] = [];
  private _outputNames: string[] = [];
  private _buffer: ArrayBuffer | null = null;
  private _isWasm = false;

  get inputNames(): readonly string[] { return this._inputNames; }
  get outputNames(): readonly string[] { return this._outputNames; }
  get isWasmFallback(): boolean { return this._isWasm; }

  /** Load a model from an ArrayBuffer (already downloaded by modelManager). */
  async loadFromBuffer(buffer: ArrayBuffer): Promise<void> {
    this._buffer = buffer;
    await this.createSession(buffer, ['webgpu', 'wasm']);
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

  /**
   * Recreate the session with WASM-only backend.
   * Called automatically when WebGPU kernels fail at every tile size.
   */
  async fallbackToWasm(): Promise<void> {
    if (this._isWasm) return;
    if (!this._buffer) throw new Error('Cannot fallback: no buffer retained');
    this.session?.release();
    await this.createSession(this._buffer, ['wasm']);
    this._isWasm = true;
    console.warn('[onnx] Fell back to WASM backend');
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
    this._buffer = null;
    this._inputNames = [];
    this._outputNames = [];
  }

  private async createSession(buffer: ArrayBuffer, providers: string[]): Promise<void> {
    await ensureOrtLoaded();
    this.session = await ort.InferenceSession.create(buffer, {
      executionProviders: providers,
    });
    this._inputNames = [...this.session.inputNames];
    this._outputNames = [...this.session.outputNames];
  }
}

// ---------------------------------------------------------------------------
// ORT loading helper (same pattern as ort_bridge's _ensureOrtLoaded)
// ---------------------------------------------------------------------------

let ortLoadPromise: Promise<void> | null = null;

/** @internal Reset ORT load state between tests. */
export function resetOrtLoad(): void { ortLoadPromise = null; }
function ensureOrtLoaded(): Promise<void> {
  if (typeof globalThis.ort !== 'undefined') return Promise.resolve();
  if (ortLoadPromise) return ortLoadPromise;
  ortLoadPromise = new Promise<void>((resolve, reject) => {
    const s = document.createElement('script');
    s.src = '/ort/ort.min.js';
    s.onload = () => {
      const loaded = globalThis.ort;
      if (typeof loaded === 'undefined') {
        reject(new Error('onnxruntime-web loaded but globalThis.ort is undefined'));
      } else {
        loaded.env.wasm.wasmPaths = '/ort/';
        loaded.env.wasm.numThreads = 1;
        resolve();
      }
    };
    s.onerror = () => reject(new Error('Failed to load /ort/ort.min.js'));
    document.head.appendChild(s);
  });
  return ortLoadPromise;
}

// ---------------------------------------------------------------------------
// Generic tiled inference
// ---------------------------------------------------------------------------

/** Per-model encode/decode logic for the tiled runner. */
interface TileCodec {
  /** Number of input channels (NCHW C dimension). */
  channels: number;
  /**
   * If set, model requires fixed spatial input (e.g. 224).
   * Tile step = fixedSize − 2 × pad so interior patches match exactly;
   * edge patches are zero-padded by encode.
   */
  fixedSize?: number;
  /** Called once before tiling begins. Return value is passed to encode/decode. */
  prepare?(rgba: Uint8ClampedArray, width: number, height: number): unknown;
  /** Extract a patch from source pixels → flat CHW float32. */
  encode(
    rgba: Uint8ClampedArray, width: number,
    patchX: number, patchY: number, patchW: number, patchH: number,
    prepared: unknown,
  ): Float32Array;
  /** Write one cropped output tile into outRgba. */
  decode(
    patchOut: Float32Array, outPatchW: number, outPatchPixels: number,
    cropX: number, cropY: number, cropW: number, cropH: number,
    outRgba: Uint8ClampedArray, outW: number,
    dstBaseX: number, dstBaseY: number,
    tileX: number, tileY: number, scale: number,
    prepared: unknown,
  ): void;
}

export const INITIAL_TILE = 64;
export const MIN_TILE     = 16;
const TILE_PAD     = 8;

/** Module-level cache: once we find a tile size that works, reuse it. */
let provenTileSize = INITIAL_TILE;

/** @internal Reset tile cache between tests. */
export function resetProvenTileSize(): void { provenTileSize = INITIAL_TILE; }

/**
 * Run inference on an RGBA image by splitting it into overlapping tiles.
 *
 * For dynamic-input models the tile size is adaptive: starts at
 * {@link provenTileSize} (initially 64), and if WebGPU fails with a buffer
 * allocation error the size is halved and the image is retried from scratch.
 * The surviving size is cached so subsequent frames don't pay the retry cost.
 *
 * For fixed-input models ({@link TileCodec.fixedSize}), tile step is derived
 * from the model's required spatial size.
 */
async function runTiledInference(
  session: OnnxInferenceSession,
  rgba: Uint8ClampedArray,
  width: number,
  height: number,
  scale: number,
  codec: TileCodec,
): Promise<{ rgba: Uint8ClampedArray<ArrayBuffer>; width: number; height: number }> {
  if (codec.fixedSize) {
    return runTilesAtSize(session, rgba, width, height, scale, codec, codec.fixedSize - 2 * TILE_PAD);
  }

  // Dynamic model: adaptive tile sizing → WASM fallback.
  let tile = provenTileSize;
  for (;;) {
    try {
      const result = await runTilesAtSize(session, rgba, width, height, scale, codec, tile);
      provenTileSize = tile;
      return result;
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (!isGpuAllocError(msg)) throw e;
      if (tile <= MIN_TILE) {
        // Tile can't shrink further — WebGPU kernel itself is broken.
        // Rebuild session with WASM backend and retry at default tile.
        if (session.isWasmFallback) throw e;
        console.warn('[tiled-inference] WebGPU incompatible, falling back to WASM');
        await session.fallbackToWasm();
        tile = INITIAL_TILE;
        continue;
      }
      tile = Math.max(MIN_TILE, tile >> 1);
      console.warn(`[tiled-inference] WebGPU allocation failed, retrying with tile=${tile}`);
    }
  }
}

export function isGpuAllocError(msg: string): boolean {
  return msg.includes('Failed to generate')
    || msg.includes('Failed to run')
    || msg.includes('JSEP')
    || msg.includes('buffer size')
    || msg.includes('allocation');
}

async function runTilesAtSize(
  session: OnnxInferenceSession,
  rgba: Uint8ClampedArray,
  width: number,
  height: number,
  scale: number,
  codec: TileCodec,
  tileStep: number,
): Promise<{ rgba: Uint8ClampedArray<ArrayBuffer>; width: number; height: number }> {
  const outW = width * scale;
  const outH = height * scale;
  const outRgba = new Uint8ClampedArray(outW * outH * 4);

  const prepared = codec.prepare?.(rgba, width, height);

  for (let ty = 0; ty < height; ty += tileStep) {
    for (let tx = 0; tx < width; tx += tileStep) {
      const tileW = Math.min(tileStep, width  - tx);
      const tileH = Math.min(tileStep, height - ty);

      const padLeft   = Math.min(TILE_PAD, tx);
      const padTop    = Math.min(TILE_PAD, ty);
      const padRight  = Math.min(TILE_PAD, width  - tx - tileW);
      const padBottom = Math.min(TILE_PAD, height - ty - tileH);

      const patchX = tx - padLeft;
      const patchY = ty - padTop;
      const patchW = tileW + padLeft + padRight;
      const patchH = tileH + padTop  + padBottom;

      const input = codec.encode(rgba, width, patchX, patchY, patchW, patchH, prepared);

      const modelW = codec.fixedSize ?? patchW;
      const modelH = codec.fixedSize ?? patchH;
      const patchOut = await session.run(input, [1, codec.channels, modelH, modelW]);

      const outPatchW      = modelW * scale;
      const outPatchPixels = outPatchW * (modelH * scale);

      codec.decode(
        patchOut, outPatchW, outPatchPixels,
        padLeft * scale, padTop * scale, tileW * scale, tileH * scale,
        outRgba, outW,
        tx * scale, ty * scale,
        tx, ty, scale,
        prepared,
      );
    }
  }

  return { rgba: outRgba, width: outW, height: outH };
}

// ---------------------------------------------------------------------------
// Codecs
// ---------------------------------------------------------------------------

/** Real-ESRGAN: RGB [1,3,H,W] → [1,3,H*s,W*s] */
const rgbCodec: TileCodec = {
  channels: 3,

  encode(rgba, width, patchX, patchY, patchW, patchH) {
    const px = patchW * patchH;
    const input = new Float32Array(3 * px);
    for (let row = 0; row < patchH; row++) {
      for (let col = 0; col < patchW; col++) {
        const srcIdx = ((patchY + row) * width + (patchX + col)) * 4;
        const dstIdx = row * patchW + col;
        input[dstIdx]          = rgba[srcIdx]     / 255;
        input[px + dstIdx]     = rgba[srcIdx + 1] / 255;
        input[2 * px + dstIdx] = rgba[srcIdx + 2] / 255;
      }
    }
    return input;
  },

  decode(patchOut, outPatchW, outPatchPixels, cropX, cropY, cropW, cropH, outRgba, outW, dstBaseX, dstBaseY) {
    for (let row = 0; row < cropH; row++) {
      for (let col = 0; col < cropW; col++) {
        const srcIdx = (cropY + row) * outPatchW + (cropX + col);
        const dstIdx = ((dstBaseY + row) * outW + (dstBaseX + col)) * 4;
        outRgba[dstIdx]     = Math.max(0, Math.min(255, Math.round(patchOut[srcIdx]                    * 255)));
        outRgba[dstIdx + 1] = Math.max(0, Math.min(255, Math.round(patchOut[outPatchPixels + srcIdx]   * 255)));
        outRgba[dstIdx + 2] = Math.max(0, Math.min(255, Math.round(patchOut[2 * outPatchPixels + srcIdx] * 255)));
        outRgba[dstIdx + 3] = 255;
      }
    }
  },
};

/** Sub-pixel CNN: Y-only [1,1,H,W] → [1,1,H*s,W*s], Cb/Cr nearest-neighbor. */
interface YCbCrPrepared { fullY: Float32Array; fullCb: Float32Array; fullCr: Float32Array; imgW: number; imgH: number }

const YCBCR_MODEL_INPUT = 224;

const ycbcrCodec: TileCodec = {
  channels: 1,
  fixedSize: YCBCR_MODEL_INPUT,

  prepare(rgba, width, height): YCbCrPrepared {
    const n = width * height;
    const fullY  = new Float32Array(n);
    const fullCb = new Float32Array(n);
    const fullCr = new Float32Array(n);
    for (let i = 0; i < n; i++) {
      const r = rgba[i * 4]     / 255;
      const g = rgba[i * 4 + 1] / 255;
      const b = rgba[i * 4 + 2] / 255;
      fullY[i]  =  0.299 * r + 0.587 * g + 0.114 * b;
      fullCb[i] = -0.169 * r - 0.331 * g + 0.500 * b + 0.5;
      fullCr[i] =  0.500 * r - 0.419 * g - 0.081 * b + 0.5;
    }
    return { fullY, fullCb, fullCr, imgW: width, imgH: height };
  },

  encode(_rgba, width, patchX, patchY, patchW, patchH, prepared) {
    const { fullY } = prepared as YCbCrPrepared;
    // Zero-padded to fixedSize × fixedSize; patch pixels placed top-left.
    const F = YCBCR_MODEL_INPUT;
    const input = new Float32Array(F * F);  // zero-initialized
    for (let row = 0; row < patchH; row++) {
      for (let col = 0; col < patchW; col++) {
        input[row * F + col] = fullY[(patchY + row) * width + (patchX + col)];
      }
    }
    return input;
  },

  decode(patchOut, outPatchW, _outPatchPixels, cropX, cropY, cropW, cropH, outRgba, outW, dstBaseX, dstBaseY, tileX, tileY, scale, prepared) {
    const { fullCb, fullCr, imgW, imgH } = prepared as YCbCrPrepared;
    for (let row = 0; row < cropH; row++) {
      for (let col = 0; col < cropW; col++) {
        const y = patchOut[(cropY + row) * outPatchW + (cropX + col)];
        const origX = Math.min(tileX + Math.floor(col / scale), imgW - 1);
        const origY = Math.min(tileY + Math.floor(row / scale), imgH - 1);
        const chromaIdx = origY * imgW + origX;
        const cb = fullCb[chromaIdx] - 0.5;
        const cr = fullCr[chromaIdx] - 0.5;
        const dstIdx = ((dstBaseY + row) * outW + (dstBaseX + col)) * 4;
        outRgba[dstIdx]     = Math.max(0, Math.min(255, Math.round((y + 1.402 * cr) * 255)));
        outRgba[dstIdx + 1] = Math.max(0, Math.min(255, Math.round((y - 0.344 * cb - 0.714 * cr) * 255)));
        outRgba[dstIdx + 2] = Math.max(0, Math.min(255, Math.round((y + 1.772 * cb) * 255)));
        outRgba[dstIdx + 3] = 255;
      }
    }
  },
};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/**
 * Run super-resolution on an RGBA pixel buffer.
 * Dispatches to the correct codec and tiles automatically.
 */
export async function runSuperResolution(
  session: OnnxInferenceSession,
  rgba: Uint8ClampedArray,
  width: number,
  height: number,
  scale: number,
  modelType: 'rgb' | 'ycbcr' = 'rgb',
): Promise<{ rgba: Uint8ClampedArray<ArrayBuffer>; width: number; height: number }> {
  return runTiledInference(session, rgba, width, height, scale, modelType === 'ycbcr' ? ycbcrCodec : rgbCodec);
}
