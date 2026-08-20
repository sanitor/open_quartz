// onnxInference.ts - TypeScript ORT-Web session lifecycle and raw tensor calls.
//
// Runs against `globalThis.ort` (onnxruntime-web loaded via <script>).
// Rust owns ONNX task planning, image transforms, and postprocessing.

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
  private _gpuOutputEnabled = false;

  get inputNames(): readonly string[] { return this._inputNames; }
  get outputNames(): readonly string[] { return this._outputNames; }
  get isWasmFallback(): boolean { return this._isWasm; }
  /** Whether this session was created with GPU I/O binding. */
  get gpuOutputEnabled(): boolean { return this._gpuOutputEnabled; }

  /** Input tensor shape from model metadata, e.g. [1, 1, 224, 224]. Empty if unavailable. */
  get inputShape(): ReadonlyArray<number | string> {
    if (!this.session) return [];
    const meta = this.session.inputMetadata;
    if (!meta || meta.length === 0) return [];
    const m = meta[0];
    return (m && 'shape' in m) ? m.shape : [];
  }

  /** Output tensor shape from model metadata. Empty if unavailable. */
  get outputShape(): ReadonlyArray<number | string> {
    if (!this.session) return [];
    const meta = this.session.outputMetadata;
    if (!meta || meta.length === 0) return [];
    const m = meta[0];
    return (m && 'shape' in m) ? m.shape : [];
  }

  /**
   * Load a model from an ArrayBuffer (already downloaded by modelManager).
   * When gpuDevice is provided, the session shares the device with the render
   * pipeline and outputs tensors as GPUBuffers (zero CPU readback).
   */
  async loadFromBuffer(buffer: ArrayBuffer, gpuDevice?: GPUDevice): Promise<void> {
    this._buffer = buffer;
    if (gpuDevice) {
      await this.createSession(buffer, [{ name: 'webgpu', device: gpuDevice } as unknown as string, 'wasm'], 'gpu-buffer');
    } else {
      await this.createSession(buffer, ['webgpu', 'wasm']);
    }
  }

  /** Load a model from a URL (blob URL or network URL). */
  async loadFromUrl(url: string, gpuDevice?: GPUDevice): Promise<void> {
    await ensureOrtLoaded();
    const options: OrtModule.InferenceSession.SessionOptions = {
      executionProviders: gpuDevice
        ? [{ name: 'webgpu', device: gpuDevice } as unknown as string, 'wasm']
        : ['webgpu', 'wasm'],
    };
    if (gpuDevice) {
      options.preferredOutputLocation = 'gpu-buffer';
      this._gpuOutputEnabled = true;
    }
    this.session = await ort.InferenceSession.create(url, options);
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
    this._gpuOutputEnabled = false;
    console.warn('[onnx] Fell back to WASM backend');
  }

  /**
   * Probe WebGPU compatibility by running a tiny dummy inference.
   * If the probe fails with a GPU error, automatically falls back to WASM.
   * Returns the backend that will be used ('webgpu' or 'wasm').
   */
  async probeBackend(inputChannels: number = 3): Promise<'webgpu' | 'wasm'> {
    if (this._isWasm) return 'wasm';
    if (!this.session) throw new Error('OnnxInferenceSession not loaded');
    const size = 8;  // minimal spatial dims
    const dummy = new Float32Array(inputChannels * size * size);
    try {
      await this.run(dummy, [1, inputChannels, size, size]);
      return 'webgpu';
    } catch {
      // WebGPU kernel failed — fall back to WASM
      try {
        await this.fallbackToWasm();
        return 'wasm';
      } catch {
        return 'wasm';
      }
    }
  }

  /** Run inference with a single float32 input tensor. Returns CPU data. */
  async run(input: Float32Array, shape: number[]): Promise<Float32Array> {
    if (!this.session) throw new Error('OnnxInferenceSession not loaded');
    const tensor = new ort.Tensor('float32', input, shape);
    const feeds: Record<string, OrtModule.Tensor> = { [this._inputNames[0]]: tensor };
    const results = await this.session.run(feeds);
    const output = results[this._outputNames[0]];
    if (output.location === 'gpu-buffer') {
      // getData() triggers GPU→CPU download
      return await output.getData(true) as Float32Array;
    }
    return output.data as Float32Array;
  }

  /**
   * Run inference and return the raw ORT output tensor (may be on GPU).
   * Caller can inspect `tensor.location` and use `tensor.gpuBuffer` directly.
   */
  async runRaw(input: Float32Array, shape: number[]): Promise<OrtModule.Tensor> {
    if (!this.session) throw new Error('OnnxInferenceSession not loaded');
    const tensor = new ort.Tensor('float32', input, shape);
    const feeds: Record<string, OrtModule.Tensor> = { [this._inputNames[0]]: tensor };
    const results = await this.session.run(feeds);
    return results[this._outputNames[0]];
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
      if (tensor.location === 'gpu-buffer') {
        out[name] = { data: await tensor.getData(true) as Float32Array, dims: tensor.dims };
      } else {
        out[name] = { data: tensor.data as Float32Array, dims: tensor.dims };
      }
    }
    return out;
  }

  dispose(): void {
    this.session?.release();
    this.session = null;
    this._buffer = null;
    this._gpuOutputEnabled = false;
    this._inputNames = [];
    this._outputNames = [];
  }

  private async createSession(
    buffer: ArrayBuffer,
    providers: (string | Record<string, unknown>)[],
    outputLocation?: OrtModule.Tensor.DataLocation,
  ): Promise<void> {
    await ensureOrtLoaded();
    const options: OrtModule.InferenceSession.SessionOptions = {
      executionProviders: providers as OrtModule.InferenceSession.SessionOptions['executionProviders'],
    };
    if (outputLocation) {
      options.preferredOutputLocation = outputLocation as 'gpu-buffer';
      this._gpuOutputEnabled = true;
    }
    this.session = await ort.InferenceSession.create(buffer, options);
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
export function ensureOrtLoaded(): Promise<void> {
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

export function isGpuAllocError(msg: string): boolean {
  return msg.includes("Failed to generate")
    || msg.includes("Failed to run")
    || msg.includes("JSEP")
    || msg.includes("buffer size")
    || msg.includes("allocation");
}
