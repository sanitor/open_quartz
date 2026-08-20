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

installOrtGlobal();

import {
  OnnxInferenceSession,
  isGpuAllocError,
  resetOrtLoad,
} from '../../src/engine/onnx/inference';

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
