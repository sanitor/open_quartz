// Shared mock module shape for `@nodes/yolo-detector`, consumed by
// `onnxSession.test.ts` and `executionEngine.onnx.test.ts` via `vi.mock`.
//
// The real bundle is a wasm-pack `--target web` output that throws on every
// export in CI (the committed stub file). Tests stand in a synchronous JS
// fake so success + failure paths are deterministic and no wasm ever loads.
//
// Because `vi.mock` factories are hoisted, the per-test state MUST be created
// via `vi.hoisted` at the top of the consuming test file. This module exposes:
//   - `YoloMockState` — the mutable state type each test file wires up.
//   - `initialYoloState()` — factory returning a fresh state (called inside
//     the test's `vi.hoisted` block).
//   - `buildYoloModuleShape(state)` — returns the `{ default, YoloDetectorWasm, … }`
//     object the `vi.mock` factory should return.

import { vi, type Mock } from 'vitest';

export interface YoloMockState {
  /** Payload the mocked `detect()` resolves with. Tests swap this per-run. */
  detectResult: unknown;
  /** When non-null, `detector.init()` rejects with this error. */
  initError: Error | null;
  /** When non-null, `__wbg_init()` rejects with this error. */
  wbgInitError: Error | null;
  /** Every `(score, iou)` pair passed to `setThresholds`, in order. */
  thresholdHistory: Array<[number, number]>;
  /** Every `(modelUrl, targetSize)` pair passed to `new YoloDetectorWasm(…)`. */
  ctorArgs: Array<[string | null | undefined, number | null | undefined]>;
  /** Method spies, exposed for call-count / arg assertions. */
  ctor: Mock;
  initSpy: Mock;
  detectSpy: Mock;
  setThresholdsSpy: Mock;
  releaseSpy: Mock;
  freeSpy: Mock;
  wbgInitSpy: Mock;
}

export function initialYoloState(): YoloMockState {
  return {
    detectResult: { detections: [] },
    initError: null,
    wbgInitError: null,
    thresholdHistory: [],
    ctorArgs: [],
    ctor: vi.fn(),
    initSpy: vi.fn(),
    detectSpy: vi.fn(),
    setThresholdsSpy: vi.fn(),
    releaseSpy: vi.fn(),
    freeSpy: vi.fn(),
    wbgInitSpy: vi.fn(),
  };
}

/**
 * Reset the mutable fields of a shared `YoloMockState` between tests without
 * replacing the object identity (so `vi.mock` factory closures still see it).
 */
export function resetYoloState(state: YoloMockState): void {
  state.detectResult = { detections: [] };
  state.initError = null;
  state.wbgInitError = null;
  state.thresholdHistory.length = 0;
  state.ctorArgs.length = 0;
  state.ctor.mockClear();
  state.initSpy.mockClear();
  state.detectSpy.mockClear();
  state.setThresholdsSpy.mockClear();
  state.releaseSpy.mockClear();
  state.freeSpy.mockClear();
  state.wbgInitSpy.mockClear();
}

export function buildYoloModuleShape(state: YoloMockState) {
  class YoloDetectorWasm {
    constructor(modelUrl?: string | null, targetSize?: number | null) {
      state.ctor(modelUrl, targetSize);
      state.ctorArgs.push([modelUrl, targetSize]);
    }

    async init(): Promise<void> {
      state.initSpy();
      if (state.initError) throw state.initError;
    }

    async detect(canvas: unknown, srcW: number, srcH: number): Promise<unknown> {
      state.detectSpy(canvas, srcW, srcH);
      return state.detectResult;
    }

    setThresholds(score: number, iou: number): void {
      state.setThresholdsSpy(score, iou);
      state.thresholdHistory.push([score, iou]);
    }

    release(): void {
      state.releaseSpy();
    }

    free(): void {
      state.freeSpy();
    }

    static captureWebgpuDevice(): void { /* no-op */ }
  }

  const __wbg_init = async (url: unknown): Promise<{ memory: object }> => {
    state.wbgInitSpy(url);
    if (state.wbgInitError) throw state.wbgInitError;
    return { memory: {} };
  };

  return {
    default: __wbg_init,
    YoloDetectorWasm,
    classNames: () => [],
    defaultModelUrl: () => '',
    defaultTargetSize: () => 640,
    initSync: () => ({ memory: {} }),
  };
}
