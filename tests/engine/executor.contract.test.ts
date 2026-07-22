import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { Node, Edge } from '@xyflow/react';
import type { ShaderNodeData } from '../../src/types';

// ---------------------------------------------------------------------------
// Mock setup — copied from executionEngine.test.ts (lines 5-135)
// ---------------------------------------------------------------------------

const { mockRendererInstance } = vi.hoisted(() => {
  const inst = {
    setSize: vi.fn(),
    createTarget: vi.fn(() => ({
      texture: { type: 1 },
      width: 512,
      height: 512,
      dispose: vi.fn(),
    })),
    getContext: vi.fn(() => ({})),
    loadImageTexture: vi.fn(() => Promise.resolve({ type: 1, dispose: vi.fn() })),
    applyTextureSampling: vi.fn(),
    loadRawTexture: vi.fn(() => ({ type: 1, dispose: vi.fn() })),
    renderSampler2DInput: vi.fn(),
    renderWithMaterial: vi.fn(),
    readTargetToDataURL: vi.fn(() => 'data:image/png;base64,mock'),
    readTargetToCanvas: vi.fn(() => document.createElement('canvas')),
    renderToScreen: vi.fn(),
    getImageTexture: vi.fn(() => undefined),
    dispose: vi.fn(),
    clearResources: vi.fn(),
    clearTarget: vi.fn(),
    canvas: document.createElement('canvas'),
  };
  return { mockRendererInstance: inst };
});

vi.mock('three', () => ({
  ShaderMaterial: class {},
  RawShaderMaterial: class {
    vertexShader = '';
    fragmentShader = '';
    uniforms: Record<string, unknown> = {};
    glslVersion = '';
    constructor(opts?: { vertexShader?: string; fragmentShader?: string; uniforms?: Record<string, unknown>; glslVersion?: string }) {
      if (opts) {
        this.vertexShader = opts.vertexShader ?? '';
        this.fragmentShader = opts.fragmentShader ?? '';
        this.uniforms = opts.uniforms ?? {};
        this.glslVersion = opts.glslVersion ?? '';
      }
    }
  },
  GLSL3: 'GLSL3',
  WebGLRenderTarget: class {
    texture = { type: 1 };
    width = 0;
    height = 0;
    dispose = vi.fn();
  },
  Texture: class { dispose = vi.fn() },
}));

vi.mock('../../src/engine/webglRenderer', () => {
  const Ctor = vi.fn(function (this: Record<string, unknown>) {
    Object.assign(this, mockRendererInstance);
  });
  return { WebGLRenderer: Ctor };
});

vi.mock('../../src/engine/shaderCompiler', () => ({
  compileNodeShader: vi.fn(
    (_code: string, _inputs: unknown[], upstreamMap?: Map<string, string>) => ({
      material: { fragmentShader: 'compiled', uniforms: {} },
      upstreamSamplers: upstreamMap ? new Map(upstreamMap) : new Map<string, string>(),
      preambleLines: 5,
      needsFeedback: false,
    }),
  ),
  validateFragmentShader: vi.fn(() => null),
}));

vi.mock('../../src/engine/graphExecutor', () => ({
  topologicalSort: vi.fn((nodes: Array<{ id: string }>) => nodes.map((n) => n.id)),
}));

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
      if (val.includes('fail')) { this.onerror?.(); }
      else { this.onload?.(); }
    });
  }
}
vi.stubGlobal('Image', MockImage);

vi.stubGlobal('atob', (b64: string) => {
  const decoded = Buffer.from(b64, 'base64').toString('binary');
  return decoded;
});

vi.mock('../../src/catalog/onnxRegistry', () => ({
  ONNX_MODELS: {
    yolov8n: { id: 'yolov8n', label: 'YOLOv8n', modelUrl: '/m.onnx', targetSize: 640, scoreThreshold: 0.25, iouThreshold: 0.45, description: '', inputs: [], outputs: [] },
  },
  DEFAULT_ONNX_MODEL_ID: 'yolov8n',
}));
vi.mock('../../src/engine/onnxOverlay', () => ({
  drawDetectionOverlay: vi.fn(() => ({
    texture: { type: 1, dispose: vi.fn() },
    dataUrl: 'data:image/png;base64,overlay',
  })),
  drawSegmentationOverlay: vi.fn(() => ({
    texture: { type: 1, dispose: vi.fn() },
    dataUrl: 'data:image/png;base64,seg',
  })),
}));

