import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ShaderNodeData } from '../../src/types';

const mocks = vi.hoisted(() => ({
  loadOnnxModel: vi.fn(async () => new ArrayBuffer(8)),
  sdk: {
    planBrowserOnnxTask: vi.fn(),
    encodeBrowserOnnxInput: vi.fn(),
    decodeBrowserOnnxOutput: vi.fn(),
    buildBrowserOnnxCompletion: vi.fn(),
  },
}));

vi.mock('../../src/sdk/internal/OnnxResourceRegistry', () => ({ loadOnnxModel: mocks.loadOnnxModel }));
vi.mock('../../src/sdk/runtime', () => ({ requireSdk: () => mocks.sdk }));

import { BrowserInferenceProvider } from '../../src/sdk/internal/BrowserInferenceProvider';

class MockTensor {
  type: string;
  data: Float32Array;
  dims: number[];

  constructor(type: string, data: Float32Array, dims: number[]) {
    this.type = type;
    this.data = data;
    this.dims = dims;
  }
}

function installOrt(run = vi.fn()): void {
  (globalThis as Record<string, unknown>).ort = {
    Tensor: MockTensor,
    InferenceSession: {
      create: vi.fn(async () => ({
        inputNames: ['input'],
        outputNames: ['output'],
        inputMetadata: [{ shape: [1, 3, 'h', 'w'] }],
        outputMetadata: [{ shape: [1, 3, 'h', 'w'] }],
        run,
        release: vi.fn(),
      })),
    },
    env: { wasm: { wasmPaths: '', numThreads: 1 } },
  };
}

function node(): ShaderNodeData {
  return {
    type: 'onnx',
    label: 'Custom',
    shaderCode: '',
    inputs: [{ id: 'in', label: 'image', dataType: 'sampler2D', direction: 'input' }],
    outputs: [
      { id: 'out', label: 'image', dataType: 'sampler2D', direction: 'output' },
      { id: 'meta', label: 'metadata', dataType: 'json', direction: 'output' },
    ],
    uniforms: {},
    onnxModelId: 'custom-model',
  };
}

function player(overrides: Partial<Record<'submitCompletion', (completion: unknown) => void>> = {}) {
  return {
    outputInfo: vi.fn(() => ({ width: 1, height: 1 })),
    readOutputRgba: vi.fn(async () => new Uint8Array([10, 20, 30, 255])),
    uploadRgba: vi.fn(),
    submitCompletion: vi.fn(overrides.submitCompletion ?? (() => undefined)),
  };
}

describe('BrowserInferenceProvider Rust ONNX bridge', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    installOrt(vi.fn(async () => ({
      output: { data: new Float32Array([0.25, 0.5, 0.75]), dims: [1, 3, 1, 1] },
    })));
    mocks.sdk.planBrowserOnnxTask.mockReturnValue({
      task: 'generic',
      family: 'generic-rgb',
      execution: {
        mode: 'single',
        input: { shape: [1, 3, 1, 1] },
        output: { shape: [1, 3, 1, 1] },
      },
    });
    mocks.sdk.encodeBrowserOnnxInput.mockReturnValue({
      tensor: [1, 0.5, 0],
      descriptor: { shape: [1, 3, 1, 1] },
    });
    mocks.sdk.decodeBrowserOnnxOutput.mockReturnValue({
      rgba: [64, 128, 191, 255],
      width: 1,
      height: 1,
      data: { ok: true },
    });
    mocks.sdk.buildBrowserOnnxCompletion.mockImplementation((request) => ({
      nodeId: request.nodeId,
      graphRevision: request.graphRevision,
      nodeGeneration: request.nodeGeneration,
      inputStamp: request.inputStamp,
      outputs: request.outputs,
      data: request.data,
    }));
  });

  it('uses Rust tensor descriptors for ORT marshalling and Rust completion envelopes', async () => {
    const provider = new BrowserInferenceProvider();
    provider.reconcile([{ id: 'onnx', data: node() }]);
    const mockPlayer = player();

    await provider.execute({
      nodeId: 'onnx',
      textureInputs: { in: 'source' },
      graphRevision: 4,
      nodeGeneration: 2,
      inputStamp: { epoch: 1, frame: 3, timelineNs: 5, deadlineNs: 8 },
    }, mockPlayer as never, vi.fn());

    expect(mocks.sdk.planBrowserOnnxTask).toHaveBeenCalledWith(expect.objectContaining({
      modelId: 'custom-model',
      task: 'generic',
      sourceWidth: 1,
      sourceHeight: 1,
    }));
    const ortSession = await ((globalThis.ort as never) as { InferenceSession: { create: ReturnType<typeof vi.fn> } }).InferenceSession.create.mock.results[0].value;
    const feeds = ortSession.run.mock.calls[0][0];
    expect(feeds.input.dims).toEqual([1, 3, 1, 1]);
    expect(mockPlayer.uploadRgba).toHaveBeenCalledWith('onnx', new Uint8Array([64, 128, 191, 255]), 1, 1);
    expect(mockPlayer.submitCompletion).toHaveBeenCalledWith(expect.objectContaining({
      nodeId: 'onnx',
      graphRevision: 4,
      nodeGeneration: 2,
      inputStamp: { epoch: 1, frame: 3, timelineNs: 5, deadlineNs: 8 },
      data: { ok: true },
    }));
  });

  it('propagates stale completion rejection from the Rust player boundary', async () => {
    const provider = new BrowserInferenceProvider();
    provider.reconcile([{ id: 'onnx', data: node() }]);
    const mockPlayer = player({
      submitCompletion: (completion) => {
        if ((completion as { graphRevision: number }).graphRevision !== 9) {
          throw new Error('Async completion graph revision is stale');
        }
      },
    });

    await expect(provider.execute({
      nodeId: 'onnx',
      textureInputs: { in: 'source' },
      graphRevision: 4,
      nodeGeneration: 2,
      inputStamp: { epoch: 1, frame: 3, timelineNs: 5, deadlineNs: 8 },
    }, mockPlayer as never, vi.fn())).rejects.toThrow('Async completion graph revision is stale');
  });
});
