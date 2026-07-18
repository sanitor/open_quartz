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
    clearTarget: vi.fn(),
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

// Mock webglRenderer — vi.fn wrapping a function constructor so vi.mocked() works
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
    needsFeedback: false,
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

  get src() {
    return this._src;
  }
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

// Mock atob for framebuffer mode
vi.stubGlobal('atob', (b64: string) => {
  const decoded = Buffer.from(b64, 'base64').toString('binary');
  return decoded;
});

// Mock onnx modules
vi.mock('../../src/engine/onnxRegistry', () => ({
  ONNX_MODELS: {
    yolov8n: { id: 'yolov8n', label: 'YOLOv8n', modelUrl: '/m.onnx', targetSize: 640, scoreThreshold: 0.25, iouThreshold: 0.45, description: '', inputs: [], outputs: [] },
  },
  DEFAULT_ONNX_MODEL_ID: 'yolov8n',
}));
vi.mock('../../src/engine/onnxSession', () => ({
  OnnxSession: vi.fn(),
}));
vi.mock('../../src/engine/onnxOverlay', () => ({
  drawDetectionOverlay: vi.fn(() => ({
    texture: { type: 1, dispose: vi.fn() },
    dataUrl: 'data:image/png;base64,overlay',
  })),
}));

import { ExecutionEngine, type ExecutionPlan } from '../../src/engine/executionEngine';
import { WebGLRenderer } from '../../src/engine/webglRenderer';
import { compileNodeShader, validateFragmentShader } from '../../src/engine/shaderCompiler';
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

describe('ExecutionEngine', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('creates WebGLRenderer in constructor', () => {
    const canvas = document.createElement('canvas');
    new ExecutionEngine(canvas);
    expect(WebGLRenderer).toHaveBeenCalledWith(canvas);
  });

  it('handles constructor failure gracefully', () => {
    vi.mocked(WebGLRenderer).mockImplementationOnce(() => {
      throw new Error('WebGL not supported');
    });
    const canvas = document.createElement('canvas');
    const engine = new ExecutionEngine(canvas);
    // Should not throw, just log error
    expect(engine.isRunning()).toBe(false);
  });

  it('isRunning() initially returns false', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    expect(engine.isRunning()).toBe(false);
  });

  it('run() with no renderer (failed constructor) returns immediately', async () => {
    vi.mocked(WebGLRenderer).mockImplementationOnce(() => {
      throw new Error('fail');
    });
    const engine = new ExecutionEngine(document.createElement('canvas'));
    await engine.run([], []);
    // Should not throw
    expect(engine.isRunning()).toBe(false);
  });

  it('run() with empty nodes completes without error', async () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    await engine.run([], []);
    // topologicalSort should be called
    expect(topologicalSort).toHaveBeenCalled();
  });

  it('run() processes input nodes with sampler2D imageDataUrl', async () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const inputNode = makeNode('input_1', {
      type: 'input',
      inputDataType: 'sampler2D' as never,
      imageDataUrl: 'data:image/png;base64,AAAA',
    });

    const onOutput = vi.fn();
    await engine.run([inputNode], [], onOutput);

    expect(mockRendererInstance.loadImageTexture).toHaveBeenCalledWith('input_1', 'data:image/png;base64,AAAA');
    expect(mockRendererInstance.applyTextureSampling).toHaveBeenCalled();
    // Image node passes through its dataUrl directly — no FBO blit
    expect(mockRendererInstance.createTarget).not.toHaveBeenCalled();
    expect(mockRendererInstance.renderSampler2DInput).not.toHaveBeenCalled();
    expect(onOutput).toHaveBeenCalledWith('input_1', 'data:image/png;base64,AAAA');
  });

  it('run() processes input nodes in framebuffer mode', async () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    // Create a proper base64 data url (4 bytes of data for 1x1 RGBA)
    const rawBytes = new Uint8Array([255, 0, 0, 255]);
    const b64 = Buffer.from(rawBytes).toString('base64');
    const inputNode = makeNode('fb_1', {
      type: 'input',
      inputMode: 'framebuffer',
      rawDataUrl: `data:application/octet-stream;base64,${b64}`,
      fbFormat: 'rgba8',
      fbWidth: 1,
      fbHeight: 1,
    });

    const onOutput = vi.fn();
    await engine.run([inputNode], [], onOutput);

    expect(mockRendererInstance.loadRawTexture).toHaveBeenCalled();
    expect(mockRendererInstance.renderSampler2DInput).toHaveBeenCalled();
    expect(onOutput).toHaveBeenCalledWith('fb_1', 'data:image/png;base64,mock');
  });

  it('run() processes shader nodes with upstream connections', async () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const inputNode = makeNode('input_1', {
      type: 'input',
      inputDataType: 'sampler2D' as never,
      imageDataUrl: 'data:image/png;base64,AAAA',
    });
    const shaderNode = makeNode('shader_1', {
      type: 'shader',
      inputs: [{ id: 'port_1', label: 'inputImage', dataType: 'sampler2D', direction: 'input' }],
      outputs: [],
      shaderCode: 'void main() { fragColor = texture(inputImage, v_uv); }',
    });

    const edges: Edge[] = [
      { id: 'e1', source: 'input_1', target: 'shader_1', sourceHandle: 'out', targetHandle: 'port_1' },
    ];

    // topologicalSort should order input before shader
    vi.mocked(topologicalSort).mockReturnValueOnce(['input_1', 'shader_1']);

    await engine.run([inputNode, shaderNode], edges);

    expect(compileNodeShader).toHaveBeenCalled();
    expect(validateFragmentShader).toHaveBeenCalled();
    expect(mockRendererInstance.renderWithMaterial).toHaveBeenCalled();
  });

  it('run() processes leaf shader nodes with output config', async () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const shaderNode = makeNode('shader_1', {
      type: 'shader',
      inputs: [{ id: 'port_1', label: 'inputImage', dataType: 'sampler2D', direction: 'input' }],
      outputs: [],
      shaderCode: 'void main() { fragColor = texture(inputImage, v_uv); }',
      width: 1024,
      height: 768,
    });

    const onOutput = vi.fn();
    const onOutputSize = vi.fn();
    vi.mocked(topologicalSort).mockReturnValueOnce(['shader_1']);

    await engine.run([shaderNode], [], onOutput, undefined, onOutputSize);

    expect(compileNodeShader).toHaveBeenCalled();
    expect(mockRendererInstance.renderWithMaterial).toHaveBeenCalled();
    expect(onOutput).toHaveBeenCalledWith('shader_1', 'data:image/png;base64,mock');
    expect(onOutputSize).toHaveBeenCalled();
  });

  it('run() calls onNodeError on shader compilation failure', async () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    vi.mocked(compileNodeShader).mockImplementationOnce(() => {
      throw new Error('ERROR: 0:5: syntax error');
    });

    const shaderNode = makeNode('shader_1', {
      type: 'shader',
      inputs: [],
      outputs: [],
    });

    const onNodeError = vi.fn();
    vi.mocked(topologicalSort).mockReturnValueOnce(['shader_1']);

    await engine.run([shaderNode], [], undefined, onNodeError);

    expect(onNodeError).toHaveBeenCalledWith('shader_1', expect.any(String));
  });

  it('run() calls onNodeError when validateFragmentShader returns error', async () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    vi.mocked(validateFragmentShader).mockReturnValueOnce('Compilation error');

    const shaderNode = makeNode('shader_1', {
      type: 'shader',
      inputs: [],
      outputs: [],
    });

    const onNodeError = vi.fn();
    vi.mocked(topologicalSort).mockReturnValueOnce(['shader_1']);

    await engine.run([shaderNode], [], undefined, onNodeError);

    expect(onNodeError).toHaveBeenCalledWith('shader_1', expect.stringContaining('Compilation error'));
  });

  it('stop() sets running=false and clears renderer resources', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    engine.stop();
    expect(engine.isRunning()).toBe(false);
    expect(mockRendererInstance.clearResources).toHaveBeenCalled();
  });

  it('stop() handles renderer already null', () => {
    vi.mocked(WebGLRenderer).mockImplementationOnce(() => {
      throw new Error('fail');
    });
    const engine = new ExecutionEngine(document.createElement('canvas'));
    // Should not throw even with null renderer
    expect(() => engine.stop()).not.toThrow();
  });
});