// ---------------------------------------------------------------------------
// Imports (must come after vi.mock calls)
// ---------------------------------------------------------------------------

import { ExecutionEngine, type ExecutionPlan } from '../../src/engine/executionEngine';
import { compileNodeShader } from '../../src/engine/shaderCompiler';
import { topologicalSort } from '../../src/engine/graphExecutor';
import type { FrameInputs } from '../../src/engine/compositor';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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

function makeFrameInputs(overrides: Partial<FrameInputs> = {}): FrameInputs {
  return {
    time: 0,
    delta: 0.016,
    frame: 0,
    date: new Float32Array([2026, 7, 9, 0]),
    mouse: new Float32Array([0, 0, 0, 0]),
    resolution: new Float32Array([512, 512, 1]),
    ...overrides,
  };
}

// ===========================================================================
// Shader Executor Contracts
// ===========================================================================
describe('Shader Executor Contracts', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('prepare() creates a render target with custom dimensions when autoSize=false', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const node = makeNode('s1', {
      type: 'shader',
      autoSize: false,
      width: 1024,
      height: 768,
      shaderCode: 'void main(){}',
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['s1']);

    const plan = engine.prepare([node], []);

    expect(plan).not.toBeNull();
    // createTarget should be called with the custom dimensions
    expect(mockRendererInstance.createTarget).toHaveBeenCalledWith(
      's1', 1024, 768, false, undefined,
    );
    expect(plan!.targets.has('s1')).toBe(true);
  });

  it('prepare() uses defaultW/defaultH when autoSize is true', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    // First node provides default resolution via fbWidth/fbHeight
    const fbInput = makeNode('fb1', {
      type: 'input',
      inputMode: 'framebuffer',
      fbWidth: 1920,
      fbHeight: 1080,
    });
    const shaderNode = makeNode('s1', {
      type: 'shader',
      autoSize: true,
      width: 99999, // should be ignored because autoSize is true
      height: 99999,
      shaderCode: 'void main(){}',
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['fb1', 's1']);

    const plan = engine.prepare([fbInput, shaderNode], []);

    expect(plan).not.toBeNull();
    expect(plan!.defaultW).toBe(1920);
    expect(plan!.defaultH).toBe(1080);
    // The shader target should use defaultW/defaultH, NOT the node's width/height
    expect(mockRendererInstance.createTarget).toHaveBeenCalledWith(
      's1', 1920, 1080, false, undefined,
    );
  });

  it('runFrame() calls renderWithMaterial with the shader material and its target', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const node = makeNode('s1', {
      type: 'shader',
      shaderCode: 'void main(){}',
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['s1']);
    const plan = engine.prepare([node], [])!;
    const material = plan.materials.get('s1')!;
    const target = plan.targets.get('s1')!;
    vi.clearAllMocks();

    engine.runFrame(plan, makeFrameInputs());

    expect(mockRendererInstance.renderWithMaterial).toHaveBeenCalledWith(material, target);
  });

  it('runFrame() binds upstream textures to the shader material uniforms', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const inputNode = makeNode('inp', {
      type: 'input',
      inputDataType: 'sampler2D' as never,
      imageDataUrl: 'data:image/png;base64,AA',
    });
    const shaderNode = makeNode('s1', {
      type: 'shader',
      inputs: [{ id: 'p1', label: 'tex', dataType: 'sampler2D', direction: 'input' }],
      shaderCode: 'void main(){}',
    });
    const edges: Edge[] = [
      { id: 'e1', source: 'inp', target: 's1', sourceHandle: 'out', targetHandle: 'p1' },
    ];
    vi.mocked(compileNodeShader).mockReturnValueOnce({
      material: { fragmentShader: 'code', uniforms: {} } as never,
      upstreamSamplers: new Map([['tex', 'inp']]),
      preambleLines: 0,
      needsFeedback: false,
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['inp', 's1']);

    const plan = engine.prepare([inputNode, shaderNode], edges)!;
    // Simulate upstream texture loaded
    const fakeTex = { type: 1, isFake: true };
    plan.textureSources.set('inp', { kind: 'image', texture: fakeTex as never });
    const material = plan.materials.get('s1')!;

    engine.runFrame(plan, makeFrameInputs());

    expect(material.uniforms['tex']).toEqual({ value: fakeTex });
  });

  it('runFrame() injects builtin uniforms (iTime, iResolution) when the shader declares them', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const node = makeNode('s1', {
      type: 'shader',
      inputs: [
        { id: 'p1', label: 'iTime', dataType: 'float', direction: 'input' },
        { id: 'p2', label: 'iResolution', dataType: 'vec3', direction: 'input' },
      ],
      shaderCode: 'void main(){}',
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['s1']);
    const plan = engine.prepare([node], [])!;
    const material = plan.materials.get('s1')!;

    engine.runFrame(plan, makeFrameInputs({ time: 3.14 }));

    expect(material.uniforms['iTime']).toEqual({ value: 3.14 });
    // iResolution should be the plan's per-node resolution, not the global one
    const nodeRes = plan.resolutionUniforms.get('s1')!;
    expect(material.uniforms['iResolution']).toEqual({ value: nodeRes });
  });
});

