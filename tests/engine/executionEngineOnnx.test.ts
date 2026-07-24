import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import type { Node, Edge } from '@xyflow/react';
import type { ShaderNodeData } from '../../src/types';

// ---------------------------------------------------------------------------
// Integration test: WebGPUExecutionEngine ONNX inference data flow
//
// Uses a fake backend (no real GPU) + mock ORT to verify the engine correctly:
//   1. Reads upstream texture to RGBA
//   2. Passes non-black RGBA to the inference function
//   3. Writes the output to a texture source for downstream nodes
//
// This catches the "all black" bug class where data doesn't flow through.
// ---------------------------------------------------------------------------

// Mock ORT must be installed before importing the engine
interface MockTensor {
  type: string;
  data: Float32Array;
  dims: number[];
  location: 'cpu' | 'gpu-buffer';
}

function installOrtGlobal(): void {
  (globalThis as Record<string, unknown>).ort = {
    InferenceSession: {
      create: vi.fn().mockResolvedValue({
        inputNames: ['input'],
        outputNames: ['output'],
        inputMetadata: [{ shape: [1, 3, 640, 640] }],
        outputMetadata: [{ shape: [1, 84, 8400] }],
        run: vi.fn().mockResolvedValue({
          output: {
            data: new Float32Array(84 * 8400),
            dims: [1, 84, 8400],
            location: 'cpu',
          },
        }),
        release: vi.fn(),
      }),
    },
    Tensor: class {
      type: string;
      data: Float32Array;
      dims: number[];
      location: 'cpu' | 'gpu-buffer';
      constructor(type: string, data: Float32Array, dims: number[]) {
        this.type = type;
        this.data = data;
        this.dims = dims;
        this.location = 'cpu';
      }
      async getData() { return this.data; }
    },
    env: { wasm: { wasmPaths: '', numThreads: 1 } },
  };
}

// Mock modelManager before engine import
vi.mock('../../src/store/helpers', () => ({
  modelManager: {
    loadCachedModel: vi.fn().mockResolvedValue(new ArrayBuffer(4)),
    loadLocalModel: vi.fn().mockResolvedValue(new ArrayBuffer(4)),
    cacheBuffer: vi.fn(),
  },
}));

installOrtGlobal();

import { WebGPUExecutionEngine, type BackendInterface } from '../../src/engine/executionEngine';
import type { RenderTarget, TextureHandle } from '../../src/engine/gpu/WebGPUBackend';
import { drawDetectionOverlay } from '../../src/engine/onnx/overlay';
import { COCO_CLASSES } from '../../src/engine/onnx/yoloDetectionPostprocess';
import type { Detection } from '../../src/engine/onnx/yoloDetectionPostprocess';

// Suppress diagnostic console logs
beforeEach(() => {
  vi.spyOn(console, 'warn').mockImplementation(() => {});
  vi.spyOn(console, 'error').mockImplementation(() => {});
  // Keep log visible for debugging
});

afterEach(() => {
  vi.restoreAllMocks();
});

// ---------------------------------------------------------------------------
// Fake backend — records what RGBA the engine reads from upstream textures
// ---------------------------------------------------------------------------

function makeFakeBackend(capturedReads: { rgba: Uint8ClampedArray; width: number; height: number }[]): BackendInterface {
  let targetId = 0;
  const fakeTexture = {} as GPUTexture;
  const fakeView = {} as GPUTextureView;
  const fakeSampler = {} as GPUSampler;
  const fakePipeline = {} as GPURenderPipeline;

  const makeTarget = (w: number, h: number): RenderTarget => ({
    texture: fakeTexture, view: fakeView, width: w, height: h,
    format: 'rgba8unorm', sampler: fakeSampler,
  });

  return {
    device: {} as GPUDevice,
    canvas: document.createElement('canvas'),
    setSize: vi.fn(),
    createTarget: vi.fn((_id: string, w: number, h: number) => {
      targetId++;
      return makeTarget(w, h);
    }),
    loadImageTexture: vi.fn().mockResolvedValue({} as TextureHandle),
    uploadVideoFrame: vi.fn((_nodeId: string, _video: HTMLVideoElement) => {
      const handle: TextureHandle = {
        texture: {} as GPUTexture, view: {} as GPUTextureView,
        width: 128, height: 72, sampler: {} as GPUSampler,
      };
      return handle;
    }),
    readTargetToRgba: vi.fn(async (target: RenderTarget) => {
      // Return a non-black RGBA so we can verify the engine passes real data
      const rgba = new Uint8ClampedArray(target.width * target.height * 4);
      for (let i = 0; i < rgba.length; i += 4) {
        rgba[i] = 200;     // R
        rgba[i + 1] = 100; // G
        rgba[i + 2] = 50;  // B
        rgba[i + 3] = 255; // A
      }
      const result = { rgba, width: target.width, height: target.height };
      capturedReads.push(result);
      return result;
    }),
    writeRgbaToTarget: vi.fn(),
    renderPass: vi.fn(),
    blitTexture: vi.fn(),
    renderToScreen: vi.fn(),
    clearTarget: vi.fn(),
    readTargetToDataURL: vi.fn().mockResolvedValue('data:image/png;base64,mock'),
    createShaderPipeline: vi.fn().mockReturnValue(fakePipeline),
    clearResources: vi.fn(),
    dispose: vi.fn(),
  };
}