describe('formatShaderError (tested via ExecutionEngine)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('filters ERROR/WARNING lines and adjusts line numbers', async () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    vi.mocked(compileNodeShader).mockImplementationOnce(() => {
      throw new Error("some preamble\nERROR: 0:10: undeclared 'x'\nother stuff");
    });

    // Mock preambleLines = 0 since compileNodeShader throws before returning
    const shaderNode = makeNode('s1', { type: 'shader', inputs: [], outputs: [] });
    const onNodeError = vi.fn();
    vi.mocked(topologicalSort).mockReturnValueOnce(['s1']);

    await engine.run([shaderNode], [], undefined, onNodeError);

    expect(onNodeError).toHaveBeenCalled();
    const errorMsg = onNodeError.mock.calls[0][1];
    // preambleLines is 0 when compileNodeShader throws, so formatShaderError sees preambleLines=0
    // with preambleLines <= 0, no adjustment happens
    expect(errorMsg).toContain('ERROR:');
  });

  it('returns original message when no ERROR/WARNING/Shader Error lines', async () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    vi.mocked(compileNodeShader).mockImplementationOnce(() => {
      throw new Error('some unknown error format');
    });

    const shaderNode = makeNode('s1', { type: 'shader', inputs: [], outputs: [] });
    const onNodeError = vi.fn();
    vi.mocked(topologicalSort).mockReturnValueOnce(['s1']);

    await engine.run([shaderNode], [], undefined, onNodeError);

    expect(onNodeError).toHaveBeenCalled();
    const errorMsg = onNodeError.mock.calls[0][1];
    expect(errorMsg).toBe('some unknown error format');
  });

  it('adjusts line numbers by preambleLines when preambleLines > 0', async () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));

    // compileNodeShader returns normally but validateFragmentShader fails
    vi.mocked(compileNodeShader).mockReturnValueOnce({
      material: { fragmentShader: 'code', uniforms: {} } as never,
      upstreamSamplers: new Map(),
      preambleLines: 5,
    });
    vi.mocked(validateFragmentShader).mockReturnValueOnce('ERROR: 0:10: undeclared');

    const shaderNode = makeNode('s1', { type: 'shader', inputs: [], outputs: [] });
    const onNodeError = vi.fn();
    vi.mocked(topologicalSort).mockReturnValueOnce(['s1']);

    await engine.run([shaderNode], [], undefined, onNodeError);

    expect(onNodeError).toHaveBeenCalled();
    const errorMsg = onNodeError.mock.calls[0][1];
    // ERROR: 0:10: -> line 10 - 5(preamble) = 5 -> ERROR: 0:5:
    expect(errorMsg).toContain('0:5:');
  });

  it('uses Shader Error / getProgramInfoLog fallback', async () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    vi.mocked(compileNodeShader).mockImplementationOnce(() => {
      throw new Error('preamble line\nShader Error: something broke\nother line');
    });

    const shaderNode = makeNode('s1', { type: 'shader', inputs: [], outputs: [] });
    const onNodeError = vi.fn();
    vi.mocked(topologicalSort).mockReturnValueOnce(['s1']);

    await engine.run([shaderNode], [], undefined, onNodeError);

    const errorMsg = onNodeError.mock.calls[0][1];
    expect(errorMsg).toContain('Shader Error');
  });
});