// ===========================================================================
// ONNX Executor Contracts
// ===========================================================================
describe('ONNX Executor Contracts', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('prepare() skips shader compilation for ONNX nodes', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const shaderA = makeNode('a', { type: 'shader', shaderCode: 'void main(){}' });
    const onnxNode = makeNode('ox1', {
      type: 'onnx',
      inputs: [{ id: 'p1', label: 'image', dataType: 'sampler2D', direction: 'input' }],
    });
    const edges: Edge[] = [
      { id: 'e1', source: 'a', target: 'ox1', sourceHandle: 'out', targetHandle: 'p1' },
    ];
    vi.mocked(topologicalSort).mockReturnValueOnce(['a', 'ox1']);

    const plan = engine.prepare([shaderA, onnxNode], edges);

    expect(plan).not.toBeNull();
    // compileNodeShader called once for shader 'a', never for ONNX 'ox1'
    expect(compileNodeShader).toHaveBeenCalledTimes(1);
    expect(plan!.materials.has('ox1')).toBe(false);
    expect(plan!.targets.has('ox1')).toBe(false);
    // But upstream binding is recorded
    expect(plan!.upstreamSamplerBindings.get('ox1')?.get('image')).toBe('a');
  });

  it('runFrame() skips ONNX nodes with status !== ready', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const onnxNode = makeNode('ox1', {
      type: 'onnx',
      onnxStatus: 'downloading',
      inputs: [{ id: 'p1', label: 'image', dataType: 'sampler2D', direction: 'input' }],
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['ox1']);
    const plan = engine.prepare([onnxNode], [])!;
    // Give it an upstream texture so we can verify it wasn't consumed
    plan.textureSources.set('upstream_src', { kind: 'fbo', target: { texture: { type: 1 }, width: 512, height: 512, dispose: vi.fn() } as never });
    plan.upstreamSamplerBindings.set('ox1', new Map([['image', 'upstream_src']]));
    vi.clearAllMocks();

    engine.runFrame(plan, makeFrameInputs());

    // Should not create a scratch target or read to canvas
    expect(mockRendererInstance.createTarget).not.toHaveBeenCalled();
    expect(mockRendererInstance.readTargetToCanvas).not.toHaveBeenCalled();
  });

  it('runFrame() skips ONNX nodes that are in-flight', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const shaderA = makeNode('a', { type: 'shader', shaderCode: 'void main(){}' });
    const onnxNode = makeNode('ox1', {
      type: 'onnx',
      onnxStatus: 'ready',
      inputs: [{ id: 'p1', label: 'image', dataType: 'sampler2D', direction: 'input' }],
    });
    const edges: Edge[] = [
      { id: 'e1', source: 'a', target: 'ox1', sourceHandle: 'out', targetHandle: 'p1' },
    ];
    vi.mocked(topologicalSort).mockReturnValueOnce(['a', 'ox1']);
    const plan = engine.prepare([shaderA, onnxNode], edges)!;

    // First runFrame: kicks off async inference, adding ox1 to onnxInFlight
    engine.runFrame(plan, makeFrameInputs());
    const firstCallCount = mockRendererInstance.readTargetToCanvas.mock.calls.length;
    expect(firstCallCount).toBe(1);

    // Second runFrame: ox1 is still in-flight (async not resolved), should skip
    engine.runFrame(plan, makeFrameInputs());
    expect(mockRendererInstance.readTargetToCanvas).toHaveBeenCalledTimes(1); // no new call
  });

  it('runFrame() reads upstream texture to canvas and starts async inference', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const shaderA = makeNode('a', { type: 'shader', shaderCode: 'void main(){}' });
    const onnxNode = makeNode('ox1', {
      type: 'onnx',
      inputs: [{ id: 'p1', label: 'image', dataType: 'sampler2D', direction: 'input' }],
    });
    const edges: Edge[] = [
      { id: 'e1', source: 'a', target: 'ox1', sourceHandle: 'out', targetHandle: 'p1' },
    ];
    vi.mocked(topologicalSort).mockReturnValueOnce(['a', 'ox1']);
    const plan = engine.prepare([shaderA, onnxNode], edges)!;
    vi.clearAllMocks();

    // Simulate shader 'a' rendered to FBO
    const fboTex = { type: 1 };
    const fboTarget = { texture: fboTex, width: 512, height: 512, dispose: vi.fn() };
    plan.textureSources.set('a', { kind: 'fbo', target: fboTarget as never });

    engine.runFrame(plan, makeFrameInputs());

    // Should create a scratch target at source dimensions
    expect(mockRendererInstance.createTarget).toHaveBeenCalledWith(
      'onnx_src_ox1', 512, 512, false, 'rgba8',
    );
    // Should render the upstream texture into the scratch target
    expect(mockRendererInstance.renderSampler2DInput).toHaveBeenCalledWith(
      fboTex, expect.objectContaining({ texture: expect.anything() }),
    );
    // Should read the scratch target to a canvas for ONNX inference
    expect(mockRendererInstance.readTargetToCanvas).toHaveBeenCalled();
  });

  it('ONNX output cache survives plan rebuild', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    // Seed the onnxOutputCache via bracket notation (private field)
    const cachedTex = { type: 1, fromOnnx: true };
    const onnxCache: Map<string, unknown> = engine['onnxOutputCache'];
    onnxCache.set('ox1', { kind: 'image', texture: cachedTex });

    // Build a new plan that includes the ONNX node
    const onnxNode = makeNode('ox1', {
      type: 'onnx',
      inputs: [{ id: 'p1', label: 'image', dataType: 'sampler2D', direction: 'input' }],
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['ox1']);
    const plan = engine.prepare([onnxNode], [])!;

    // Plan starts with no textureSources for ox1 (prepare doesn't set it for ONNX)
    expect(plan.textureSources.has('ox1')).toBe(false);

    // runFrame restores cached ONNX outputs into the plan
    engine.runFrame(plan, makeFrameInputs());

    expect(plan.textureSources.has('ox1')).toBe(true);
    const restored = plan.textureSources.get('ox1');
    if (restored && 'texture' in restored) {
      expect(restored.texture).toBe(cachedTex);
    } else {
      expect.unreachable('Expected texture source to be restored from cache');
    }
  });
});