// ---------------------------------------------------------------------------
// Graph builders — minimal video → shader → onnx → renderer
// ---------------------------------------------------------------------------

function makeVideoNode(id: string): Node<ShaderNodeData> {
  return {
    id,
    type: 'default',
    position: { x: 0, y: 0 },
    data: {
      type: 'input',
      label: 'Video',
      inputMode: 'video',
      inputDataType: 'sampler2D',
      shaderCode: '',
      inputs: [],
      outputs: [{ id: 'out', label: 'output', dataType: 'sampler2D', direction: 'output' }],
      uniforms: {},
    },
  };
}

function makeShaderNode(id: string): Node<ShaderNodeData> {
  return {
    id,
    type: 'default',
    position: { x: 100, y: 0 },
    data: {
      type: 'shader',
      label: 'Resample',
      shaderTemplateId: 'resample',
      shaderCode: '@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f { return textureSample(inputImage, inputImageSampler, v_uv); }',
      inputs: [{ id: 'in', label: 'inputImage', dataType: 'sampler2D', direction: 'input' }],
      outputs: [{ id: 'out', label: 'output', dataType: 'sampler2D', direction: 'output' }],
      uniforms: {},
      autoSize: true,
    },
  };
}

function makeOnnxNode(id: string): Node<ShaderNodeData> {
  return {
    id,
    type: 'default',
    position: { x: 200, y: 0 },
    data: {
      type: 'onnx',
      label: 'YOLOv8n',
      onnxSource: 'catalog',
      onnxCatalogId: 'yolov8n',
      onnxStatus: 'ready',
      shaderCode: '',
      inputs: [{ id: 'in', label: 'image', dataType: 'sampler2D', direction: 'input' }],
      outputs: [
        { id: 'det', label: 'detections', dataType: 'roi', direction: 'output' },
        { id: 'overlay', label: 'overlay', dataType: 'sampler2D', direction: 'output' },
      ],
      uniforms: {},
    },
  };
}