// ---------------------------------------------------------------------------
// Helper to build a minimal FrameInputs
// ---------------------------------------------------------------------------
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
// prepare()
// ---------------------------------------------------------------------------
describe('ExecutionEngine.prepare()', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('returns null when renderer failed to initialise', () => {
    vi.mocked(WebGLRenderer).mockImplementationOnce(() => {
      throw new Error('no webgl');
    });
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const plan = engine.prepare([], []);
    expect(plan).toBeNull();
  });

  it('returns an ExecutionPlan with correct sortedIds, materials, and targets for a simple shader graph', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const shaderA = makeNode('a', {
      type: 'shader',
      inputs: [{ id: 'p1', label: 'tex', dataType: 'sampler2D', direction: 'input' }],
      shaderCode: 'void main(){}',
    });
    const shaderB = makeNode('b', {
      type: 'shader',
      inputs: [{ id: 'p2', label: 'src', dataType: 'sampler2D', direction: 'input' }],
      shaderCode: 'void main(){}',
    });
    const edges: Edge[] = [
      { id: 'e1', source: 'a', target: 'b', sourceHandle: 'out', targetHandle: 'p2' },
    ];
    vi.mocked(topologicalSort).mockReturnValueOnce(['a', 'b']);

    const plan = engine.prepare([shaderA, shaderB], edges);

    expect(plan).not.toBeNull();
    expect(plan!.sortedIds).toEqual(['a', 'b']);
    expect(plan!.materials.has('a')).toBe(true);
    expect(plan!.materials.has('b')).toBe(true);
    expect(plan!.targets.has('a')).toBe(true);
    expect(plan!.targets.has('b')).toBe(true);
  });

  it('handles renderer nodes: no material created, upstream recorded', () => {
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

    expect(plan).not.toBeNull();
    // Renderer should NOT have a material
    expect(plan!.materials.has('r1')).toBe(false);
    // But upstream bindings should be recorded
    expect(plan!.upstreamSamplerBindings.get('r1')?.get('input')).toBe('a');
    // Renderer nodes should be outputNodes
    expect(plan!.outputNodes).toContain('r1');
  });

  it('handles onnx nodes: upstream recorded, no material', () => {
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
    expect(plan!.materials.has('ox1')).toBe(false);
    expect(plan!.upstreamSamplerBindings.get('ox1')?.get('image')).toBe('a');
  });

  it('detects builtin uniforms on unconnected ports', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const node = makeNode('s1', {
      type: 'shader',
      inputs: [
        { id: 'p1', label: 'iTime', dataType: 'float', direction: 'input' },
        { id: 'p2', label: 'iMouse', dataType: 'vec4', direction: 'input' },
        { id: 'p3', label: 'customVal', dataType: 'float', direction: 'input' },
      ],
      shaderCode: 'void main(){}',
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['s1']);

    const plan = engine.prepare([node], []);

    expect(plan).not.toBeNull();
    const builtins = plan!.builtinPorts.get('s1');
    expect(builtins).toBeDefined();
    expect(builtins!.has('iTime')).toBe(true);
    expect(builtins!.has('iMouse')).toBe(true);
    // Non-builtin should NOT appear
    expect(builtins!.has('customVal')).toBe(false);
  });

  it('calls onNodeError for unconnected sampler2D ports on shader nodes', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const node = makeNode('s1', {
      type: 'shader',
      inputs: [
        { id: 'p1', label: 'myTex', dataType: 'sampler2D', direction: 'input' },
      ],
      shaderCode: 'void main(){}',
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['s1']);
    const onNodeError = vi.fn();

    engine.prepare([node], [], onNodeError);

    expect(onNodeError).toHaveBeenCalledWith('s1', expect.stringContaining("'myTex'"));
  });

  it('does NOT report unconnected sampler2D on renderer nodes', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const rendererNode = makeNode('r1', {
      type: 'renderer',
      inputs: [{ id: 'p1', label: 'input', dataType: 'sampler2D', direction: 'input' }],
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['r1']);
    const onNodeError = vi.fn();

    engine.prepare([rendererNode], [], onNodeError);

    expect(onNodeError).not.toHaveBeenCalled();
  });

  it('reports onOutputSize for renderer whose upstream has a target', () => {
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
    const onOutputSize = vi.fn();

    engine.prepare([shaderA, rendererNode], edges, undefined, onOutputSize);

    // a gets onOutputSize from target creation, r1 from upstream target dimensions
    expect(onOutputSize).toHaveBeenCalledWith('r1', 512, 512);
  });

  it('uses leaf nodes as outputNodes when there are no renderer nodes', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const shaderA = makeNode('a', { type: 'shader', shaderCode: 'void main(){}' });
    const shaderB = makeNode('b', {
      type: 'shader',
      inputs: [{ id: 'p1', label: 'src', dataType: 'sampler2D', direction: 'input' }],
      shaderCode: 'void main(){}',
    });
    const edges: Edge[] = [
      { id: 'e1', source: 'a', target: 'b', sourceHandle: 'out', targetHandle: 'p1' },
    ];
    vi.mocked(topologicalSort).mockReturnValueOnce(['a', 'b']);

    const plan = engine.prepare([shaderA, shaderB], edges);

    // Only 'b' is a leaf (not the source of any edge)
    expect(plan!.outputNodes).toEqual(['b']);
  });

  it('picks default resolution from framebuffer input node', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const fbInput = makeNode('fb1', {
      type: 'input',
      inputMode: 'framebuffer',
      fbWidth: 1920,
      fbHeight: 1080,
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['fb1']);

    const plan = engine.prepare([fbInput], []);

    expect(plan!.defaultW).toBe(1920);
    expect(plan!.defaultH).toBe(1080);
  });

  it('handles shader compile error gracefully — plan still returned', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    vi.mocked(compileNodeShader).mockImplementationOnce(() => {
      throw new Error('ERROR: 0:5: syntax error');
    });
    const node = makeNode('s1', { type: 'shader', shaderCode: 'bad' });
    vi.mocked(topologicalSort).mockReturnValueOnce(['s1']);
    const onNodeError = vi.fn();

    const plan = engine.prepare([node], [], onNodeError);

    // Plan should still be returned (other nodes may be fine)
    expect(plan).not.toBeNull();
    // Material should NOT be set for the broken node
    expect(plan!.materials.has('s1')).toBe(false);
    expect(onNodeError).toHaveBeenCalledWith('s1', expect.stringContaining('ERROR:'));
  });

  it('creates feedback ping-pong targets when compileNodeShader returns needsFeedback=true', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const node = makeNode('fb1', {
      type: 'shader',
      shaderCode: 'void main() { vec4 c = texture(previousFrame, v_uv); }',
      inputs: [],
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['fb1']);
    vi.mocked(compileNodeShader).mockReturnValueOnce({
      material: { fragmentShader: 'fb', uniforms: {} } as never,
      upstreamSamplers: new Map(),
      preambleLines: 5,
      needsFeedback: true,
    });

    const plan = engine.prepare([node], [])!;

    expect(plan.feedbackTargets.has('fb1')).toBe(true);
    const targets = plan.feedbackTargets.get('fb1')!;
    expect(targets).toHaveLength(2);
    expect(targets[0]).toBeDefined();
    expect(targets[1]).toBeDefined();
    expect(targets[0]).not.toBe(targets[1]); // distinct targets
    expect(plan.feedbackReadIndex.get('fb1')).toBe(0);
    expect(plan.feedbackFirstFrame.has('fb1')).toBe(true);
    // Feedback node's primary target should be the first ping-pong target
    expect(plan.targets.get('fb1')).toBe(targets[0]);
    // Targets should be rgba32f (float textures)
    expect(mockRendererInstance.createTarget).toHaveBeenCalledWith('fb1_fb0', expect.any(Number), expect.any(Number), true, 'rgba32f');
    expect(mockRendererInstance.createTarget).toHaveBeenCalledWith('fb1_fb1', expect.any(Number), expect.any(Number), true, 'rgba32f');
  });

  it('non-feedback nodes do not populate feedback plan fields', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const node = makeNode('s1', { type: 'shader', shaderCode: 'void main(){}' });
    vi.mocked(topologicalSort).mockReturnValueOnce(['s1']);

    const plan = engine.prepare([node], [])!;

    expect(plan.feedbackTargets.has('s1')).toBe(false);
    expect(plan.feedbackFirstFrame.size).toBe(0);
  });
});

