import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { Node, Edge } from '@xyflow/react';
import type { ShaderNodeData } from '../../src/types';

// Hoisted spies + configurable state used inside vi.mock factories.
const {
  mockRendererInstance,
  onnxSessionState,
  onnxOverlayState,
  onnxSessionCtor,
} = vi.hoisted(() => {
  interface OnnxSessionState {
    runResult: { detections: unknown[]; scoreThreshold: number; iouThreshold: number };
    runError: Error | null;
    initError: Error | null;
    setThresholdsSpy: ReturnType<typeof vi.fn>;
    runSpy: ReturnType<typeof vi.fn>;
    initSpy: ReturnType<typeof vi.fn>;
    disposeSpy: ReturnType<typeof vi.fn>;
  }

  interface OnnxOverlayState {
    drawSpy: ReturnType<typeof vi.fn>;
    dataUrl: string;
    texture: { needsUpdate: boolean; flipY: boolean };
    canvas: HTMLCanvasElement | null;
  }

  const rendererInst = {
    setSize: vi.fn(),
    createTarget: vi.fn((_id: string, w: number, h: number) => ({
      texture: { type: 1 },
      width: w,
      height: h,
      dispose: vi.fn(),
    })),
    getContext: vi.fn(() => ({})),
    loadImageTexture: vi.fn(() => Promise.resolve({ type: 1, image: null, dispose: vi.fn() })),
    applyTextureSampling: vi.fn(),
    loadRawTexture: vi.fn(() => ({ type: 1, dispose: vi.fn() })),
    renderSampler2DInput: vi.fn(),
    renderWithMaterial: vi.fn(),
    readTargetToDataURL: vi.fn(() => 'data:image/png;base64,mock'),
    readTargetToCanvas: vi.fn(() => document.createElement('canvas')),
    dispose: vi.fn(),
    clearResources: vi.fn(),
  };

  const sessionState: OnnxSessionState = {
    runResult: { detections: [], scoreThreshold: 0.25, iouThreshold: 0.45 },
    runError: null,
    initError: null,
    setThresholdsSpy: vi.fn(),
    runSpy: vi.fn(),
    initSpy: vi.fn(),
    disposeSpy: vi.fn(),
  };

  const overlayState: OnnxOverlayState = {
    drawSpy: vi.fn(),
    dataUrl: 'data:image/png;base64,overlay',
    texture: { needsUpdate: true, flipY: false },
    canvas: null,
  };

  const ctorSpy = vi.fn();

  return {
    mockRendererInstance: rendererInst,
    onnxSessionState: sessionState,
    onnxOverlayState: overlayState,
    onnxSessionCtor: ctorSpy,
  };
});

// three — mirror executionEngine.test.ts.
vi.mock('three', () => ({
  ShaderMaterial: class {},
  RawShaderMaterial: class {
    vertexShader = '';
    fragmentShader = '';
    uniforms: Record<string, unknown> = {};
    glslVersion = '';
  },
  GLSL3: 'GLSL3',
  WebGLRenderTarget: class {
    texture = { type: 1 };
    width = 0;
    height = 0;
    dispose = vi.fn();
  },
  Texture: class { dispose = vi.fn() },
  CanvasTexture: class {
    image: unknown;
    needsUpdate = false;
    flipY = true;
    dispose = vi.fn();
    constructor(img?: unknown) {
      this.image = img;
    }
  },
}));

vi.mock('../../src/engine/webglRenderer', () => {
  const Ctor = vi.fn(function (this: Record<string, unknown>) {
    Object.assign(this, mockRendererInstance);
  });
  return { WebGLRenderer: Ctor };
});

vi.mock('../../src/engine/shaderCompiler', () => ({
  compileNodeShader: vi.fn(() => ({
    material: { fragmentShader: 'compiled', uniforms: {} },
    upstreamSamplers: new Map(),
    preambleLines: 0,
  })),
  validateFragmentShader: vi.fn(() => null),
}));

vi.mock('../../src/engine/graphExecutor', () => ({
  topologicalSort: vi.fn((nodes: Array<{ id: string }>) => nodes.map((n) => n.id)),
}));

