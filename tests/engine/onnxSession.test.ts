import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import type { OnnxDetection } from '../../src/engine/onnxSession';
import type { OnnxModelDescriptor } from '../../src/engine/onnxRegistry';
import type { YoloMockState } from '../fixtures/onnxMock';
import { buildYoloModuleShape, resetYoloState } from '../fixtures/onnxMock';

// vi.hoisted runs before all imports, so state defined here is available inside
// the vi.mock factory when it fires on the first `@nodes/yolo-detector` load.
const { yoloState } = vi.hoisted(() => {
  const state: YoloMockState = {
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
  return { yoloState: state };
});

vi.mock('@nodes/yolo-detector', () => buildYoloModuleShape(yoloState));

import { OnnxSession } from '../../src/engine/onnxSession';

function makeDescriptor(overrides: Partial<OnnxModelDescriptor> = {}): OnnxModelDescriptor {
  return {
    id: 'yolov8n',
    label: 'YOLOv8n Detector',
    modelUrl: '/models/yolov8n.onnx',
    targetSize: 640,
    scoreThreshold: 0.25,
    iouThreshold: 0.45,
    description: 'test',
    inputs: [{ id: 'in', label: 'image', dataType: 'sampler2D', direction: 'input' }],
    outputs: [
      { id: 'out_d', label: 'detections', dataType: 'roi', direction: 'output' },
      { id: 'out_o', label: 'overlay', dataType: 'sampler2D', direction: 'output' },
    ],
    ...overrides,
  };
}

describe('OnnxSession', () => {
  beforeEach(() => {
    resetYoloState(yoloState);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('is idle immediately after construction', () => {
    const session = new OnnxSession(makeDescriptor());
    expect(session.status).toBe('idle');
    expect(session.error).toBeNull();
  });

  it('exposes the descriptor it was constructed with', () => {
    const desc = makeDescriptor({ id: 'yolov8n', targetSize: 320 });
    const session = new OnnxSession(desc);
    expect(session.descriptor).toBe(desc);
  });

  describe('init()', () => {
    it('transitions idle → ready and passes descriptor url + targetSize to the wasm ctor', async () => {
      const desc = makeDescriptor({ modelUrl: '/models/x.onnx', targetSize: 512 });
      const session = new OnnxSession(desc);

      await session.init();

      expect(session.status).toBe('ready');
      expect(session.error).toBeNull();
      expect(yoloState.ctorArgs).toEqual([['/models/x.onnx', 512]]);
      expect(yoloState.initSpy).toHaveBeenCalledTimes(1);
      // wbgInitSpy call count depends on whether the module-scoped wasmReady
      // promise is already resolved from a prior test — assert only that
      // status became 'ready'.
    });

    it('uses descriptor default thresholds when no args passed', async () => {
      const desc = makeDescriptor({ scoreThreshold: 0.31, iouThreshold: 0.52 });
      const session = new OnnxSession(desc);

      await session.init();

      expect(yoloState.thresholdHistory).toEqual([[0.31, 0.52]]);
    });

    it('overrides descriptor thresholds when args are supplied', async () => {
      const session = new OnnxSession(makeDescriptor({ scoreThreshold: 0.25, iouThreshold: 0.45 }));

      await session.init(0.8, 0.1);

      expect(yoloState.thresholdHistory).toEqual([[0.8, 0.1]]);
    });

    it('is a no-op when already ready — does not re-init', async () => {
      const session = new OnnxSession(makeDescriptor());

      await session.init();
      await session.init();

      expect(yoloState.initSpy).toHaveBeenCalledTimes(1);
      expect(yoloState.ctor).toHaveBeenCalledTimes(1);
    });

    it('does not re-init while a first init is still in-flight', async () => {
      const session = new OnnxSession(makeDescriptor());

      const first = session.init();
      // Second call sees status === 'loading' and returns early.
      const second = session.init();
      await Promise.all([first, second]);

      expect(yoloState.initSpy).toHaveBeenCalledTimes(1);
    });

    it('sets status=error on detector.init() failure and re-throws', async () => {
      yoloState.initError = new Error('model fetch failed');

      const session = new OnnxSession(makeDescriptor());
      await expect(session.init()).rejects.toThrow('model fetch failed');
      expect(session.status).toBe('error');
      expect(session.error).toBe('model fetch failed');
    });

    it('stringifies a non-Error rejection reason into .error', async () => {
      // Force a non-Error rejection through the ctor path so we exercise the
      // `err instanceof Error ? err.message : String(err)` branch.
      yoloState.ctor.mockImplementationOnce(() => {
        throw 'plain string failure';
      });

      const session = new OnnxSession(makeDescriptor());
      await expect(session.init()).rejects.toBeDefined();
      expect(session.status).toBe('error');
      expect(session.error).toBe('plain string failure');
    });
  });

  describe('setThresholds()', () => {
    it('forwards to the detector when the session is ready', async () => {
      const session = new OnnxSession(makeDescriptor());

      await session.init();
      yoloState.setThresholdsSpy.mockClear();

      session.setThresholds(0.7, 0.3);
      expect(yoloState.setThresholdsSpy).toHaveBeenCalledWith(0.7, 0.3);
    });

    it('is a silent no-op when the session is uninitialized', () => {
      const session = new OnnxSession(makeDescriptor());

      expect(() => session.setThresholds(0.5, 0.4)).not.toThrow();
      expect(yoloState.setThresholdsSpy).not.toHaveBeenCalled();
    });
  });

  describe('run()', () => {
    it('throws when called before init', async () => {
      const session = new OnnxSession(makeDescriptor({ id: 'yolov8n' }));
      await expect(session.run(document.createElement('canvas'), 640, 480)).rejects.toThrow(
        /OnnxSession\(yolov8n\) not ready/,
      );
    });

    it('throws with the descriptor id after a failed init', async () => {
      yoloState.initError = new Error('boom');

      const session = new OnnxSession(makeDescriptor({ id: 'yolov8n' }));
      await session.init().catch(() => { /* expected — verify follow-up behavior */ });

      await expect(session.run(document.createElement('canvas'), 100, 100)).rejects.toThrow(
        /OnnxSession\(yolov8n\) not ready/,
      );
    });

    it('returns descriptor thresholds and passes canvas + dims through', async () => {
      const desc = makeDescriptor({ scoreThreshold: 0.11, iouThreshold: 0.22 });
      const session = new OnnxSession(desc);
      yoloState.detectResult = { detections: [] };

      await session.init();
      const canvas = document.createElement('canvas');
      const result = await session.run(canvas, 640, 360);

      expect(yoloState.detectSpy).toHaveBeenCalledWith(canvas, 640, 360);
      expect(result.scoreThreshold).toBe(0.11);
      expect(result.iouThreshold).toBe(0.22);
      expect(result.detections).toEqual([]);
    });

    it('narrows a well-formed detection payload', async () => {
      const session = new OnnxSession(makeDescriptor());

      const good: OnnxDetection = {
        bbox: [0.1, 0.2, 0.5, 0.6],
        score: 0.8,
        class_id: 3,
        class_name: 'car',
      };
      yoloState.detectResult = { detections: [good] };

      await session.init();
      const result = await session.run(document.createElement('canvas'), 100, 100);

      expect(result.detections).toHaveLength(1);
      expect(result.detections[0]).toEqual(good);
    });

    it('filters malformed entries and keeps valid ones', async () => {
      const session = new OnnxSession(makeDescriptor());

      const valid: OnnxDetection = { bbox: [0, 0, 1, 1], score: 0.5, class_id: 0, class_name: 'person' };
      yoloState.detectResult = {
        detections: [
          valid,
          // Missing required fields.
          { bbox: [0, 0, 1, 1], score: 0.5, class_id: 0 },
          // Wrong bbox length.
          { bbox: [0, 0, 1], score: 0.5, class_id: 0, class_name: 'x' },
          // Wrong bbox element type.
          { bbox: [0, 0, 1, 'nope'], score: 0.5, class_id: 0, class_name: 'x' },
          // bbox not an array.
          { bbox: 'not-an-array', score: 0.5, class_id: 0, class_name: 'x' },
          // Non-number score.
          { bbox: [0, 0, 1, 1], score: '0.5', class_id: 0, class_name: 'x' },
          // Non-number class_id.
          { bbox: [0, 0, 1, 1], score: 0.5, class_id: '0', class_name: 'x' },
          // Non-string class_name.
          { bbox: [0, 0, 1, 1], score: 0.5, class_id: 0, class_name: 123 },
          null,
          'nope',
          42,
        ],
      };

      await session.init();
      const result = await session.run(document.createElement('canvas'), 10, 10);

      expect(result.detections).toEqual([valid]);
    });

    it('returns [] when detect resolves with a non-object', async () => {
      const session = new OnnxSession(makeDescriptor());
      await session.init();

      yoloState.detectResult = null;
      expect((await session.run(document.createElement('canvas'), 10, 10)).detections).toEqual([]);

      yoloState.detectResult = undefined;
      expect((await session.run(document.createElement('canvas'), 10, 10)).detections).toEqual([]);

      yoloState.detectResult = 42;
      expect((await session.run(document.createElement('canvas'), 10, 10)).detections).toEqual([]);

      yoloState.detectResult = 'string';
      expect((await session.run(document.createElement('canvas'), 10, 10)).detections).toEqual([]);
    });

    it('returns [] when detect resolves with an object missing a detections field', async () => {
      const session = new OnnxSession(makeDescriptor());
      await session.init();

      yoloState.detectResult = { somethingElse: [1, 2, 3] };
      expect((await session.run(document.createElement('canvas'), 10, 10)).detections).toEqual([]);
    });

    it('returns [] when detections is not an array', async () => {
      const session = new OnnxSession(makeDescriptor());
      await session.init();

      yoloState.detectResult = { detections: 'not-an-array' };
      expect((await session.run(document.createElement('canvas'), 10, 10)).detections).toEqual([]);

      yoloState.detectResult = { detections: null };
      expect((await session.run(document.createElement('canvas'), 10, 10)).detections).toEqual([]);

      yoloState.detectResult = { detections: { 0: 'x' } };
      expect((await session.run(document.createElement('canvas'), 10, 10)).detections).toEqual([]);
    });
  });

  describe('dispose()', () => {
    it('calls release + free on the detector and resets to idle', async () => {
      const session = new OnnxSession(makeDescriptor());

      await session.init();
      session.dispose();

      expect(yoloState.releaseSpy).toHaveBeenCalledTimes(1);
      expect(yoloState.freeSpy).toHaveBeenCalledTimes(1);
      expect(session.status).toBe('idle');
      expect(session.error).toBeNull();
    });

    it('is idempotent — a second call is a no-op', async () => {
      const session = new OnnxSession(makeDescriptor());

      await session.init();
      session.dispose();
      session.dispose();

      expect(yoloState.releaseSpy).toHaveBeenCalledTimes(1);
      expect(yoloState.freeSpy).toHaveBeenCalledTimes(1);
    });

    it('is a no-op when the session was never initialized', () => {
      const session = new OnnxSession(makeDescriptor());

      expect(() => session.dispose()).not.toThrow();
      expect(yoloState.releaseSpy).not.toHaveBeenCalled();
      expect(yoloState.freeSpy).not.toHaveBeenCalled();
      expect(session.status).toBe('idle');
    });

    it('allows re-init after dispose', async () => {
      const session = new OnnxSession(makeDescriptor());

      await session.init();
      session.dispose();
      expect(session.status).toBe('idle');

      await session.init();
      expect(session.status).toBe('ready');
      expect(yoloState.ctor).toHaveBeenCalledTimes(2);
    });
  });
});