// ---------------------------------------------------------------------------
// runFrame()
// ---------------------------------------------------------------------------
describe('ExecutionEngine.runFrame()', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  function prepareSingleShaderPlan(engine: ExecutionEngine): ExecutionPlan {
    const node = makeNode('s1', {
      type: 'shader',
      inputs: [],
      shaderCode: 'void main(){}',
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['s1']);
    const plan = engine.prepare([node], [])!;
    vi.clearAllMocks(); // clear prepare's calls so runFrame assertions are clean
    return plan;
  }

  it('renders shader nodes through renderWithMaterial', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const plan = prepareSingleShaderPlan(engine);

    engine.runFrame(plan, makeFrameInputs());

    expect(mockRendererInstance.renderWithMaterial).toHaveBeenCalledTimes(1);
  });

  it('injects builtin iTime uniform from FrameInputs', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const node = makeNode('s1', {
      type: 'shader',
      inputs: [
        { id: 'p1', label: 'iTime', dataType: 'float', direction: 'input' },
        { id: 'p2', label: 'iFrame', dataType: 'int', direction: 'input' },
      ],
      shaderCode: 'void main(){}',
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['s1']);
    const plan = engine.prepare([node], [])!;
    // The plan's material should receive builtin uniforms
    const material = plan.materials.get('s1')!;

    engine.runFrame(plan, makeFrameInputs({ time: 2.5, frame: 120 }));

    expect(material.uniforms['iTime']).toEqual({ value: 2.5 });
    expect(material.uniforms['iFrame']).toEqual({ value: 120 });
  });

  it('skips renderer nodes during render pass', () => {
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
    vi.clearAllMocks();

    engine.runFrame(plan, makeFrameInputs());

    // renderWithMaterial called once for 'a', not for 'r1'
    expect(mockRendererInstance.renderWithMaterial).toHaveBeenCalledTimes(1);
  });

  it('updates textureSources from videoTextures in builtins', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const plan = prepareSingleShaderPlan(engine);
    const videoTex = { type: 1, isVideoTexture: true } as never;
    const videoTextures = new Map([['vid1', videoTex]]);

    engine.runFrame(plan, makeFrameInputs({ videoTextures }));

    expect(plan.textureSources.get('vid1')).toEqual({ kind: 'image', texture: videoTex });
  });

  it('handles missing material gracefully — skips the node', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const node = makeNode('s1', { type: 'shader', shaderCode: 'void main(){}' });
    vi.mocked(topologicalSort).mockReturnValueOnce(['s1']);
    // Force compile failure so material is not set
    vi.mocked(compileNodeShader).mockImplementationOnce(() => {
      throw new Error('compile fail');
    });
    const plan = engine.prepare([node], [])!;
    vi.clearAllMocks();

    // Should not throw
    engine.runFrame(plan, makeFrameInputs());

    expect(mockRendererInstance.renderWithMaterial).not.toHaveBeenCalled();
  });

  it('applies self-uniforms that are not overridden by upstream', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const node = makeNode('s1', {
      type: 'shader',
      inputs: [],
      uniforms: { brightness: 0.8, contrast: '1.2' },
      shaderCode: 'void main(){}',
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['s1']);
    const plan = engine.prepare([node], [])!;
    const material = plan.materials.get('s1')!;

    engine.runFrame(plan, makeFrameInputs());

    // normalizeUniformValue converts '1.2' to 1.2
    expect(material.uniforms['brightness']).toEqual({ value: 0.8 });
    expect(material.uniforms['contrast']).toEqual({ value: 1.2 });
  });

  it('does nothing if renderer is null', () => {
    vi.mocked(WebGLRenderer).mockImplementationOnce(() => {
      throw new Error('no webgl');
    });
    const engine = new ExecutionEngine(document.createElement('canvas'));
    // Build a minimal plan manually
    const plan: ExecutionPlan = {
      sortedIds: ['s1'],
      nodeMap: new Map([['s1', makeNode('s1', { type: 'shader' })]]),
      edges: [],
      materials: new Map(),
      upstreamSamplerBindings: new Map(),
      scalarUpstream: new Map(),
      scalarBindings: new Map(),
      selfUniforms: new Map(),
      targets: new Map(),
      textureSources: new Map(),
      outputNodes: ['s1'],
      builtinPorts: new Map(),
      resolutionUniforms: new Map(),
      preambleLines: new Map(),
      defaultW: 512,
      defaultH: 512,
      mathValues: new Map(),
      feedbackTargets: new Map(),
      feedbackReadIndex: new Map(),
      feedbackFirstFrame: new Set(),
    };

    // Should not throw
    engine.runFrame(plan, makeFrameInputs());
    expect(mockRendererInstance.renderWithMaterial).not.toHaveBeenCalled();
  });

  it('binds upstream FBO texture to sampler uniform', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const inputNode = makeNode('inp', { type: 'input', inputDataType: 'sampler2D' as never, imageDataUrl: 'data:image/png;base64,AA' });
    const shaderNode = makeNode('s1', {
      type: 'shader',
      inputs: [{ id: 'p1', label: 'tex', dataType: 'sampler2D', direction: 'input' }],
      shaderCode: 'void main(){}',
    });
    const edges: Edge[] = [
      { id: 'e1', source: 'inp', target: 's1', sourceHandle: 'out', targetHandle: 'p1' },
    ];

    // compileNodeShader returns upstreamSamplers that reference the upstream
    vi.mocked(compileNodeShader).mockReturnValue({
      material: { fragmentShader: 'code', uniforms: {} } as never,
      upstreamSamplers: new Map([['tex', 'inp']]),
      preambleLines: 0,
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['inp', 's1']);

    const plan = engine.prepare([inputNode, shaderNode], edges)!;
    // Simulate that input node loaded a texture
    const fakeTex = { type: 1, isFakeTexture: true };
    plan.textureSources.set('inp', { kind: 'image', texture: fakeTex as never });
    const material = plan.materials.get('s1')!;

    engine.runFrame(plan, makeFrameInputs());

    // The material should have the upstream texture bound
    expect(material.uniforms['tex']).toEqual({ value: fakeTex });
  });

  // --- Feedback runFrame tests ---

  it('feedback: binds previousFrame uniform and swaps read index after render', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const node = makeNode('fb1', {
      type: 'shader',
      shaderCode: 'void main() { vec4 c = texture(previousFrame, v_uv); }',
      inputs: [],
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['fb1']);
    vi.mocked(compileNodeShader).mockReturnValueOnce({
      material: { fragmentShader: 'fb', uniforms: {} } as never,
      upstreamSamplers: new Map(),
      preambleLines: 5,
      needsFeedback: true,
    });
    const plan = engine.prepare([node], [])!;
    vi.clearAllMocks();

    const material = plan.materials.get('fb1')!;
    // prepare created feedbackTargets with 2 targets; read index starts at 0
    const fbTargets = plan.feedbackTargets.get('fb1')!;

    engine.runFrame(plan, makeFrameInputs());

    // previousFrame should be bound to fbTargets[0] (read target)
    expect(material.uniforms['previousFrame']).toBeDefined();
    expect(material.uniforms['previousFrame'].value).toBe(fbTargets[0].texture);
    // Render was called with fbTargets[1] (write target)
    expect(mockRendererInstance.renderWithMaterial).toHaveBeenCalledWith(material, fbTargets[1]);
    // Read index should be swapped to 1
    expect(plan.feedbackReadIndex.get('fb1')).toBe(1);
    // textureSources set to the write target
    expect(plan.textureSources.get('fb1')).toEqual({ kind: 'fbo', target: fbTargets[1] });
  });

  it('feedback: second frame reads from swapped target and writes to the other', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const node = makeNode('fb2', {
      type: 'shader',
      shaderCode: 'void main() { vec4 c = texture(previousFrame, v_uv); }',
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['fb2']);
    vi.mocked(compileNodeShader).mockReturnValueOnce({
      material: { fragmentShader: 'fb', uniforms: {} } as never,
      upstreamSamplers: new Map(),
      preambleLines: 5,
      needsFeedback: true,
    });
    const plan = engine.prepare([node], [])!;
    vi.clearAllMocks();

    const fbTargets = plan.feedbackTargets.get('fb2')!;
    // First frame: clear + swap (this test starts from state after first frame)
    plan.feedbackFirstFrame.delete('fb2');
    plan.feedbackReadIndex.set('fb2', 1); // after first frame, read index = 1

    const material = plan.materials.get('fb2')!;
    engine.runFrame(plan, makeFrameInputs());

    // previousFrame should now read from fbTargets[1]
    expect(material.uniforms['previousFrame'].value).toBe(fbTargets[1].texture);
    // Should render to fbTargets[0] (write = 1 - read)
    expect(mockRendererInstance.renderWithMaterial).toHaveBeenCalledWith(material, fbTargets[0]);
    // Read index flips back to 0
    expect(plan.feedbackReadIndex.get('fb2')).toBe(0);
  });

  it('feedback: first frame clears both ping-pong targets', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const node = makeNode('fb3', {
      type: 'shader',
      shaderCode: 'void main() { vec4 c = texture(previousFrame, v_uv); }',
      feedbackClearColor: [0, 0, 0, 1],
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['fb3']);
    vi.mocked(compileNodeShader).mockReturnValueOnce({
      material: { fragmentShader: 'fb', uniforms: {} } as never,
      upstreamSamplers: new Map(),
      preambleLines: 5,
      needsFeedback: true,
    });
    const plan = engine.prepare([node], [])!;
    vi.clearAllMocks();

    const fbTargets = plan.feedbackTargets.get('fb3')!;

    engine.runFrame(plan, makeFrameInputs());

    // clearTarget called for both targets
    expect(mockRendererInstance.clearTarget).toHaveBeenCalledWith(fbTargets[0], [0, 0, 0, 1]);
    expect(mockRendererInstance.clearTarget).toHaveBeenCalledWith(fbTargets[1], [0, 0, 0, 1]);
    // feedbackFirstFrame should be cleared after first run
    expect(plan.feedbackFirstFrame.has('fb3')).toBe(false);
  });

  it('feedback: first frame clears with default clear color (no color specified)', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const node = makeNode('fb4', {
      type: 'shader',
      shaderCode: 'void main() { vec4 c = texture(previousFrame, v_uv); }',
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['fb4']);
    vi.mocked(compileNodeShader).mockReturnValueOnce({
      material: { fragmentShader: 'fb', uniforms: {} } as never,
      upstreamSamplers: new Map(),
      preambleLines: 5,
      needsFeedback: true,
    });
    const plan = engine.prepare([node], [])!;
    vi.clearAllMocks();

    const fbTargets = plan.feedbackTargets.get('fb4')!;

    engine.runFrame(plan, makeFrameInputs());

    // When no clear color specified, color is undefined
    expect(mockRendererInstance.clearTarget).toHaveBeenCalledWith(fbTargets[0], undefined);
    expect(mockRendererInstance.clearTarget).toHaveBeenCalledWith(fbTargets[1], undefined);
  });

  it('feedback: does not clear on subsequent frames', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const node = makeNode('fb5', {
      type: 'shader',
      shaderCode: 'void main() { vec4 c = texture(previousFrame, v_uv); }',
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['fb5']);
    vi.mocked(compileNodeShader).mockReturnValueOnce({
      material: { fragmentShader: 'fb', uniforms: {} } as never,
      upstreamSamplers: new Map(),
      preambleLines: 5,
      needsFeedback: true,
    });
    const plan = engine.prepare([node], [])!;
    vi.clearAllMocks();

    // Simulate first frame happened already
    plan.feedbackFirstFrame.delete('fb5');

    engine.runFrame(plan, makeFrameInputs());

    // clearTarget should NOT be called on subsequent frames
    expect(mockRendererInstance.clearTarget).not.toHaveBeenCalled();
  });
});