vi.mock('../../src/engine/onnxSession', () => {
  class OnnxSession {
    readonly descriptor: { id: string; scoreThreshold: number; iouThreshold: number };
    private _status: 'idle' | 'loading' | 'ready' | 'error' = 'idle';

    constructor(descriptor: { id: string; scoreThreshold: number; iouThreshold: number }) {
      onnxSessionCtor(descriptor);
      this.descriptor = descriptor;
    }

    get status() { return this._status; }
    get error() { return null as string | null; }

    async init(): Promise<void> {
      onnxSessionState.initSpy();
      if (onnxSessionState.initError) {
        this._status = 'error';
        throw onnxSessionState.initError;
      }
      this._status = 'ready';
    }

    async run(canvas: unknown, srcW: number, srcH: number) {
      onnxSessionState.runSpy(canvas, srcW, srcH);
      if (onnxSessionState.runError) throw onnxSessionState.runError;
      return onnxSessionState.runResult;
    }

    setThresholds(score: number, iou: number): void {
      onnxSessionState.setThresholdsSpy(score, iou);
    }

    dispose(): void {
      onnxSessionState.disposeSpy();
      this._status = 'idle';
    }
  }
  return { OnnxSession };
});

vi.mock('../../src/engine/onnxOverlay', () => ({
  drawDetectionOverlay: vi.fn((sourceCanvas: unknown, w: number, h: number, detections: unknown[]) => {
    onnxOverlayState.drawSpy(sourceCanvas, w, h, detections);
    return {
      dataUrl: onnxOverlayState.dataUrl,
      texture: onnxOverlayState.texture,
      canvas: onnxOverlayState.canvas ?? document.createElement('canvas'),
    };
  }),
}));

// Mock Image so image inputs resolve during defaultW/H detection.
class MockImage {
  onload: (() => void) | null = null;
  onerror: (() => void) | null = null;
  naturalWidth = 256;
  naturalHeight = 256;
  private _src = '';
  get src() { return this._src; }
  set src(val: string) {
    this._src = val;
    Promise.resolve().then(() => {
      if (val.includes('fail')) {
        this.onerror?.();
      } else {
        this.onload?.();
      }
    });
  }
}
vi.stubGlobal('Image', MockImage);

import { ExecutionEngine } from '../../src/engine/executionEngine';
import { topologicalSort } from '../../src/engine/graphExecutor';
import { drawDetectionOverlay } from '../../src/engine/onnxOverlay';

function makeNode(id: string, data: Partial<ShaderNodeData>): Node<ShaderNodeData> {
  return {
    id,
    type: 'custom',
    position: { x: 0, y: 0 },
    data: {
      type: 'shader',
      label: id,
      shaderCode: 'void main() {}',
      inputs: [],
      outputs: [],
      uniforms: {},
      ...data,
    },
  };
}

function onnxNode(id: string, overrides: Partial<ShaderNodeData> = {}): Node<ShaderNodeData> {
  return makeNode(id, {
    type: 'onnx',
    label: 'yolov8n',
    shaderCode: '',
    inputs: [{ id: `${id}_image`, label: 'image', dataType: 'sampler2D', direction: 'input' }],
    outputs: [
      { id: `${id}_det`, label: 'detections', dataType: 'roi', direction: 'output' },
      { id: `${id}_ov`, label: 'overlay', dataType: 'sampler2D', direction: 'output' },
    ],
    uniforms: {},
    onnxModelId: 'yolov8n',
    ...overrides,
  });
}

