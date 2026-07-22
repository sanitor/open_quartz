import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { Node, Edge } from '@xyflow/react';
import type { ShaderNodeData } from '../../src/types';

// Use vi.hoisted so mocks are available inside vi.mock factories (which are hoisted)
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
    canvas: document.createElement('canvas'),
  };
  return { mockRendererInstance: inst };
});

// Mock three
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

// Mock webglRenderer
vi.mock('../../src/engine/webglRenderer', () => {
  const Ctor = vi.fn(function (this: Record<string, unknown>) {
    Object.assign(this, mockRendererInstance);
  });
  return { WebGLRenderer: Ctor };
});

// Mock shaderCompiler
vi.mock('../../src/engine/shaderCompiler', () => ({
  compileNodeShader: vi.fn(() => ({
    material: { fragmentShader: 'compiled', uniforms: {} },
    upstreamSamplers: new Map(),
    preambleLines: 5,
  })),
  validateFragmentShader: vi.fn(() => null),
}));

// Mock graphExecutor
vi.mock('../../src/engine/graphExecutor', () => ({
  topologicalSort: vi.fn((nodes: Array<{ id: string }>) => nodes.map((n) => n.id)),
}));

// Mock Image globally
class MockImage {
  onload: (() => void) | null = null;
  onerror: (() => void) | null = null;
  naturalWidth = 256;
  naturalHeight = 256;
  private _src = '';
  get src() { return this._src; }
  set src(val: string) {
    this._src = val;
    Promise.resolve().then(() => { this.onload?.(); });
  }
}
vi.stubGlobal('Image', MockImage);

// Mock onnx modules
vi.mock('../../src/catalog/onnxRegistry', () => ({
  ONNX_MODELS: {},
  DEFAULT_ONNX_MODEL_ID: 'yolov8n',
}));
vi.mock('../../src/engine/onnxSession', () => ({
  OnnxSession: vi.fn(),
}));
vi.mock('../../src/engine/onnxOverlay', () => ({
  drawDetectionOverlay: vi.fn(),
}));

