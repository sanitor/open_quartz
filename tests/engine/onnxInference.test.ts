import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

// ---------------------------------------------------------------------------
// Mock ORT on globalThis — must be set BEFORE importing the module under test
// so ensureOrtLoaded's `typeof globalThis.ort !== 'undefined'` check passes.
// ---------------------------------------------------------------------------

interface MockOrtSession {
  inputNames: string[];
  outputNames: string[];
  run: ReturnType<typeof vi.fn>;
  release: ReturnType<typeof vi.fn>;
}

function makeMockSession(overrides: Partial<MockOrtSession> = {}): MockOrtSession {
  return {
    inputNames: ['input'],
    outputNames: ['output'],
    run: vi.fn(),
    release: vi.fn(),
    ...overrides,
  };
}

let mockSession: MockOrtSession;
const mockCreate = vi.fn();

function installOrtGlobal(): void {
  mockSession = makeMockSession();
  mockCreate.mockResolvedValue(mockSession);

  (globalThis as Record<string, unknown>).ort = {
    InferenceSession: { create: mockCreate },
    Tensor: class MockTensor {
      type: string;
      data: Float32Array;
      dims: number[];
      constructor(type: string, data: Float32Array, dims: number[]) {
        this.type = type;
        this.data = data;
        this.dims = dims;
      }
    },
    env: { wasm: { wasmPaths: '', numThreads: 1 } },
  };
}

function removeOrtGlobal(): void {
  delete (globalThis as Record<string, unknown>).ort;
}

installOrtGlobal();