// ---------------------------------------------------------------------------
// readOutputs()
// ---------------------------------------------------------------------------
describe('ExecutionEngine.readOutputs()', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('calls onOutput for leaf shader nodes with their target', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const node = makeNode('s1', { type: 'shader', shaderCode: 'void main(){}' });
    vi.mocked(topologicalSort).mockReturnValueOnce(['s1']);
    const plan = engine.prepare([node], [])!;
    // Ensure the target and textureSources are populated (prepare does this)
    const onOutput = vi.fn();

    engine.readOutputs(plan, onOutput);

    expect(onOutput).toHaveBeenCalledWith('s1', 'data:image/png;base64,mock');
    expect(mockRendererInstance.readTargetToDataURL).toHaveBeenCalled();
  });

  it('renderer output reads upstream FBO via readTargetToDataURL', () => {
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
    // Simulate that shader 'a' was rendered to its FBO
    const fboTarget = { texture: { type: 1 }, width: 512, height: 512, dispose: vi.fn() };
    plan.textureSources.set('a', { kind: 'fbo', target: fboTarget as never });
    const onOutput = vi.fn();

    engine.readOutputs(plan, onOutput);

    expect(onOutput).toHaveBeenCalledWith('r1', 'data:image/png;base64,mock');
  });

  it('renderer with image texture source renders through scratch target', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const shaderA = makeNode('a', {
      type: 'shader',
      shaderCode: 'void main(){}',
      imageWidth: 640,
      imageHeight: 480,
    });
    const rendererNode = makeNode('r1', {
      type: 'renderer',
      inputs: [{ id: 'p1', label: 'input', dataType: 'sampler2D', direction: 'input' }],
    });
    const edges: Edge[] = [
      { id: 'e1', source: 'a', target: 'r1', sourceHandle: 'out', targetHandle: 'p1' },
    ];
    vi.mocked(topologicalSort).mockReturnValueOnce(['a', 'r1']);
    const plan = engine.prepare([shaderA, rendererNode], edges)!;
    const imageTex = { type: 1 };
    plan.textureSources.set('a', { kind: 'image', texture: imageTex as never });
    const onOutput = vi.fn();

    engine.readOutputs(plan, onOutput);

    // Should blit via renderSampler2DInput for image sources
    expect(mockRendererInstance.renderSampler2DInput).toHaveBeenCalled();
    expect(onOutput).toHaveBeenCalledWith('r1', 'data:image/png;base64,mock');
  });

  it('does nothing if renderer is null', () => {
    vi.mocked(WebGLRenderer).mockImplementationOnce(() => {
      throw new Error('no webgl');
    });
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const plan: ExecutionPlan = {
      sortedIds: [],
      nodeMap: new Map(),
      edges: [],
      materials: new Map(),
      upstreamSamplerBindings: new Map(),
      scalarUpstream: new Map(),
      scalarBindings: new Map(),
      selfUniforms: new Map(),
      targets: new Map(),
      textureSources: new Map(),
      outputNodes: ['s1'],
      builtinPorts: new Map(),
      resolutionUniforms: new Map(),
      preambleLines: new Map(),
      defaultW: 512,
      defaultH: 512,
      mathValues: new Map(),
      feedbackTargets: new Map(),
      feedbackReadIndex: new Map(),
      feedbackFirstFrame: new Set(),
    };
    const onOutput = vi.fn();

    engine.readOutputs(plan, onOutput);

    expect(onOutput).not.toHaveBeenCalled();
  });

  it('reads output for feedback nodes from textureSources (latest write target)', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const node = makeNode('fb_fb', {
      type: 'shader',
      shaderCode: 'void main() { vec4 c = texture(previousFrame, v_uv); }',
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['fb_fb']);
    vi.mocked(compileNodeShader).mockReturnValueOnce({
      material: { fragmentShader: 'fb', uniforms: {} } as never,
      upstreamSamplers: new Map(),
      preambleLines: 5,
      needsFeedback: true,
    });
    const plan = engine.prepare([node], [])!;
    // Simulate that runFrame was called, writing to the feedback write target
    const writeTarget = { texture: { type: 1 }, width: 512, height: 512, dispose: vi.fn() };
    plan.textureSources.set('fb_fb', { kind: 'fbo', target: writeTarget as never });
    const onOutput = vi.fn();

    engine.readOutputs(plan, onOutput);

    expect(onOutput).toHaveBeenCalledWith('fb_fb', 'data:image/png;base64,mock');
    expect(mockRendererInstance.readTargetToDataURL).toHaveBeenCalledWith(writeTarget, 512);
  });
});