import { ExecutionEngine } from '../../src/engine/executionEngine';
import { compileNodeShader } from '../../src/engine/shaderCompiler';
import { topologicalSort } from '../../src/engine/graphExecutor';
import type { FrameInputs } from '../../src/engine/compositor';

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
    time: 1.0,
    delta: 0.016,
    frame: 60,
    date: new Float32Array([2026, 7, 9, 0]),
    mouse: new Float32Array([0, 0, 0, 0]),
    resolution: new Float32Array([512, 512, 1]),
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// prepare() with math nodes
// ---------------------------------------------------------------------------
describe('ExecutionEngine.prepare() — math nodes', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('math nodes produce no material and no render target', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const mathNode = makeNode('m1', {
      type: 'math',
      mathOp: 'add',
      inputs: [
        { id: 'in_a', label: 'a', dataType: 'auto', direction: 'input', defaultValue: 0 },
        { id: 'in_b', label: 'b', dataType: 'auto', direction: 'input', defaultValue: 0 },
      ],
      outputs: [{ id: 'out_result', label: 'result', dataType: 'auto', direction: 'output' }],
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['m1']);

    const plan = engine.prepare([mathNode], []);

    expect(plan).not.toBeNull();
    expect(plan!.materials.has('m1')).toBe(false);
    expect(plan!.targets.has('m1')).toBe(false);
  });

  it('math node records upstream port bindings in upstreamSamplerBindings', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const inputNode = makeNode('inp1', {
      type: 'input',
      inputs: [{ id: 'in_value', label: 'value', dataType: 'float', direction: 'input' }],
      uniforms: { value: 5 },
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
      { id: 'e1', source: 'inp1', target: 'm1', sourceHandle: 'out_result', targetHandle: 'in_a' },
    ];
    vi.mocked(topologicalSort).mockReturnValueOnce(['inp1', 'm1']);

    const plan = engine.prepare([inputNode, mathNode], edges);

    expect(plan).not.toBeNull();
    const bindings = plan!.upstreamSamplerBindings.get('m1');
    expect(bindings).toBeDefined();
    expect(bindings!.get('a')).toBe('inp1');
  });

  it('system source nodes are skipped — no material, no target', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const sysNode = makeNode('sys1', {
      type: 'input',
      inputMode: 'system',
      systemSource: 'time',
      inputs: [{ id: 'in_value', label: 'value', dataType: 'float', direction: 'input' }],
      outputs: [{ id: 'out_result', label: 'result', dataType: 'float', direction: 'output' }],
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['sys1']);

    const plan = engine.prepare([sysNode], []);

    expect(plan).not.toBeNull();
    expect(plan!.materials.has('sys1')).toBe(false);
    expect(plan!.targets.has('sys1')).toBe(false);
  });

  it('scalarUpstream records math-to-shader connections', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const mathNode = makeNode('m1', {
      type: 'math',
      mathOp: 'add',
      inputs: [
        { id: 'in_a', label: 'a', dataType: 'auto', direction: 'input', defaultValue: 0 },
        { id: 'in_b', label: 'b', dataType: 'auto', direction: 'input', defaultValue: 0 },
      ],
      outputs: [{ id: 'out_result', label: 'result', dataType: 'auto', direction: 'output' }],
    });
    const shaderNode = makeNode('s1', {
      type: 'shader',
      inputs: [
        { id: 'p_intensity', label: 'intensity', dataType: 'float', direction: 'input', defaultValue: 0 },
      ],
      shaderCode: 'void main(){}',
    });
    const edges: Edge[] = [
      { id: 'e1', source: 'm1', target: 's1', sourceHandle: 'out_result', targetHandle: 'p_intensity' },
    ];
    vi.mocked(topologicalSort).mockReturnValueOnce(['m1', 's1']);

    const plan = engine.prepare([mathNode, shaderNode], edges);

    expect(plan).not.toBeNull();
    // scalarUpstream maps the shader's port label to the source node id
    const scalarUp = plan!.scalarUpstream.get('s1');
    expect(scalarUp).toBeDefined();
    expect(scalarUp!.get('intensity')).toBe('m1');
  });
});