// ===========================================================================
// Math Executor Contracts
// ===========================================================================
describe('Math Executor Contracts', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('prepare() records upstream bindings for math nodes', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const inputNode = makeNode('inp', {
      type: 'input',
      inputMode: 'system',
      systemSource: 'time',
    });
    const mathNode = makeNode('m1', {
      type: 'math',
      mathOp: 'multiply',
      inputs: [
        { id: 'in_a', label: 'a', dataType: 'auto', direction: 'input', defaultValue: 0 },
        { id: 'in_b', label: 'b', dataType: 'auto', direction: 'input', defaultValue: 0 },
      ],
      outputs: [{ id: 'out_result', label: 'result', dataType: 'auto', direction: 'output' }],
    });
    const edges: Edge[] = [
      { id: 'e1', source: 'inp', target: 'm1', sourceHandle: 'out', targetHandle: 'in_a' },
    ];
    vi.mocked(topologicalSort).mockReturnValueOnce(['inp', 'm1']);

    const plan = engine.prepare([inputNode, mathNode], edges);

    expect(plan).not.toBeNull();
    // upstream bindings recorded: port 'a' -> 'inp'
    const bindings = plan!.upstreamSamplerBindings.get('m1');
    expect(bindings).toBeDefined();
    expect(bindings!.get('a')).toBe('inp');
    // No material or target for math nodes
    expect(plan!.materials.has('m1')).toBe(false);
    expect(plan!.targets.has('m1')).toBe(false);
  });

  it('runFrame() computes math result and stores in plan.mathValues', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const mathNode = makeNode('m1', {
      type: 'math',
      mathOp: 'add',
      inputs: [
        { id: 'in_a', label: 'a', dataType: 'auto', direction: 'input', defaultValue: 0 },
        { id: 'in_b', label: 'b', dataType: 'auto', direction: 'input', defaultValue: 0 },
      ],
      outputs: [{ id: 'out_result', label: 'result', dataType: 'auto', direction: 'output' }],
      uniforms: { a: 7, b: 3 },
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['m1']);
    const plan = engine.prepare([mathNode], [])!;

    engine.runFrame(plan, makeFrameInputs());

    expect(plan.mathValues.get('m1')).toBe(10);
  });

  it('math chaining: A(add, a=1, b=2) → B(multiply, a=result_A, b=10) → B result is 30', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const mathA = makeNode('mA', {
      type: 'math',
      mathOp: 'add',
      inputs: [
        { id: 'in_a', label: 'a', dataType: 'auto', direction: 'input', defaultValue: 0 },
        { id: 'in_b', label: 'b', dataType: 'auto', direction: 'input', defaultValue: 0 },
      ],
      outputs: [{ id: 'out_result', label: 'result', dataType: 'auto', direction: 'output' }],
      uniforms: { a: 1, b: 2 },
    });
    const mathB = makeNode('mB', {
      type: 'math',
      mathOp: 'multiply',
      inputs: [
        { id: 'in_a', label: 'a', dataType: 'auto', direction: 'input', defaultValue: 0 },
        { id: 'in_b', label: 'b', dataType: 'auto', direction: 'input', defaultValue: 0 },
      ],
      outputs: [{ id: 'out_result', label: 'result', dataType: 'auto', direction: 'output' }],
      uniforms: { b: 10 },
    });
    const edges: Edge[] = [
      { id: 'e1', source: 'mA', target: 'mB', sourceHandle: 'out_result', targetHandle: 'in_a' },
    ];
    vi.mocked(topologicalSort).mockReturnValueOnce(['mA', 'mB']);

    const plan = engine.prepare([mathA, mathB], edges)!;
    engine.runFrame(plan, makeFrameInputs());

    expect(plan.mathValues.get('mA')).toBe(3);   // 1 + 2
    expect(plan.mathValues.get('mB')).toBe(30);   // 3 * 10
  });

  it('system source (time) feeding math: Time→Math(multiply, b=2) with time=5.0, result=10.0', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const timeNode = makeNode('time1', {
      type: 'input',
      inputMode: 'system',
      systemSource: 'time',
    });
    const mathNode = makeNode('m1', {
      type: 'math',
      mathOp: 'multiply',
      inputs: [
        { id: 'in_a', label: 'a', dataType: 'auto', direction: 'input', defaultValue: 0 },
        { id: 'in_b', label: 'b', dataType: 'auto', direction: 'input', defaultValue: 0 },
      ],
      outputs: [{ id: 'out_result', label: 'result', dataType: 'auto', direction: 'output' }],
      uniforms: { b: 2 },
    });
    const edges: Edge[] = [
      { id: 'e1', source: 'time1', target: 'm1', sourceHandle: 'out', targetHandle: 'in_a' },
    ];
    vi.mocked(topologicalSort).mockReturnValueOnce(['time1', 'm1']);

    const plan = engine.prepare([timeNode, mathNode], edges)!;
    engine.runFrame(plan, makeFrameInputs({ time: 5.0 }));

    expect(plan.mathValues.get('m1')).toBe(10.0);
  });

  it('unconnected inputs use default value from uniforms', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const mathNode = makeNode('m1', {
      type: 'math',
      mathOp: 'add',
      inputs: [
        { id: 'in_a', label: 'a', dataType: 'auto', direction: 'input', defaultValue: 0 },
        { id: 'in_b', label: 'b', dataType: 'auto', direction: 'input', defaultValue: 0 },
      ],
      outputs: [{ id: 'out_result', label: 'result', dataType: 'auto', direction: 'output' }],
      uniforms: { a: 42, b: 8 },
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['m1']);

    const plan = engine.prepare([mathNode], [])!;
    engine.runFrame(plan, makeFrameInputs());

    // Both inputs unconnected → falls back to node.data.uniforms
    expect(plan.mathValues.get('m1')).toBe(50);
  });
});