import {
  OnnxInferenceSession,
  runSuperResolution,
  isGpuAllocError,
  INITIAL_TILE,
  MIN_TILE,
  resetProvenTileSize,
  resetOrtLoad,
} from '../../src/engine/onnx/inference';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Create a flat RGBA image (all pixels set to the same colour). */
function makeRgba(w: number, h: number, r = 128, g = 128, b = 128, a = 255): Uint8ClampedArray {
  const buf = new Uint8ClampedArray(w * h * 4);
  for (let i = 0; i < w * h; i++) {
    buf[i * 4] = r;
    buf[i * 4 + 1] = g;
    buf[i * 4 + 2] = b;
    buf[i * 4 + 3] = a;
  }
  return buf;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('isGpuAllocError', () => {
  it.each([
    { msg: 'Failed to generate shader program', expected: true },
    { msg: 'Failed to run on executor', expected: true },
    { msg: 'JSEP kernel error', expected: true },
    { msg: 'requested buffer size too large', expected: true },
    { msg: 'GPU allocation limit exceeded', expected: true },
  ])('returns true for GPU-related error: "$msg"', ({ msg, expected }) => {
    expect(isGpuAllocError(msg)).toBe(expected);
  });

  it.each([
    'Model input shape mismatch',
    'Network request failed',
    'Invalid tensor type',
    '',
  ])('returns false for unrelated error: "%s"', (msg) => {
    expect(isGpuAllocError(msg)).toBe(false);
  });
});

describe('OnnxInferenceSession', () => {
  beforeEach(() => {
    installOrtGlobal();
    resetOrtLoad();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe('constructor state', () => {
    it('starts with no session and empty name lists', () => {
      const s = new OnnxInferenceSession();
      expect(s.inputNames).toEqual([]);
      expect(s.outputNames).toEqual([]);
      expect(s.isWasmFallback).toBe(false);
    });
  });

  describe('loadFromBuffer', () => {
    it('calls ort.InferenceSession.create with webgpu+wasm providers', async () => {
      const s = new OnnxInferenceSession();
      const buf = new ArrayBuffer(8);
      await s.loadFromBuffer(buf);

      expect(mockCreate).toHaveBeenCalledWith(buf, {
        executionProviders: ['webgpu', 'wasm'],
      });
    });

    it('populates inputNames and outputNames from the created session', async () => {
      mockSession.inputNames = ['img_input', 'mask'];
      mockSession.outputNames = ['sr_output', 'conf'];
      mockCreate.mockResolvedValue(mockSession);

      const s = new OnnxInferenceSession();
      await s.loadFromBuffer(new ArrayBuffer(4));
      expect([...s.inputNames]).toEqual(['img_input', 'mask']);
      expect([...s.outputNames]).toEqual(['sr_output', 'conf']);
    });
  });

  describe('run', () => {
    it('throws when session not loaded', async () => {
      const s = new OnnxInferenceSession();
      await expect(s.run(new Float32Array(1), [1])).rejects.toThrow(
        'OnnxInferenceSession not loaded',
      );
    });

    it('returns float32 output data from the session', async () => {
      const outputData = new Float32Array([0.1, 0.2, 0.3]);
      mockSession.run.mockResolvedValue({
        output: { data: outputData, dims: [1, 3] },
      });

      const s = new OnnxInferenceSession();
      await s.loadFromBuffer(new ArrayBuffer(4));
      const result = await s.run(new Float32Array([1, 2, 3]), [1, 3]);
      expect(result).toBe(outputData);
    });

    it('feeds the first inputName as key in the feeds record', async () => {
      mockSession.inputNames = ['my_input'];
      mockSession.outputNames = ['my_output'];
      mockCreate.mockResolvedValue(mockSession);
      mockSession.run.mockResolvedValue({
        my_output: { data: new Float32Array(1), dims: [1] },
      });

      const s = new OnnxInferenceSession();
      await s.loadFromBuffer(new ArrayBuffer(4));
      const input = new Float32Array([42]);
      await s.run(input, [1, 1]);

      const feeds = mockSession.run.mock.calls[0][0];
      expect(feeds).toHaveProperty('my_input');
      expect(feeds.my_input.data).toBe(input);
    });
  });

  describe('fallbackToWasm', () => {
    it('recreates session with wasm-only provider', async () => {
      const s = new OnnxInferenceSession();
      await s.loadFromBuffer(new ArrayBuffer(4));
      mockCreate.mockClear();

      await s.fallbackToWasm();

      expect(mockCreate).toHaveBeenCalledWith(expect.any(ArrayBuffer), {
        executionProviders: ['wasm'],
      });
    });

    it('sets isWasmFallback to true after fallback', async () => {
      const s = new OnnxInferenceSession();
      await s.loadFromBuffer(new ArrayBuffer(4));
      expect(s.isWasmFallback).toBe(false);

      await s.fallbackToWasm();
      expect(s.isWasmFallback).toBe(true);
    });

    it('releases the previous session before creating a new one', async () => {
      const firstSession = makeMockSession();
      mockCreate.mockResolvedValueOnce(firstSession);

      const s = new OnnxInferenceSession();
      await s.loadFromBuffer(new ArrayBuffer(4));

      const secondSession = makeMockSession();
      mockCreate.mockResolvedValueOnce(secondSession);

      await s.fallbackToWasm();
      expect(firstSession.release).toHaveBeenCalled();
    });

    it('is idempotent — second call is a no-op', async () => {
      const s = new OnnxInferenceSession();
      await s.loadFromBuffer(new ArrayBuffer(4));
      mockCreate.mockClear();

      await s.fallbackToWasm();
      await s.fallbackToWasm();
      // Only one create call for the fallback
      expect(mockCreate).toHaveBeenCalledTimes(1);
    });

    it('throws when no buffer was retained', async () => {
      const s = new OnnxInferenceSession();
      // loadFromUrl doesn't retain buffer — simulate by just not loading
      await expect(s.fallbackToWasm()).rejects.toThrow('Cannot fallback: no buffer retained');
    });
  });

  describe('dispose', () => {
    it('releases the session and clears state', async () => {
      mockSession.inputNames = ['a'];
      mockSession.outputNames = ['b'];
      mockCreate.mockResolvedValue(mockSession);

      const s = new OnnxInferenceSession();
      await s.loadFromBuffer(new ArrayBuffer(4));
      expect(s.inputNames).toEqual(['a']);

      s.dispose();

      expect(mockSession.release).toHaveBeenCalled();
      expect(s.inputNames).toEqual([]);
      expect(s.outputNames).toEqual([]);
    });

    it('run throws after dispose', async () => {
      const s = new OnnxInferenceSession();
      await s.loadFromBuffer(new ArrayBuffer(4));
      s.dispose();

      await expect(s.run(new Float32Array(1), [1])).rejects.toThrow(
        'OnnxInferenceSession not loaded',
      );
    });

    it('is safe to call when no session is loaded', () => {
      const s = new OnnxInferenceSession();
      expect(() => s.dispose()).not.toThrow();
    });
  });
});

describe('runSuperResolution', () => {
  const scale = 2;

  beforeEach(() => {
    installOrtGlobal();
    resetOrtLoad();
    resetProvenTileSize();
    vi.spyOn(console, 'warn').mockImplementation(() => {});
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  /**
   * Build a loaded OnnxInferenceSession whose run() returns a Float32Array
   * of the correct size (channels × outH × outW) for any input shape.
   */
  async function makeLoadedSession(
    runImpl?: (input: Float32Array, shape: number[]) => Promise<Float32Array>,
  ): Promise<OnnxInferenceSession> {
    const sess = new OnnxInferenceSession();
    // Default run: return correctly-sized zeros
    const defaultRun = async (_input: Float32Array, shape: number[]): Promise<Float32Array> => {
      const [, channels, h, w] = shape;
      return new Float32Array(channels * (h * scale) * (w * scale));
    };
    mockSession.run.mockImplementation(async (feeds: Record<string, { data: Float32Array; dims: number[] }>) => {
      const tensorIn = Object.values(feeds)[0];
      const data = tensorIn.data;
      const dims = tensorIn.dims;
      const impl = runImpl ?? defaultRun;
      const result = await impl(data, dims);
      return { output: { data: result, dims: [1, dims[1], dims[2] * scale, dims[3] * scale] } };
    });
    await sess.loadFromBuffer(new ArrayBuffer(4));
    return sess;
  }

  describe('rgb model (3 channels)', () => {
    it('produces output with dimensions = input × scale', async () => {
      const w = 16, h = 16;
      const session = await makeLoadedSession();
      const rgba = makeRgba(w, h);

      const result = await runSuperResolution(session, rgba, w, h, scale, 'rgb');

      expect(result.width).toBe(w * scale);
      expect(result.height).toBe(h * scale);
      expect(result.rgba.length).toBe(w * scale * h * scale * 4);
    });

    it('passes 3-channel input shape to session.run', async () => {
      const w = 8, h = 8;
      const session = await makeLoadedSession();
      const rgba = makeRgba(w, h);

      await runSuperResolution(session, rgba, w, h, scale, 'rgb');

      // session.run was called with feeds containing a tensor
      const call = mockSession.run.mock.calls[0][0];
      const tensor = Object.values(call)[0] as { dims: number[] };
      // Shape is [1, 3, H, W]
      expect(tensor.dims[1]).toBe(3);
    });

    it('output pixels have alpha = 255', async () => {
      const w = 8, h = 8;
      const session = await makeLoadedSession();
      const rgba = makeRgba(w, h);

      const result = await runSuperResolution(session, rgba, w, h, scale, 'rgb');
      // Check every alpha byte
      for (let i = 3; i < result.rgba.length; i += 4) {
        expect(result.rgba[i]).toBe(255);
      }
    });
  });

  describe('ycbcr model (1 channel, fixed 224)', () => {
    it('produces output with dimensions = input × scale', async () => {
      const w = 16, h = 16;
      // ycbcr uses fixedSize=224, so the run shape will be [1,1,224,224]
      mockSession.run.mockImplementation(async (feeds: Record<string, { data: Float32Array; dims: number[] }>) => {
        const tensor = Object.values(feeds)[0];
        const dims = tensor.dims;
        // output is [1,1, 224*scale, 224*scale]
        return {
          output: {
            data: new Float32Array(dims[1] * (dims[2] * scale) * (dims[3] * scale)),
            dims: [1, dims[1], dims[2] * scale, dims[3] * scale],
          },
        };
      });

      const session = new OnnxInferenceSession();
      await session.loadFromBuffer(new ArrayBuffer(4));
      const rgba = makeRgba(w, h);

      const result = await runSuperResolution(session, rgba, w, h, scale, 'ycbcr');

      expect(result.width).toBe(w * scale);
      expect(result.height).toBe(h * scale);
      expect(result.rgba.length).toBe(w * scale * h * scale * 4);
    });

    it('passes 1-channel input shape to session.run', async () => {
      const w = 16, h = 16;
      mockSession.run.mockImplementation(async (feeds: Record<string, { data: Float32Array; dims: number[] }>) => {
        const tensor = Object.values(feeds)[0];
        const dims = tensor.dims;
        return {
          output: {
            data: new Float32Array(dims[1] * (dims[2] * scale) * (dims[3] * scale)),
            dims: [1, dims[1], dims[2] * scale, dims[3] * scale],
          },
        };
      });

      const session = new OnnxInferenceSession();
      await session.loadFromBuffer(new ArrayBuffer(4));
      const rgba = makeRgba(w, h);

      await runSuperResolution(session, rgba, w, h, scale, 'ycbcr');

      const call = mockSession.run.mock.calls[0][0];
      const tensor = Object.values(call)[0] as { dims: number[] };
      // ycbcr uses 1 channel
      expect(tensor.dims[1]).toBe(1);
      // Fixed 224 spatial dims
      expect(tensor.dims[2]).toBe(224);
      expect(tensor.dims[3]).toBe(224);
    });
  });

  describe('tiling for larger images', () => {
    it('produces correct output size when image > tile size', async () => {
      // Image larger than INITIAL_TILE (64)
      const w = 100, h = 80;
      const session = await makeLoadedSession();
      const rgba = makeRgba(w, h);

      const result = await runSuperResolution(session, rgba, w, h, scale, 'rgb');

      expect(result.width).toBe(w * scale);
      expect(result.height).toBe(h * scale);
      expect(result.rgba.length).toBe(w * scale * h * scale * 4);
    });

    it('calls session.run multiple times for tiled images', async () => {
      const w = 100, h = 100;
      const session = await makeLoadedSession();
      const rgba = makeRgba(w, h);

      await runSuperResolution(session, rgba, w, h, scale, 'rgb');

      // At tile=64 with a 100×100 image, we need at least 2×2 = 4 tiles
      expect(mockSession.run.mock.calls.length).toBeGreaterThanOrEqual(4);
    });
  });

  describe('adaptive retry on GPU allocation errors', () => {
    it('halves tile size when session.run throws GPU alloc error', async () => {
      let callCount = 0;
      const tileSizesUsed: number[] = [];

      mockSession.run.mockImplementation(async (feeds: Record<string, { data: Float32Array; dims: number[] }>) => {
        callCount++;
        const tensor = Object.values(feeds)[0];
        const dims = tensor.dims;
        const h = dims[2], w = dims[3];
        tileSizesUsed.push(Math.max(w, h));

        // Fail on the first call (tile=64), succeed thereafter (tile=32)
        if (callCount === 1) {
          throw new Error('Failed to generate shader program');
        }
        return {
          output: {
            data: new Float32Array(dims[1] * (h * scale) * (w * scale)),
            dims: [1, dims[1], h * scale, w * scale],
          },
        };
      });

      const w = 16, h = 16;
      const session = new OnnxInferenceSession();
      await session.loadFromBuffer(new ArrayBuffer(4));
      const rgba = makeRgba(w, h);

      await runSuperResolution(session, rgba, w, h, scale, 'rgb');

      // First attempt at 64 (or whatever INITIAL_TILE with padding made),
      // then retries at half size
      expect(callCount).toBeGreaterThan(1);
    });

    it('falls back to WASM when tile reaches MIN_TILE and still fails', async () => {
      // Track create calls and return fresh sessions each time
      let gpuFailed = true;
      mockCreate.mockImplementation(async (_buf: ArrayBuffer, opts: { executionProviders: string[] }) => {
        const sess = makeMockSession();
        sess.run.mockImplementation(async (feeds: Record<string, { data: Float32Array; dims: number[] }>) => {
          if (gpuFailed && opts.executionProviders.includes('webgpu')) {
            throw new Error('Failed to generate shader program');
          }
          // WASM backend succeeds
          gpuFailed = false;
          const tensor = Object.values(feeds)[0];
          const dims = tensor.dims;
          return {
            output: {
              data: new Float32Array(dims[1] * (dims[2] * scale) * (dims[3] * scale)),
              dims: [1, dims[1], dims[2] * scale, dims[3] * scale],
            },
          };
        });
        return sess;
      });

      const w = 16, h = 16;
      const session = new OnnxInferenceSession();
      await session.loadFromBuffer(new ArrayBuffer(4));
      const rgba = makeRgba(w, h);

      const result = await runSuperResolution(session, rgba, w, h, scale, 'rgb');

      expect(session.isWasmFallback).toBe(true);
      expect(result.width).toBe(w * scale);
      expect(result.height).toBe(h * scale);
    });

    it('re-throws non-GPU errors without retrying', async () => {
      mockSession.run.mockRejectedValue(new Error('Model input shape mismatch'));

      const w = 8, h = 8;
      const session = new OnnxInferenceSession();
      await session.loadFromBuffer(new ArrayBuffer(4));
      const rgba = makeRgba(w, h);

      await expect(
        runSuperResolution(session, rgba, w, h, scale, 'rgb'),
      ).rejects.toThrow('Model input shape mismatch');

      // Only one run attempt — no retry
      expect(mockSession.run).toHaveBeenCalledTimes(1);
    });
  });

  describe('defaults to rgb model type', () => {
    it('uses rgb codec when modelType is omitted', async () => {
      const w = 8, h = 8;
      const session = await makeLoadedSession();
      const rgba = makeRgba(w, h);

      await runSuperResolution(session, rgba, w, h, scale);

      const call = mockSession.run.mock.calls[0][0];
      const tensor = Object.values(call)[0] as { dims: number[] };
      // rgb = 3 channels
      expect(tensor.dims[1]).toBe(3);
    });
  });
});