// ---------------------------------------------------------------------------
// runFrame() — math CPU eval
// ---------------------------------------------------------------------------
describe('ExecutionEngine.runFrame() — math CPU eval', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('math node reads value from upstream input node uniform', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const inputNode = makeNode('inp1', {
      type: 'input',
      inputs: [{ id: 'in_value', label: 'value', dataType: 'float', direction: 'input' }],
      uniforms: { value: 7 },
    });
    const mathNode = makeNode('m1', {
      type: 'math',
      mathOp: 'negate',
      inputs: [
        { id: 'in_a', label: 'a', dataType: 'auto', direction: 'input', defaultValue: 0 },
      ],
      outputs: [{ id: 'out_result', label: 'result', dataType: 'auto', direction: 'output' }],
    });
    const edges: Edge[] = [
      { id: 'e1', source: 'inp1', target: 'm1', sourceHandle: 'out_result', targetHandle: 'in_a' },
    ];
    vi.mocked(topologicalSort).mockReturnValueOnce(['inp1', 'm1']);

    const plan = engine.prepare([inputNode, mathNode], edges)!;
    engine.runFrame(plan, makeFrameInputs());

    // negate(7) = -7
    expect(plan.mathValues.get('m1')).toBe(-7);
  });

  it('math node reads time from upstream system source node', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const sysNode = makeNode('sys1', {
      type: 'input',
      inputMode: 'system',
      systemSource: 'time',
      inputs: [{ id: 'in_value', label: 'value', dataType: 'float', direction: 'input' }],
      outputs: [{ id: 'out_result', label: 'result', dataType: 'float', direction: 'output' }],
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
      { id: 'e1', source: 'sys1', target: 'm1', sourceHandle: 'out_result', targetHandle: 'in_a' },
    ];
    vi.mocked(topologicalSort).mockReturnValueOnce(['sys1', 'm1']);

    const plan = engine.prepare([sysNode, mathNode], edges)!;
    engine.runFrame(plan, makeFrameInputs({ time: 3.5 }));

    // multiply(time=3.5, b=2) = 7
    expect(plan.mathValues.get('m1')).toBe(7);
  });

  it('math node reads frame from upstream system source node', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const sysNode = makeNode('sys1', {
      type: 'input',
      inputMode: 'system',
      systemSource: 'frame',
      inputs: [{ id: 'in_value', label: 'value', dataType: 'float', direction: 'input' }],
      outputs: [{ id: 'out_result', label: 'result', dataType: 'float', direction: 'output' }],
    });
    const mathNode = makeNode('m1', {
      type: 'math',
      mathOp: 'divide',
      inputs: [
        { id: 'in_a', label: 'a', dataType: 'auto', direction: 'input', defaultValue: 0 },
        { id: 'in_b', label: 'b', dataType: 'auto', direction: 'input', defaultValue: 0 },
      ],
      outputs: [{ id: 'out_result', label: 'result', dataType: 'auto', direction: 'output' }],
      uniforms: { b: 10 },
    });
    const edges: Edge[] = [
      { id: 'e1', source: 'sys1', target: 'm1', sourceHandle: 'out_result', targetHandle: 'in_a' },
    ];
    vi.mocked(topologicalSort).mockReturnValueOnce(['sys1', 'm1']);

    const plan = engine.prepare([sysNode, mathNode], edges)!;
    engine.runFrame(plan, makeFrameInputs({ frame: 100 }));

    // divide(frame=100, b=10) = 10
    expect(plan.mathValues.get('m1')).toBe(10);
  });

  it('math→math chain propagates via mathValues', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const inputNode = makeNode('inp1', {
      type: 'input',
      inputs: [{ id: 'in_value', label: 'value', dataType: 'float', direction: 'input' }],
      uniforms: { value: 3 },
    });
    const math1 = makeNode('m1', {
      type: 'math',
      mathOp: 'multiply',
      inputs: [
        { id: 'in_a', label: 'a', dataType: 'auto', direction: 'input', defaultValue: 0 },
        { id: 'in_b', label: 'b', dataType: 'auto', direction: 'input', defaultValue: 0 },
      ],
      outputs: [{ id: 'out_result', label: 'result', dataType: 'auto', direction: 'output' }],
      uniforms: { b: 4 },
    });
    const math2 = makeNode('m2', {
      type: 'math',
      mathOp: 'add',
      inputs: [
        { id: 'in_a', label: 'a', dataType: 'auto', direction: 'input', defaultValue: 0 },
        { id: 'in_b', label: 'b', dataType: 'auto', direction: 'input', defaultValue: 0 },
      ],
      outputs: [{ id: 'out_result', label: 'result', dataType: 'auto', direction: 'output' }],
      uniforms: { b: 10 },
    });
    const edges: Edge[] = [
      { id: 'e1', source: 'inp1', target: 'm1', sourceHandle: 'out_result', targetHandle: 'in_a' },
      { id: 'e2', source: 'm1', target: 'm2', sourceHandle: 'out_result', targetHandle: 'in_a' },
    ];
    vi.mocked(topologicalSort).mockReturnValueOnce(['inp1', 'm1', 'm2']);

    const plan = engine.prepare([inputNode, math1, math2], edges)!;
    engine.runFrame(plan, makeFrameInputs());

    // m1: multiply(3, 4) = 12
    expect(plan.mathValues.get('m1')).toBe(12);
    // m2: add(12, 10) = 22
    expect(plan.mathValues.get('m2')).toBe(22);
  });

  it('math result injected into downstream shader uniform via scalarUpstream', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const inputNode = makeNode('inp1', {
      type: 'input',
      inputs: [{ id: 'in_value', label: 'value', dataType: 'float', direction: 'input' }],
      uniforms: { value: 5 },
    });
    const mathNode = makeNode('m1', {
      type: 'math',
      mathOp: 'multiply',
      inputs: [
        { id: 'in_a', label: 'a', dataType: 'auto', direction: 'input', defaultValue: 0 },
        { id: 'in_b', label: 'b', dataType: 'auto', direction: 'input', defaultValue: 0 },
      ],
      outputs: [{ id: 'out_result', label: 'result', dataType: 'auto', direction: 'output' }],
      uniforms: { b: 3 },
    });
    const shaderNode = makeNode('s1', {
      type: 'shader',
      inputs: [
        { id: 'p_intensity', label: 'intensity', dataType: 'float', direction: 'input', defaultValue: 0 },
      ],
      shaderCode: 'void main(){}',
    });
    const edges: Edge[] = [
      { id: 'e1', source: 'inp1', target: 'm1', sourceHandle: 'out_result', targetHandle: 'in_a' },
      { id: 'e2', source: 'm1', target: 's1', sourceHandle: 'out_result', targetHandle: 'p_intensity' },
    ];
    vi.mocked(topologicalSort).mockReturnValueOnce(['inp1', 'm1', 's1']);

    const plan = engine.prepare([inputNode, mathNode, shaderNode], edges)!;
    const material = plan.materials.get('s1')!;
    engine.runFrame(plan, makeFrameInputs());

    // math: multiply(5, 3) = 15 → injected into shader's 'intensity' uniform
    expect(plan.mathValues.get('m1')).toBe(15);
    expect(material.uniforms['intensity']).toEqual({ value: 15 });
  });
});