// ===========================================================================
// Renderer Executor Contracts
// ===========================================================================
describe('Renderer Executor Contracts', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('prepare() does not create a render target for renderer nodes', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const rendererNode = makeNode('r1', {
      type: 'renderer',
      inputs: [{ id: 'p1', label: 'input', dataType: 'sampler2D', direction: 'input' }],
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['r1']);

    const plan = engine.prepare([rendererNode], []);

    expect(plan).not.toBeNull();
    expect(plan!.targets.has('r1')).toBe(false);
    expect(plan!.materials.has('r1')).toBe(false);
  });

  it('prepare() records renderer in plan.outputNodes', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const shaderA = makeNode('a', { type: 'shader', shaderCode: 'void main(){}' });
    const rendererNode = makeNode('r1', {
      type: 'renderer',
      inputs: [{ id: 'p1', label: 'input', dataType: 'sampler2D', direction: 'input' }],
    });
    const edges: Edge[] = [
      { id: 'e1', source: 'a', target: 'r1', sourceHandle: 'out', targetHandle: 'p1' },
    ];
    vi.mocked(topologicalSort).mockReturnValueOnce(['a', 'r1']);

    const plan = engine.prepare([shaderA, rendererNode], edges);

    expect(plan!.outputNodes).toContain('r1');
  });

  it('renderRendererToScreen() calls renderer.renderToScreen with upstream FBO texture', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const shaderA = makeNode('a', { type: 'shader', shaderCode: 'void main(){}' });
    const rendererNode = makeNode('r1', {
      type: 'renderer',
      inputs: [{ id: 'p1', label: 'input', dataType: 'sampler2D', direction: 'input' }],
    });
    const edges: Edge[] = [
      { id: 'e1', source: 'a', target: 'r1', sourceHandle: 'out', targetHandle: 'p1' },
    ];
    vi.mocked(topologicalSort).mockReturnValueOnce(['a', 'r1']);
    const plan = engine.prepare([shaderA, rendererNode], edges)!;
    const fboTexture = { type: 1, isFBO: true };
    const fboTarget = { texture: fboTexture, width: 512, height: 512, dispose: vi.fn() };
    plan.textureSources.set('a', { kind: 'fbo', target: fboTarget as never });

    engine.renderRendererToScreen(plan, 'r1');

    expect(mockRendererInstance.renderToScreen).toHaveBeenCalledWith(fboTexture);
  });

  it('renderRendererToScreen() calls renderer.renderToScreen with upstream image texture', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const shaderA = makeNode('a', { type: 'shader', shaderCode: 'void main(){}' });
    const rendererNode = makeNode('r1', {
      type: 'renderer',
      inputs: [{ id: 'p1', label: 'input', dataType: 'sampler2D', direction: 'input' }],
    });
    const edges: Edge[] = [
      { id: 'e1', source: 'a', target: 'r1', sourceHandle: 'out', targetHandle: 'p1' },
    ];
    vi.mocked(topologicalSort).mockReturnValueOnce(['a', 'r1']);
    const plan = engine.prepare([shaderA, rendererNode], edges)!;
    const imageTex = { type: 1, isImage: true };
    plan.textureSources.set('a', { kind: 'image', texture: imageTex as never });

    engine.renderRendererToScreen(plan, 'r1');

    expect(mockRendererInstance.renderToScreen).toHaveBeenCalledWith(imageTex);
  });

  it('captureRendererScreenshot() reads target to data URL', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const shaderA = makeNode('a', { type: 'shader', shaderCode: 'void main(){}' });
    const rendererNode = makeNode('r1', {
      type: 'renderer',
      inputs: [{ id: 'p1', label: 'input', dataType: 'sampler2D', direction: 'input' }],
    });
    const edges: Edge[] = [
      { id: 'e1', source: 'a', target: 'r1', sourceHandle: 'out', targetHandle: 'p1' },
    ];
    vi.mocked(topologicalSort).mockReturnValueOnce(['a', 'r1']);
    const plan = engine.prepare([shaderA, rendererNode], edges)!;
    const fboTarget = { texture: { type: 1 }, width: 512, height: 512, dispose: vi.fn() };
    plan.textureSources.set('a', { kind: 'fbo', target: fboTarget as never });

    const result = engine.captureRendererScreenshot(plan, 'r1');

    expect(result).toBe('data:image/png;base64,mock');
    expect(mockRendererInstance.readTargetToDataURL).toHaveBeenCalledWith(fboTarget);
  });
});

