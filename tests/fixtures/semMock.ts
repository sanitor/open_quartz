import { vi, type Mock } from 'vitest';

export interface SemMockState {
  segmentResult: unknown;
  initError: Error | null;
  wbgInitError: Error | null;
  ctorArgs: Array<[string | null | undefined, number | null | undefined]>;
  ctor: Mock;
  initSpy: Mock;
  segmentSpy: Mock;
  releaseSpy: Mock;
  freeSpy: Mock;
  wbgInitSpy: Mock;
}

export function initialSemState(): SemMockState {
  return {
    segmentResult: { segmentation: { mask_rgba: new Uint8Array(0), mask_w: 0, mask_h: 0, class_counts: [], num_classes: 19 } },
    initError: null,
    wbgInitError: null,
    ctorArgs: [],
    ctor: vi.fn(),
    initSpy: vi.fn(),
    segmentSpy: vi.fn(),
    releaseSpy: vi.fn(),
    freeSpy: vi.fn(),
    wbgInitSpy: vi.fn(),
  };
}

export function resetSemState(state: SemMockState): void {
  state.segmentResult = { segmentation: { mask_rgba: new Uint8Array(0), mask_w: 0, mask_h: 0, class_counts: [], num_classes: 19 } };
  state.initError = null;
  state.wbgInitError = null;
  state.ctorArgs.length = 0;
  state.ctor.mockClear();
  state.initSpy.mockClear();
  state.segmentSpy.mockClear();
  state.releaseSpy.mockClear();
  state.freeSpy.mockClear();
  state.wbgInitSpy.mockClear();
}

export function buildSemModuleShape(state: SemMockState) {
  class YoloSemWasm {
    constructor(modelUrl?: string | null, targetSize?: number | null) {
      state.ctor(modelUrl, targetSize);
      state.ctorArgs.push([modelUrl, targetSize]);
    }

    async init(): Promise<void> {
      state.initSpy();
      if (state.initError) throw state.initError;
    }

    async segment(canvas: unknown, srcW: number, srcH: number): Promise<unknown> {
      state.segmentSpy(canvas, srcW, srcH);
      return state.segmentResult;
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
    YoloSemWasm,
    classNames: () => [],
    defaultModelUrl: () => '',
    defaultTargetSize: () => 640,
    initSync: () => ({ memory: {} }),
  };
}
