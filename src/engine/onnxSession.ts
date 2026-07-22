// OnnxSession — one-per-descriptor detector façade with lazy wasm init.
//
// - The upstream `rimeflow-yolov8n` `ort_bridge` auto-loads onnxruntime-web
//   from `/ort/ort.min.js` on the first `ort_init` call, so no pre-check on
//   `globalThis.ort` is required here.
// - The wasm module is a static import via the Vite alias
//   `@nodes/yolo-detector` (`vite.config.ts`, `vitest.config.ts`,
//   `tsconfig.app.json` paths). We lazily *initialize* it on first use.
// - `wasm-pack --target web` emits `detect(...)` as `Promise<any>` — no typed
//   binding for our `DetectionJs` struct because `serde_wasm_bindgen` doesn't
//   emit `.d.ts`. We own the domain type here and narrow at the boundary.

import __wbg_init, { YoloDetectorWasm } from '@nodes/yolo-detector';
import type { OnnxModelDescriptor } from '../catalog/onnxRegistry';

export interface OnnxDetection {
  bbox: [number, number, number, number];
  score: number;
  class_id: number;
  class_name: string;
}

export interface OnnxResult {
  detections: OnnxDetection[];
  scoreThreshold: number;
  iouThreshold: number;
}

export type OnnxSessionStatus = 'idle' | 'loading' | 'ready' | 'error';

const WASM_URL = new URL(
  '../../rust/crates/yolo-detector/pkg/yolo_detector_bg.wasm',
  import.meta.url,
);

// The wasm module has a single global instantiation. Multiple sessions share it.
let wasmReady: Promise<void> | null = null;

function ensureWasmReady(): Promise<void> {
  if (wasmReady) return wasmReady;
  const promise = __wbg_init(WASM_URL).then(() => undefined);
  wasmReady = promise;
  return promise;
}

function isDetection(v: unknown): v is OnnxDetection {
  if (!v || typeof v !== 'object') return false;
  if (!('bbox' in v && 'score' in v && 'class_id' in v && 'class_name' in v)) return false;
  const bbox = v.bbox;
  if (!Array.isArray(bbox) || bbox.length !== 4) return false;
  for (const n of bbox) {
    if (typeof n !== 'number') return false;
  }
  return (
    typeof v.score === 'number' &&
    typeof v.class_id === 'number' &&
    typeof v.class_name === 'string'
  );
}

function parseDetections(raw: unknown): OnnxDetection[] {
  if (!raw || typeof raw !== 'object' || !('detections' in raw)) return [];
  const list = raw.detections;
  if (!Array.isArray(list)) return [];
  return list.filter(isDetection);
}

export class OnnxSession {
  readonly descriptor: OnnxModelDescriptor;
  private detector: YoloDetectorWasm | null = null;
  private _status: OnnxSessionStatus = 'idle';
  private _error: string | null = null;

  constructor(descriptor: OnnxModelDescriptor) {
    this.descriptor = descriptor;
  }

  get status(): OnnxSessionStatus { return this._status; }
  get error(): string | null { return this._error; }

  async init(scoreThreshold?: number, iouThreshold?: number): Promise<void> {
    if (this._status === 'loading' || this._status === 'ready') return;
    this._status = 'loading';
    this._error = null;
    try {
      await ensureWasmReady();
      const detector = new YoloDetectorWasm(this.descriptor.modelUrl, this.descriptor.targetSize);
      const score = scoreThreshold ?? this.descriptor.scoreThreshold;
      const iou = iouThreshold ?? this.descriptor.iouThreshold;
      detector.setThresholds(score, iou);
      await detector.init();
      this.detector = detector;
      this._status = 'ready';
    } catch (err) {
      this._status = 'error';
      this._error = err instanceof Error ? err.message : String(err);
      throw err;
    }
  }

  setThresholds(score: number, iou: number): void {
    this.detector?.setThresholds(score, iou);
  }

  async run(
    canvas: HTMLCanvasElement | OffscreenCanvas,
    srcW: number,
    srcH: number,
  ): Promise<OnnxResult> {
    if (!this.detector || this._status !== 'ready') {
      throw new Error(`OnnxSession(${this.descriptor.id}) not ready`);
    }
    const raw: unknown = await this.detector.detect(canvas, srcW, srcH);
    return {
      detections: parseDetections(raw),
      scoreThreshold: this.descriptor.scoreThreshold,
      iouThreshold: this.descriptor.iouThreshold,
    };
  }

  dispose(): void {
    if (!this.detector) return;
    this.detector.release();
    this.detector.free();
    this.detector = null;
    this._status = 'idle';
    this._error = null;
  }
}