// ===========================================================================
// Input/Constant Executor Contracts
// ===========================================================================
describe('Input/Constant Executor Contracts', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('image input: prepare() calls loadImageTexture and sets textureSources', async () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const inputNode = makeNode('img1', {
      type: 'input',
      inputDataType: 'sampler2D' as never,
      imageDataUrl: 'data:image/png;base64,AAAA',
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['img1']);

    const plan = engine.prepare([inputNode], []);

    expect(plan).not.toBeNull();
    // loadImageTexture is called during prepare and returns a promise
    expect(mockRendererInstance.loadImageTexture).toHaveBeenCalledWith('img1', 'data:image/png;base64,AAAA');
    // The pending texture should be tracked
    expect(plan!.pendingTextures.length).toBeGreaterThan(0);
    // After resolving, textureSources should have the image
    await Promise.all(plan!.pendingTextures);
    expect(plan!.textureSources.has('img1')).toBe(true);
    const src = plan!.textureSources.get('img1')!;
    expect(src.kind).toBe('image');
  });

  it('framebuffer input: prepare() calls loadRawTexture for framebuffer mode nodes', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const rawBytes = new Uint8Array([255, 0, 0, 255]);
    const b64 = Buffer.from(rawBytes).toString('base64');
    const fbNode = makeNode('fb1', {
      type: 'input',
      inputMode: 'framebuffer',
      rawDataUrl: `data:application/octet-stream;base64,${b64}`,
      fbFormat: 'rgba8',
      fbWidth: 4,
      fbHeight: 4,
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['fb1']);

    const plan = engine.prepare([fbNode], []);

    expect(plan).not.toBeNull();
    expect(mockRendererInstance.loadRawTexture).toHaveBeenCalled();
    expect(mockRendererInstance.renderSampler2DInput).toHaveBeenCalled();
    expect(plan!.textureSources.has('fb1')).toBe(true);
    expect(plan!.textureSources.get('fb1')!.kind).toBe('fbo');
  });

  it('constant node: prepare() compiles shader, creates target at default size', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const constNode = makeNode('c1', {
      type: 'constant',
      shaderCode: 'void main() { fragColor = vec4(1.0, 0.0, 0.0, 1.0); }',
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['c1']);

    const plan = engine.prepare([constNode], []);

    expect(plan).not.toBeNull();
    expect(compileNodeShader).toHaveBeenCalledTimes(1);
    expect(plan!.materials.has('c1')).toBe(true);
    expect(plan!.targets.has('c1')).toBe(true);
    // Default size when no resolution source nodes exist
    expect(mockRendererInstance.createTarget).toHaveBeenCalledWith(
      'c1', 512, 512, false, undefined,
    );
  });

  it('system input (time): prepare() skips shader compilation', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const timeNode = makeNode('time1', {
      type: 'input',
      inputMode: 'system',
      systemSource: 'time',
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['time1']);

    const plan = engine.prepare([timeNode], []);

    expect(plan).not.toBeNull();
    expect(compileNodeShader).not.toHaveBeenCalled();
    expect(plan!.materials.has('time1')).toBe(false);
    expect(plan!.targets.has('time1')).toBe(false);
  });

  it('video input: runFrame() picks up videoTextures from builtins', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    // Create a shader that references a video input upstream
    const shaderNode = makeNode('s1', {
      type: 'shader',
      inputs: [{ id: 'p1', label: 'vid', dataType: 'sampler2D', direction: 'input' }],
      shaderCode: 'void main(){}',
    });
    vi.mocked(compileNodeShader).mockReturnValueOnce({
      material: { fragmentShader: 'code', uniforms: {} } as never,
      upstreamSamplers: new Map([['vid', 'v1']]),
      preambleLines: 0,
      needsFeedback: false,
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['s1']);
    const plan = engine.prepare([shaderNode], [])!;
    vi.clearAllMocks();

    // Pass video texture via builtins
    const videoTex = { type: 1, isVideo: true };
    const videoTextures = new Map([['v1', videoTex as never]]);

    engine.runFrame(plan, makeFrameInputs({ videoTextures }));

    // Video texture should be injected into textureSources
    expect(plan.textureSources.get('v1')).toEqual({ kind: 'image', texture: videoTex });
    // And bound to the shader's uniform
    const material = plan.materials.get('s1')!;
    expect(material.uniforms['vid']).toEqual({ value: videoTex });
  });
});