// ---------------------------------------------------------------------------
// isRenderableNode — tested indirectly through prepare/runFrame behavior
// ---------------------------------------------------------------------------
describe('isRenderableNode behavior', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('math nodes are not rendered (no renderWithMaterial call)', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const mathNode = makeNode('m1', {
      type: 'math',
      mathOp: 'add',
      inputs: [
        { id: 'in_a', label: 'a', dataType: 'auto', direction: 'input', defaultValue: 0 },
        { id: 'in_b', label: 'b', dataType: 'auto', direction: 'input', defaultValue: 0 },
      ],
      outputs: [{ id: 'out_result', label: 'result', dataType: 'auto', direction: 'output' }],
      uniforms: { a: 1, b: 2 },
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['m1']);

    const plan = engine.prepare([mathNode], [])!;
    engine.runFrame(plan, makeFrameInputs());

    expect(mockRendererInstance.renderWithMaterial).not.toHaveBeenCalled();
    // But math still evaluates
    expect(plan.mathValues.get('m1')).toBe(3);
  });

  it('system source nodes are not rendered (no renderWithMaterial call)', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const sysNode = makeNode('sys1', {
      type: 'input',
      inputMode: 'system',
      systemSource: 'time',
      inputs: [{ id: 'in_value', label: 'value', dataType: 'float', direction: 'input' }],
      outputs: [{ id: 'out_result', label: 'result', dataType: 'float', direction: 'output' }],
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['sys1']);

    const plan = engine.prepare([sysNode], [])!;
    engine.runFrame(plan, makeFrameInputs());

    expect(mockRendererInstance.renderWithMaterial).not.toHaveBeenCalled();
  });

  it('shader nodes are rendered (renderWithMaterial called)', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const shaderNode = makeNode('s1', {
      type: 'shader',
      inputs: [],
      shaderCode: 'void main(){}',
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['s1']);

    const plan = engine.prepare([shaderNode], [])!;
    vi.clearAllMocks();
    engine.runFrame(plan, makeFrameInputs());

    expect(mockRendererInstance.renderWithMaterial).toHaveBeenCalledTimes(1);
  });

  it('constant nodes are rendered (renderWithMaterial called)', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const constNode = makeNode('c1', {
      type: 'constant',
      inputs: [],
      shaderCode: 'void main(){}',
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['c1']);

    const plan = engine.prepare([constNode], [])!;
    vi.clearAllMocks();
    engine.runFrame(plan, makeFrameInputs());

    expect(mockRendererInstance.renderWithMaterial).toHaveBeenCalledTimes(1);
  });
});