function makeEdge(source: string, target: string, targetHandle?: string): Edge {
  return { id: `e-${source}-${target}`, source, target, targetHandle };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('WebGPUExecutionEngine ONNX data flow', () => {
  it('reads upstream texture RGBA and passes it to inference (non-black)', async () => {
    const capturedReads: { rgba: Uint8ClampedArray; width: number; height: number }[] = [];
    const engine = new WebGPUExecutionEngine();
    const backend = makeFakeBackend(capturedReads);
    engine.initWithBackend(backend);

    const nodes = [makeVideoNode('input_1'), makeShaderNode('shader_3'), makeOnnxNode('onnx_2')];
    const edges = [makeEdge('input_1', 'shader_3', 'in'), makeEdge('shader_3', 'onnx_2', 'in')];

    const callbacks = {
      onNodeError: vi.fn(),
      onOutputSize: vi.fn(),
      onOutputData: vi.fn(),
      onOnnxComplete: vi.fn(),
      onBackendDetected: vi.fn(),
    };

    const plan = engine.prepare(nodes, edges, callbacks.onNodeError, callbacks.onOutputSize, callbacks.onOutputData, undefined, undefined, undefined, callbacks.onBackendDetected);
    expect(plan).not.toBeNull();

    // Simulate runFrame: set up a video texture source + shader target
    if (!plan) return;

    // Video input — simulating uploadVideoFrame
    const fakeHandle: TextureHandle = {
      texture: {} as GPUTexture, view: {} as GPUTextureView,
      width: 128, height: 72, sampler: {} as GPUSampler,
    };
    plan.textureSources.set('input_1', { kind: 'image', handle: fakeHandle });

    // Shader output — simulating renderPass having run
    const shaderTarget = backend.createTarget('shader_3', 128, 72);
    plan.textureSources.set('shader_3', { kind: 'target', target: shaderTarget });

    // Mock the ORT session run to return a YOLO output with a detection
    const ortGlobal = (globalThis as Record<string, unknown>).ort as {
      InferenceSession: { create: ReturnType<typeof vi.fn> };
    };
    const mockSession = await ortGlobal.InferenceSession.create.mock.results[0]?.value;
    if (mockSession) {
      mockSession.run.mockImplementation(async () => {
        // Return [1, 84, 8400] with one detection (person, score 0.9)
        const raw = new Float32Array(84 * 8400);
        raw[0 * 8400 + 0] = 64;   // cx
        raw[1 * 8400 + 0] = 36;   // cy
        raw[2 * 8400 + 0] = 40;   // w
        raw[3 * 8400 + 0] = 60;   // h
        raw[(4 + 0) * 8400 + 0] = 0.9; // class 0 (person) score
        return { output: { data: raw, dims: [1, 84, 8400], location: 'cpu' } };
      });
    }

    // Mock canvas for overlay
    const mockCtx = {
      drawImage: vi.fn(),
      strokeRect: vi.fn(),
      fillRect: vi.fn(),
      fillText: vi.fn(),
      getImageData: vi.fn(() => ({ data: new Uint8ClampedArray(128 * 72 * 4).fill(200) })),
      putImageData: vi.fn(),
      measureText: vi.fn(() => ({ width: 50 })),
    };
    vi.spyOn(document, 'createElement').mockImplementation((tag: string) => {
      if (tag === 'canvas') {
        return {
          width: 0, height: 0,
          getContext: vi.fn(() => mockCtx),
          toDataURL: vi.fn(() => 'data:image/png;base64,mock'),
        } as unknown as HTMLElement;
      }
      return {} as HTMLElement;
    });

    // Trigger ONNX inference
    const fakeVideo = document.createElement('video') as HTMLVideoElement;
    const builtins = {
      time: 0, delta: 0, frame: 1, date: new Float32Array(4),
      mouse: new Float32Array(4), resolution: new Float32Array([128, 72, 1]),
      videoElements: new Map([['input_1', fakeVideo]]),
    };

    engine.runFrame(plan, builtins);

    // Wait for async inference to complete
    await vi.waitFor(() => {
      expect(capturedReads.length).toBeGreaterThan(0);
    });

    // Verify the engine read the upstream texture and it was non-black
    expect(capturedReads.length).toBe(1);
    expect(capturedReads[0].rgba[0]).toBeGreaterThan(0); // R channel has data
    expect(capturedReads[0].rgba[3]).toBe(255);          // alpha = 255
  });

  it('re-runs inference on frame 2 when upstream is video (does not cache black result)', async () => {
    const capturedReads: { rgba: Uint8ClampedArray; width: number; height: number }[] = [];
    const engine = new WebGPUExecutionEngine();
    const backend = makeFakeBackend(capturedReads);
    engine.initWithBackend(backend);

    const nodes = [makeVideoNode('input_1'), makeShaderNode('shader_3'), makeOnnxNode('onnx_2')];
    const edges = [makeEdge('input_1', 'shader_3', 'in'), makeEdge('shader_3', 'onnx_2', 'in')];

    const callbacks = {
      onNodeError: vi.fn(),
      onOutputSize: vi.fn(),
      onOutputData: vi.fn(),
      onOnnxComplete: vi.fn(),
      onBackendDetected: vi.fn(),
    };

    const plan = engine.prepare(nodes, edges, callbacks.onNodeError, callbacks.onOutputSize, callbacks.onOutputData, undefined, undefined, undefined, callbacks.onBackendDetected);
    if (!plan) return;

    // Manually set shader_3's upstream binding (compileWgslShader needs real GPU)
    const shaderBindings = new Map<string, string>();
    shaderBindings.set('inputImage', 'input_1');
    plan.upstreamSamplerBindings.set('shader_3', shaderBindings);

    const fakeVideo = document.createElement('video') as HTMLVideoElement;
    const builtins = {
      time: 0, delta: 0, frame: 1, date: new Float32Array(4),
      mouse: new Float32Array(4), resolution: new Float32Array([128, 72, 1]),
      videoElements: new Map([['input_1', fakeVideo]]),
    };

    // Pre-set shader_3's render target (simulates renderPass having run)
    const shaderTarget = backend.createTarget('shader_3', 128, 72);
    plan.textureSources.set('shader_3', { kind: 'target', target: shaderTarget });

    // Frame 1
    engine.runFrame(plan, builtins);

    // Wait for first inference to complete
    await vi.waitFor(() => {
      expect(capturedReads.length).toBeGreaterThanOrEqual(1);
    });

    // Frame 2: video is now available — must re-infer, not skip
    builtins.frame = 2;
    engine.runFrame(plan, builtins);

    // Should trigger a second read (re-inference), not skip due to cache
    await vi.waitFor(() => {
      expect(capturedReads.length).toBeGreaterThanOrEqual(2);
    });
  });
});