// ---------------------------------------------------------------------------
// renderRendererToScreen()
// ---------------------------------------------------------------------------
describe('ExecutionEngine.renderRendererToScreen()', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('calls renderToScreen with upstream FBO texture', () => {
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

  it('calls renderToScreen with image texture when source is image kind', () => {
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
    const imgTex = { type: 1, isImage: true };
    plan.textureSources.set('a', { kind: 'image', texture: imgTex as never });

    engine.renderRendererToScreen(plan, 'r1');

    expect(mockRendererInstance.renderToScreen).toHaveBeenCalledWith(imgTex);
  });

  it('does nothing for non-renderer node id', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const node = makeNode('s1', { type: 'shader', shaderCode: 'void main(){}' });
    vi.mocked(topologicalSort).mockReturnValueOnce(['s1']);
    const plan = engine.prepare([node], [])!;

    engine.renderRendererToScreen(plan, 's1');

    expect(mockRendererInstance.renderToScreen).not.toHaveBeenCalled();
  });

  it('does nothing when upstream has no texture source', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const rendererNode = makeNode('r1', {
      type: 'renderer',
      inputs: [{ id: 'p1', label: 'input', dataType: 'sampler2D', direction: 'input' }],
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['r1']);
    const plan = engine.prepare([rendererNode], [])!;

    engine.renderRendererToScreen(plan, 'r1');

    expect(mockRendererInstance.renderToScreen).not.toHaveBeenCalled();
  });
});

