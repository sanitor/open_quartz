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
    dispose: vi.fn(),
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

import { ExecutionEngine } from '../../src/engine/executionEngine';
import { WebGLRenderer } from '../../src/engine/webglRenderer';
import { compileNodeShader, validateFragmentShader } from '../../src/engine/shaderCompiler';
import { topologicalSort } from '../../src/engine/graphExecutor';

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
    expect(onOutput).toHaveBeenCalledWith('input_1', 'data:image/png;base64,mock');
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

  it('run() processes output nodes', async () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    const outputNode = makeNode('output_1', {
      type: 'output',
      inputs: [{ id: 'port_1', label: 'inputImage', dataType: 'sampler2D', direction: 'input' }],
      shaderCode: 'void main() { fragColor = texture(inputImage, v_uv); }',
    });

    const onOutput = vi.fn();
    const onOutputSize = vi.fn();
    vi.mocked(topologicalSort).mockReturnValueOnce(['output_1']);

    await engine.run([outputNode], [], onOutput, undefined, onOutputSize);

    expect(compileNodeShader).toHaveBeenCalled();
    expect(mockRendererInstance.renderWithMaterial).toHaveBeenCalled();
    expect(onOutput).toHaveBeenCalledWith('output_1', 'data:image/png;base64,mock');
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

  it('stop() sets running=false and disposes renderer', () => {
    const engine = new ExecutionEngine(document.createElement('canvas'));
    engine.stop();
    expect(engine.isRunning()).toBe(false);
    expect(mockRendererInstance.dispose).toHaveBeenCalled();
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
