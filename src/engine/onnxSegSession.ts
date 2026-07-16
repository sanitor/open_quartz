import __wbg_init, { YoloSemWasm } from '@nodes/yolo-sem';

export interface SegmentationResult {
  maskRgba: Uint8Array;
  maskW: number;
  maskH: number;
  classCounts: number[];
  numClasses: number;
}

export interface SegSessionResult {
  segmentation: SegmentationResult;
}

export type SegSessionStatus = 'idle' | 'loading' | 'ready' | 'error';

const WASM_URL = new URL(
  '../../rust/crates/yolo-sem/pkg/yolo_sem_bg.wasm',
  import.meta.url,
);

let wasmReady: Promise<void> | null = null;

function ensureWasmReady(): Promise<void> {
  if (wasmReady) return wasmReady;
  const promise = __wbg_init(WASM_URL).then(() => undefined);
  wasmReady = promise;
  return promise;
}

function parseSegmentation(raw: unknown): SegmentationResult | null {
  if (!raw || typeof raw !== 'object' || !('segmentation' in raw)) return null;
  const seg = raw.segmentation;
  if (!seg || typeof seg !== 'object') return null;
  if (
    !('mask_rgba' in seg) ||
    !('mask_w' in seg) ||
    !('mask_h' in seg) ||
    !('class_counts' in seg) ||
    !('num_classes' in seg)
  ) return null;
  const maskRgba = seg.mask_rgba;
  if (!(maskRgba instanceof Uint8Array) && !Array.isArray(maskRgba)) return null;
  return {
    maskRgba: maskRgba instanceof Uint8Array ? maskRgba : new Uint8Array(maskRgba as number[]),
    maskW: Number(seg.mask_w),
    maskH: Number(seg.mask_h),
    classCounts: Array.isArray(seg.class_counts) ? (seg.class_counts as number[]) : [],
    numClasses: Number(seg.num_classes),
  };
}

export class SemSegSession {
  private segmenter: YoloSemWasm | null = null;
  private _status: SegSessionStatus = 'idle';
  private _error: string | null = null;
  private modelUrl: string;
  private targetSize: number | undefined;

  constructor(modelUrl: string, targetSize?: number) {
    this.modelUrl = modelUrl;
    this.targetSize = targetSize;
  }

  get status(): SegSessionStatus { return this._status; }
  get error(): string | null { return this._error; }

  async init(): Promise<void> {
    if (this._status === 'loading' || this._status === 'ready') return;
    this._status = 'loading';
    this._error = null;
    try {
      await ensureWasmReady();
      const segmenter = new YoloSemWasm(this.modelUrl, this.targetSize);
      await segmenter.init();
      this.segmenter = segmenter;
      this._status = 'ready';
    } catch (err) {
      this._status = 'error';
      this._error = err instanceof Error ? err.message : String(err);
      throw err;
    }
  }

  async run(
    canvas: HTMLCanvasElement | OffscreenCanvas,
    srcW: number,
    srcH: number,
  ): Promise<SegSessionResult> {
    if (!this.segmenter || this._status !== 'ready') {
      throw new Error('SemSegSession not ready');
    }
    const raw: unknown = await this.segmenter.segment(canvas, srcW, srcH);
    const seg = parseSegmentation(raw);
    if (!seg) throw new Error('Failed to parse segmentation result');
    return { segmentation: seg };
  }

  dispose(): void {
    if (!this.segmenter) return;
    this.segmenter.release();
    this.segmenter.free();
    this.segmenter = null;
    this._status = 'idle';
    this._error = null;
  }
}