// ---------------------------------------------------------------------------
// getCanvas()
// ---------------------------------------------------------------------------
describe('ExecutionEngine.getCanvas()', () => {
  it('returns the canvas from the renderer', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const canvas = engine.getCanvas();
    expect(canvas).toBeInstanceOf(HTMLCanvasElement);
  });

  it('returns null when renderer failed to initialise', () => {
    vi.mocked(WebGLRenderer).mockImplementationOnce(() => {
      throw new Error('no webgl');
    });
    const engine = new ExecutionEngine(document.createElement('canvas'));
    expect(engine.getCanvas()).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// captureRendererScreenshot()
// ---------------------------------------------------------------------------
describe('ExecutionEngine.captureRendererScreenshot()', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('returns dataURL from upstream FBO', () => {
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

  it('returns dataURL from image source via scratch target', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const sourceNode = makeNode('img', {
      type: 'shader',
      shaderCode: 'void main(){}',
      imageWidth: 800,
      imageHeight: 600,
    });
    const rendererNode = makeNode('r1', {
      type: 'renderer',
      inputs: [{ id: 'p1', label: 'input', dataType: 'sampler2D', direction: 'input' }],
    });
    const edges: Edge[] = [
      { id: 'e1', source: 'img', target: 'r1', sourceHandle: 'out', targetHandle: 'p1' },
    ];
    vi.mocked(topologicalSort).mockReturnValueOnce(['img', 'r1']);
    const plan = engine.prepare([sourceNode, rendererNode], edges)!;
    const imgTex = { type: 1 };
    plan.textureSources.set('img', { kind: 'image', texture: imgTex as never });

    const result = engine.captureRendererScreenshot(plan, 'r1');

    expect(result).toBe('data:image/png;base64,mock');
    expect(mockRendererInstance.renderSampler2DInput).toHaveBeenCalled();
    expect(mockRendererInstance.readTargetToDataURL).toHaveBeenCalled();
  });

  it('returns null when no upstream source exists', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const rendererNode = makeNode('r1', {
      type: 'renderer',
      inputs: [{ id: 'p1', label: 'input', dataType: 'sampler2D', direction: 'input' }],
    });
    vi.mocked(topologicalSort).mockReturnValueOnce(['r1']);
    const plan = engine.prepare([rendererNode], [])!;

    const result = engine.captureRendererScreenshot(plan, 'r1');

    expect(result).toBeNull();
  });

  it('returns null when renderer is null', () => {
    vi.mocked(WebGLRenderer).mockImplementationOnce(() => {
      throw new Error('no webgl');
    });
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const plan: ExecutionPlan = {
      sortedIds: [],
      nodeMap: new Map(),
      edges: [],
      materials: new Map(),
      upstreamSamplerBindings: new Map([['r1', new Map([['input', 'a']])]]),
      scalarUpstream: new Map(),
      scalarBindings: new Map(),
      selfUniforms: new Map(),
      targets: new Map(),
      textureSources: new Map(),
      outputNodes: ['r1'],
      builtinPorts: new Map(),
      resolutionUniforms: new Map(),
      preambleLines: new Map(),
      defaultW: 512,
      defaultH: 512,
      mathValues: new Map(),
      feedbackTargets: new Map(),
      feedbackReadIndex: new Map(),
      feedbackFirstFrame: new Set(),
    };

    const result = engine.captureRendererScreenshot(plan, 'r1');

    expect(result).toBeNull();
  });
});