function imageInputNode(id: string): Node<ShaderNodeData> {
  return makeNode(id, {
    type: 'input',
    inputDataType: 'sampler2D',
    imageDataUrl: `data:image/png;base64,seed-${id}`,
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  onnxSessionState.runResult = { detections: [], scoreThreshold: 0.25, iouThreshold: 0.45 };
  onnxSessionState.runError = null;
  onnxSessionState.initError = null;
  onnxOverlayState.dataUrl = 'data:image/png;base64,overlay';
  onnxOverlayState.canvas = null;
});

describe('ExecutionEngine ONNX branch', () => {
  it('happy path: image input → onnx node emits detections, overlay dataUrl, and output size', async () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const input = imageInputNode('img_1');
    const onnx = onnxNode('onnx_1');
    const edges: Edge[] = [
      { id: 'e', source: 'img_1', target: 'onnx_1', sourceHandle: 'out', targetHandle: 'onnx_1_image' },
    ];

    const detections = [{ bbox: [0, 0, 1, 1], score: 0.9, class_id: 0, class_name: 'x' }];
    onnxSessionState.runResult = { detections, scoreThreshold: 0.25, iouThreshold: 0.45 };

    const onOutput = vi.fn();
    const onOutputData = vi.fn();
    const onOutputSize = vi.fn();
    const onNodeError = vi.fn();

    vi.mocked(topologicalSort).mockReturnValueOnce(['img_1', 'onnx_1']);
    await engine.run([input, onnx], edges, onOutput, onNodeError, onOutputSize, onOutputData);

    expect(onOutputData).toHaveBeenCalledWith('onnx_1', { detections });
    expect(onOutput).toHaveBeenCalledWith('onnx_1', 'data:image/png;base64,overlay');
    expect(onOutputSize).toHaveBeenCalledWith('onnx_1', 256, 256);
    expect(onNodeError).not.toHaveBeenCalled();
  });

  it('fbo source: reads srcW/srcH from the upstream target dims', async () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    // Use an image input to seed default sizes, then a shader node that
    // produces an FBO source.
    const input = imageInputNode('img_1');
    const shader = makeNode('shader_1', {
      type: 'shader',
      inputs: [{ id: 'in_img', label: 'inputImage', dataType: 'sampler2D', direction: 'input' }],
      outputs: [],
      shaderCode: 'void main() {}',
      width: 800,
      height: 600,
      autoSize: false,
    });
    const onnx = onnxNode('onnx_1');

    const edges: Edge[] = [
      { id: 'e1', source: 'img_1', target: 'shader_1', sourceHandle: 'out', targetHandle: 'in_img' },
      { id: 'e2', source: 'shader_1', target: 'onnx_1', sourceHandle: 'out', targetHandle: 'onnx_1_image' },
    ];

    // Ensure createTarget returns targets whose reported width/height match.
    // The default mock captures (w, h) from arguments.
    vi.mocked(topologicalSort).mockReturnValueOnce(['img_1', 'shader_1', 'onnx_1']);
    const onOutputSize = vi.fn();
    await engine.run([input, shader, onnx], edges, undefined, undefined, onOutputSize);

    // onOutputSize for onnx node reflects the FBO source dims (800×600).
    const onnxSizeCall = onOutputSize.mock.calls.find((c) => c[0] === 'onnx_1');
    expect(onnxSizeCall).toBeDefined();
    expect(onnxSizeCall?.[1]).toBe(800);
    expect(onnxSizeCall?.[2]).toBe(600);
  });

  it('image source: reads srcW/srcH from HTMLImageElement.naturalWidth/Height', async () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));

    const input = imageInputNode('img_1');
    const onnx = onnxNode('onnx_1');
    const edges: Edge[] = [
      { id: 'e', source: 'img_1', target: 'onnx_1', sourceHandle: 'out', targetHandle: 'onnx_1_image' },
    ];

    // MockImage defaults to 256×256 → executionEngine derives defaultW/H = 256
    // AND the image texture's .image is null in the mock (loadImageTexture
    // returns { image: null }), so the branch that reads from HTMLImageElement
    // exits and falls back to defaultW/H = 256.
    vi.mocked(topologicalSort).mockReturnValueOnce(['img_1', 'onnx_1']);
    const onOutputSize = vi.fn();
    await engine.run([input, onnx], edges, undefined, undefined, onOutputSize);

    const onnxSizeCall = onOutputSize.mock.calls.find((c) => c[0] === 'onnx_1');
    expect(onnxSizeCall).toBeDefined();
    expect(onnxSizeCall?.[1]).toBe(256);
    expect(onnxSizeCall?.[2]).toBe(256);
  });

  it('image source with a real HTMLImageElement uses its naturalWidth/Height', async () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));

    // Use jsdom's real HTMLImageElement so `instanceof HTMLImageElement` narrows.
    const realImg = document.createElement('img');
    Object.defineProperty(realImg, 'naturalWidth', { configurable: true, get: () => 1024 });
    Object.defineProperty(realImg, 'naturalHeight', { configurable: true, get: () => 768 });
    mockRendererInstance.loadImageTexture.mockReturnValueOnce(
      Promise.resolve({ type: 1, image: realImg, dispose: vi.fn() }),
    );

    const input = imageInputNode('img_1');
    const onnx = onnxNode('onnx_1');
    const edges: Edge[] = [
      { id: 'e', source: 'img_1', target: 'onnx_1', sourceHandle: 'out', targetHandle: 'onnx_1_image' },
    ];

    vi.mocked(topologicalSort).mockReturnValueOnce(['img_1', 'onnx_1']);
    const onOutputSize = vi.fn();
    await engine.run([input, onnx], edges, undefined, undefined, onOutputSize);

    const onnxSizeCall = onOutputSize.mock.calls.find((c) => c[0] === 'onnx_1');
    expect(onnxSizeCall?.[1]).toBe(1024);
    expect(onnxSizeCall?.[2]).toBe(768);
  });

  it('emits onNodeError when the ONNX descriptor has no sampler2D input port', async () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const onnx = onnxNode('onnx_1', { inputs: [] });

    const onNodeError = vi.fn();
    const onOutput = vi.fn();
    vi.mocked(topologicalSort).mockReturnValueOnce(['onnx_1']);
    await engine.run([onnx], [], onOutput, onNodeError);

    expect(onNodeError).toHaveBeenCalledWith('onnx_1', expect.stringContaining('missing sampler2D input'));
    expect(onOutput).not.toHaveBeenCalled();
  });

  it("emits onNodeError when the sampler2D input isn't connected", async () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const onnx = onnxNode('onnx_1');

    const onNodeError = vi.fn();
    const onOutput = vi.fn();
    vi.mocked(topologicalSort).mockReturnValueOnce(['onnx_1']);
    await engine.run([onnx], [], onOutput, onNodeError);

    expect(onNodeError).toHaveBeenCalledWith('onnx_1', expect.stringMatching(/not connected/));
    expect(onOutput).not.toHaveBeenCalled();
  });

  it('emits onNodeError with "Unknown ONNX model" for an unregistered modelId', async () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const input = imageInputNode('img_1');
    const onnx = onnxNode('onnx_1', { onnxModelId: 'not-a-real-model' });
    const edges: Edge[] = [
      { id: 'e', source: 'img_1', target: 'onnx_1', sourceHandle: 'out', targetHandle: 'onnx_1_image' },
    ];

    const onNodeError = vi.fn();
    vi.mocked(topologicalSort).mockReturnValueOnce(['img_1', 'onnx_1']);
    await engine.run([input, onnx], edges, undefined, onNodeError);

    expect(onNodeError).toHaveBeenCalledWith('onnx_1', expect.stringContaining('Unknown ONNX model'));
  });

  it('emits onNodeError when the upstream produced no texture', async () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    // Deliberately make the upstream image load fail — no texture entered
    // the map. The ONNX branch should then report the upstream by id.
    const failedInput = makeNode('img_1', {
      type: 'input',
      inputDataType: 'sampler2D',
      imageDataUrl: 'data:image/png;base64,fail',
    });
    mockRendererInstance.loadImageTexture.mockImplementationOnce(() => Promise.reject(new Error('load failed')));

    const onnx = onnxNode('onnx_1');
    const edges: Edge[] = [
      { id: 'e', source: 'img_1', target: 'onnx_1', sourceHandle: 'out', targetHandle: 'onnx_1_image' },
    ];

    const onNodeError = vi.fn();
    vi.mocked(topologicalSort).mockReturnValueOnce(['img_1', 'onnx_1']);
    await engine.run([failedInput, onnx], edges, undefined, onNodeError);

    // Two errors: one for the failed image load, one for the ONNX upstream miss.
    expect(onNodeError).toHaveBeenCalledWith('onnx_1', expect.stringContaining('produced no texture'));
  });

  it('reports session.run() failures via onNodeError and continues to the next node', async () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    onnxSessionState.runError = new Error('inference blew up');

    const input = imageInputNode('img_1');
    const onnx1 = onnxNode('onnx_1');
    const onnx2 = onnxNode('onnx_2');

    const edges: Edge[] = [
      { id: 'e1', source: 'img_1', target: 'onnx_1', sourceHandle: 'out', targetHandle: 'onnx_1_image' },
      { id: 'e2', source: 'img_1', target: 'onnx_2', sourceHandle: 'out', targetHandle: 'onnx_2_image' },
    ];

    const onNodeError = vi.fn();
    vi.mocked(topologicalSort).mockReturnValueOnce(['img_1', 'onnx_1', 'onnx_2']);
    await engine.run([input, onnx1, onnx2], edges, undefined, onNodeError);

    // Both onnx nodes error because runError is sticky.
    expect(onNodeError).toHaveBeenCalledWith('onnx_1', 'inference blew up');
    expect(onNodeError).toHaveBeenCalledWith('onnx_2', 'inference blew up');
    // Confirms that the loop continued after the first failure —
    // `run()` was invoked twice.
    expect(onnxSessionState.runSpy).toHaveBeenCalledTimes(2);
  });

  it('caches sessions by model id — two onnx nodes share one OnnxSession instance', async () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));

    const input = imageInputNode('img_1');
    const onnx1 = onnxNode('onnx_1');
    const onnx2 = onnxNode('onnx_2');

    const edges: Edge[] = [
      { id: 'e1', source: 'img_1', target: 'onnx_1', sourceHandle: 'out', targetHandle: 'onnx_1_image' },
      { id: 'e2', source: 'img_1', target: 'onnx_2', sourceHandle: 'out', targetHandle: 'onnx_2_image' },
    ];

    vi.mocked(topologicalSort).mockReturnValueOnce(['img_1', 'onnx_1', 'onnx_2']);
    await engine.run([input, onnx1, onnx2], edges);

    expect(onnxSessionCtor).toHaveBeenCalledTimes(1);
    // Both nodes ran inference on the shared session.
    expect(onnxSessionState.runSpy).toHaveBeenCalledTimes(2);
  });

  it('applies node-scoped score / iou threshold overrides via session.setThresholds', async () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));

    const input = imageInputNode('img_1');
    const onnx = onnxNode('onnx_1', {
      onnxScoreThreshold: 0.6,
      onnxIouThreshold: 0.2,
    });
    const edges: Edge[] = [
      { id: 'e', source: 'img_1', target: 'onnx_1', sourceHandle: 'out', targetHandle: 'onnx_1_image' },
    ];

    vi.mocked(topologicalSort).mockReturnValueOnce(['img_1', 'onnx_1']);
    await engine.run([input, onnx], edges);

    expect(onnxSessionState.setThresholdsSpy).toHaveBeenCalledWith(0.6, 0.2);
  });

  it('skips setThresholds when only one of the two overrides is set', async () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));

    const input = imageInputNode('img_1');
    const onnx = onnxNode('onnx_1', { onnxScoreThreshold: 0.6 });
    const edges: Edge[] = [
      { id: 'e', source: 'img_1', target: 'onnx_1', sourceHandle: 'out', targetHandle: 'onnx_1_image' },
    ];

    vi.mocked(topologicalSort).mockReturnValueOnce(['img_1', 'onnx_1']);
    await engine.run([input, onnx], edges);

    expect(onnxSessionState.setThresholdsSpy).not.toHaveBeenCalled();
  });

  it('feeds the overlay drawer with the source canvas and the detections', async () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));

    const detections = [{ bbox: [0, 0, 1, 1], score: 1, class_id: 0, class_name: 'cat' }];
    onnxSessionState.runResult = { detections, scoreThreshold: 0.25, iouThreshold: 0.45 };

    const scratchCanvas = document.createElement('canvas');
    mockRendererInstance.readTargetToCanvas.mockReturnValueOnce(scratchCanvas);

    const input = imageInputNode('img_1');
    const onnx = onnxNode('onnx_1');
    const edges: Edge[] = [
      { id: 'e', source: 'img_1', target: 'onnx_1', sourceHandle: 'out', targetHandle: 'onnx_1_image' },
    ];

    vi.mocked(topologicalSort).mockReturnValueOnce(['img_1', 'onnx_1']);
    await engine.run([input, onnx], edges);

    expect(drawDetectionOverlay).toHaveBeenCalledTimes(1);
    const callArgs = vi.mocked(drawDetectionOverlay).mock.calls[0];
    expect(callArgs[0]).toBe(scratchCanvas);
    expect(callArgs[3]).toBe(detections);
  });

  it('uses default model id when node.data.onnxModelId is undefined', async () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));

    const input = imageInputNode('img_1');
    const onnx = onnxNode('onnx_1', { onnxModelId: undefined });
    const edges: Edge[] = [
      { id: 'e', source: 'img_1', target: 'onnx_1', sourceHandle: 'out', targetHandle: 'onnx_1_image' },
    ];

    vi.mocked(topologicalSort).mockReturnValueOnce(['img_1', 'onnx_1']);
    const onNodeError = vi.fn();
    await engine.run([input, onnx], edges, undefined, onNodeError);

    // No error — the default id resolves in the registry.
    expect(onNodeError).not.toHaveBeenCalled();
    expect(onnxSessionCtor).toHaveBeenCalledTimes(1);
    // The descriptor passed to the constructor is the yolov8n descriptor.
    expect(vi.mocked(onnxSessionCtor).mock.calls[0][0]).toMatchObject({ id: 'yolov8n' });
  });

  it('stop() disposes any cached OnnxSession', async () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));

    const input = imageInputNode('img_1');
    const onnx = onnxNode('onnx_1');
    const edges: Edge[] = [
      { id: 'e', source: 'img_1', target: 'onnx_1', sourceHandle: 'out', targetHandle: 'onnx_1_image' },
    ];

    vi.mocked(topologicalSort).mockReturnValueOnce(['img_1', 'onnx_1']);
    await engine.run([input, onnx], edges);

    engine.stop();
    expect(onnxSessionState.disposeSpy).toHaveBeenCalledTimes(1);
  });
});
