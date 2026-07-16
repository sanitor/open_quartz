import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import type { SemMockState } from '../fixtures/semMock';
import { buildSemModuleShape, resetSemState } from '../fixtures/semMock';

const { semState } = vi.hoisted(() => {
  const state: SemMockState = {
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
  return { semState: state };
});

vi.mock('@nodes/yolo-sem', () => buildSemModuleShape(semState));

import { SemSegSession } from '../../src/engine/onnxSegSession';

describe('SemSegSession', () => {
  beforeEach(() => {
    resetSemState(semState);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('is idle immediately after construction', () => {
    const session = new SemSegSession('/models/test.onnx');
    expect(session.status).toBe('idle');
    expect(session.error).toBeNull();
  });

  describe('init()', () => {
    it('transitions idle → ready and passes model url + target size to ctor', async () => {
      const session = new SemSegSession('/models/sem.onnx', 512);
      await session.init();

      expect(session.status).toBe('ready');
      expect(session.error).toBeNull();
      expect(semState.ctorArgs).toEqual([['/models/sem.onnx', 512]]);
      expect(semState.initSpy).toHaveBeenCalledTimes(1);
    });

    it('is a no-op when already ready', async () => {
      const session = new SemSegSession('/models/sem.onnx');
      await session.init();
      await session.init();

      expect(semState.initSpy).toHaveBeenCalledTimes(1);
      expect(semState.ctor).toHaveBeenCalledTimes(1);
    });

    it('does not re-init while first init is in-flight', async () => {
      const session = new SemSegSession('/models/sem.onnx');
      const first = session.init();
      const second = session.init();
      await Promise.all([first, second]);

      expect(semState.initSpy).toHaveBeenCalledTimes(1);
    });

    it('sets status=error on init failure and re-throws', async () => {
      semState.initError = new Error('model fetch failed');
      const session = new SemSegSession('/models/sem.onnx');

      await expect(session.init()).rejects.toThrow('model fetch failed');
      expect(session.status).toBe('error');
      expect(session.error).toBe('model fetch failed');
    });

    it('stringifies a non-Error rejection into .error', async () => {
      semState.ctor.mockImplementationOnce(() => {
        throw 'plain string failure';
      });

      const session = new SemSegSession('/models/sem.onnx');
      await expect(session.init()).rejects.toBeDefined();
      expect(session.status).toBe('error');
      expect(session.error).toBe('plain string failure');
    });
  });

  describe('run()', () => {
    it('throws when called before init', async () => {
      const session = new SemSegSession('/models/sem.onnx');
      await expect(session.run(document.createElement('canvas'), 640, 480)).rejects.toThrow(
        /not ready/,
      );
    });

    it('returns parsed segmentation result', async () => {
      const mask = new Uint8Array([0, 0, 0, 192, 255, 0, 0, 192]);
      semState.segmentResult = {
        segmentation: {
          mask_rgba: mask,
          mask_w: 2,
          mask_h: 1,
          class_counts: [1, 1],
          num_classes: 2,
        },
      };

      const session = new SemSegSession('/models/sem.onnx');
      await session.init();
      const result = await session.run(document.createElement('canvas'), 100, 100);

      expect(semState.segmentSpy).toHaveBeenCalledTimes(1);
      expect(result.segmentation.maskW).toBe(2);
      expect(result.segmentation.maskH).toBe(1);
      expect(result.segmentation.numClasses).toBe(2);
      expect(result.segmentation.classCounts).toEqual([1, 1]);
      expect(result.segmentation.maskRgba).toBeInstanceOf(Uint8Array);
    });

    it('throws when segment returns malformed result', async () => {
      semState.segmentResult = { somethingElse: 42 };
      const session = new SemSegSession('/models/sem.onnx');
      await session.init();

      await expect(session.run(document.createElement('canvas'), 10, 10)).rejects.toThrow(
        /parse segmentation/,
      );
    });

    it('throws when segment returns null', async () => {
      semState.segmentResult = null;
      const session = new SemSegSession('/models/sem.onnx');
      await session.init();

      await expect(session.run(document.createElement('canvas'), 10, 10)).rejects.toThrow(
        /parse segmentation/,
      );
    });

    it('handles class_counts as a plain array from serde', async () => {
      semState.segmentResult = {
        segmentation: {
          mask_rgba: new Uint8Array(4),
          mask_w: 1,
          mask_h: 1,
          class_counts: [5, 3, 0],
          num_classes: 3,
        },
      };

      const session = new SemSegSession('/models/sem.onnx');
      await session.init();
      const result = await session.run(document.createElement('canvas'), 10, 10);
      expect(result.segmentation.classCounts).toEqual([5, 3, 0]);
    });

    it('converts mask_rgba array to Uint8Array if needed', async () => {
      semState.segmentResult = {
        segmentation: {
          mask_rgba: [0, 0, 0, 192],
          mask_w: 1,
          mask_h: 1,
          class_counts: [1],
          num_classes: 1,
        },
      };

      const session = new SemSegSession('/models/sem.onnx');
      await session.init();
      const result = await session.run(document.createElement('canvas'), 10, 10);
      expect(result.segmentation.maskRgba).toBeInstanceOf(Uint8Array);
    });
  });

  describe('dispose()', () => {
    it('calls release + free and resets to idle', async () => {
      const session = new SemSegSession('/models/sem.onnx');
      await session.init();
      session.dispose();

      expect(semState.releaseSpy).toHaveBeenCalledTimes(1);
      expect(semState.freeSpy).toHaveBeenCalledTimes(1);
      expect(session.status).toBe('idle');
      expect(session.error).toBeNull();
    });

    it('is idempotent', async () => {
      const session = new SemSegSession('/models/sem.onnx');
      await session.init();
      session.dispose();
      session.dispose();

      expect(semState.releaseSpy).toHaveBeenCalledTimes(1);
      expect(semState.freeSpy).toHaveBeenCalledTimes(1);
    });

    it('is a no-op when never initialized', () => {
      const session = new SemSegSession('/models/sem.onnx');
      expect(() => session.dispose()).not.toThrow();
      expect(semState.releaseSpy).not.toHaveBeenCalled();
    });

    it('allows re-init after dispose', async () => {
      const session = new SemSegSession('/models/sem.onnx');
      await session.init();
      session.dispose();
      expect(session.status).toBe('idle');

      await session.init();
      expect(session.status).toBe('ready');
      expect(semState.ctor).toHaveBeenCalledTimes(2);
    });
  });
});
