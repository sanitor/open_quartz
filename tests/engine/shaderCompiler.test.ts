import { describe, it, expect, vi } from 'vitest';

// Mock THREE.js
vi.mock('three', () => {
  const GLSL3 = 'GLSL3';
  class RawShaderMaterial {
    vertexShader: string;
    fragmentShader: string;
    uniforms: Record<string, unknown>;
    glslVersion: string;
    constructor(opts: {
      vertexShader: string;
      fragmentShader: string;
      uniforms: Record<string, unknown>;
      glslVersion: string;
    }) {
      this.vertexShader = opts.vertexShader;
      this.fragmentShader = opts.fragmentShader;
      this.uniforms = opts.uniforms;
      this.glslVersion = opts.glslVersion;
    }
  }
  return { RawShaderMaterial, GLSL3 };
});

import { compileNodeShader, validateFragmentShader } from '../../src/engine/shaderCompiler';

describe('compileNodeShader', () => {
  it('returns material with fragmentShader containing user code', () => {
    const code = 'void main() { fragColor = vec4(1.0); }';
    const result = compileNodeShader(code, [], new Map());
    expect(result.material.fragmentShader).toContain('void main()');
    expect(result.material.fragmentShader).toContain('fragColor = vec4(1.0)');
    expect(result.material.fragmentShader).toContain('precision highp float;');
    expect(result.material.fragmentShader).toContain('in vec2 v_uv;');
    expect(result.material.fragmentShader).toContain('out vec4 fragColor;');
  });

  it('populates upstreamSamplers for sampler2D upstream connections', () => {
    const code = 'uniform sampler2D myTex;\nvoid main() { fragColor = texture(myTex, v_uv); }';
    const inputs = [{ label: 'myTex', dataType: 'sampler2D' }];
    const upstreamMap = new Map([['myTex', 'node_1']]);

    const result = compileNodeShader(code, inputs, upstreamMap);
    expect(result.upstreamSamplers.get('myTex')).toBe('node_1');
    expect(result.material.fragmentShader).toContain('uniform sampler2D myTex;');
  });

  it('injects scalar uniform for connected upstream and strips user declaration', () => {
    const code = 'uniform float brightness;\nvoid main() { fragColor = vec4(brightness); }';
    const inputs = [{ label: 'brightness', dataType: 'float' }];
    const upstreamMap = new Map([['brightness', 'node_2']]);

    const result = compileNodeShader(code, inputs, upstreamMap);
    expect(result.material.fragmentShader).toContain('uniform float brightness;');
    // The user's duplicate declaration should be stripped - only the injected one remains
    const matches = result.material.fragmentShader.match(/uniform float brightness/g);
    expect(matches).toHaveLength(1);
    expect(result.upstreamSamplers.has('brightness')).toBe(false);
  });

  it('auto-injects unconnected non-sampler inputs', () => {
    const code = 'void main() { fragColor = vec4(intensity); }';
    const inputs = [{ label: 'intensity', dataType: 'float' }];
    const upstreamMap = new Map<string, string>();

    const result = compileNodeShader(code, inputs, upstreamMap);
    expect(result.material.fragmentShader).toContain('uniform float intensity;');
  });

  it('does not auto-inject sampler2D for unconnected inputs', () => {
    const code = 'void main() { fragColor = vec4(1.0); }';
    const inputs = [{ label: 'tex', dataType: 'sampler2D' }];
    const upstreamMap = new Map<string, string>();

    const result = compileNodeShader(code, inputs, upstreamMap);
    expect(result.material.fragmentShader).not.toContain('uniform sampler2D tex;');
  });

  it('calculates preambleLines as 1 + 3 + uniformCount + 1', () => {
    const code = 'void main() { fragColor = vec4(1.0); }';
    // 0 uniforms: preambleLines = 1 + 3 + 0 + 1 = 5
    const result0 = compileNodeShader(code, [], new Map());
    expect(result0.preambleLines).toBe(5);

    // 2 uniforms from upstream
    const inputs = [
      { label: 'a', dataType: 'float' },
      { label: 'b', dataType: 'float' },
    ];
    const upstreamMap = new Map([
      ['a', 'n1'],
      ['b', 'n2'],
    ]);
    const result2 = compileNodeShader(
      'uniform float a;\nuniform float b;\nvoid main() {}',
      inputs,
      upstreamMap,
    );
    expect(result2.preambleLines).toBe(7); // 1 + 3 + 2 + 1

    // 1 upstream + 1 unconnected = 2 uniforms
    const inputs3 = [
      { label: 'x', dataType: 'float' },
      { label: 'y', dataType: 'int' },
    ];
    const upstream3 = new Map([['x', 'n1']]);
    const result3 = compileNodeShader(
      'uniform float x;\nvoid main() {}',
      inputs3,
      upstream3,
    );
    expect(result3.preambleLines).toBe(7);
  });

  it('strips duplicate uniform declarations from user code', () => {
    const code =
      'uniform float a;\nuniform int b;\nvoid main() { fragColor = vec4(a, float(b), 0.0, 1.0); }';
    const inputs = [
      { label: 'a', dataType: 'float' },
      { label: 'b', dataType: 'int' },
    ];
    const upstreamMap = new Map([
      ['a', 'n1'],
      ['b', 'n2'],
    ]);
    const result = compileNodeShader(code, inputs, upstreamMap);

    const aMatches = result.material.fragmentShader.match(/uniform float a;/g);
    expect(aMatches).toHaveLength(1);
    const bMatches = result.material.fragmentShader.match(/uniform int b;/g);
    expect(bMatches).toHaveLength(1);
  });

  it('strips #version, precision, and out vec4 from user code (stripInjected)', () => {
    const code =
      '#version 300 es\nprecision highp float;\nout vec4 fragColor;\nvoid main() { fragColor = vec4(1.0); }';
    const result = compileNodeShader(code, [], new Map());
    // The preamble adds these, so the user version should be stripped
    const precisionMatches = result.material.fragmentShader.match(/precision highp float;/g);
    expect(precisionMatches).toHaveLength(1);
    // #version should not appear in the fragmentShader at all (it's prepended by THREE.js later)
    expect(result.material.fragmentShader).not.toContain('#version');
  });

  it('sets glslVersion to GLSL3 on the material', () => {
    const result = compileNodeShader('void main() {}', [], new Map());
    expect(result.material.glslVersion).toBe('GLSL3');
  });

  it('sets vertexShader on the material', () => {
    const result = compileNodeShader('void main() {}', [], new Map());
    expect(result.material.vertexShader).toContain('v_uv');
    expect(result.material.vertexShader).toContain('gl_Position');
  });
});

describe('validateFragmentShader', () => {
  interface MockGLOptions {
    compileStatus: boolean;
    infoLog: string | null;
    createShaderReturns?: WebGLShader | null;
  }

  function createMockGL(opts: MockGLOptions) {
    return {
      FRAGMENT_SHADER: 0x8b30,
      COMPILE_STATUS: 0x8b81,
      createShader: vi.fn(() => 'createShaderReturns' in opts ? opts.createShaderReturns : ({} as WebGLShader)),
      shaderSource: vi.fn(),
      compileShader: vi.fn(),
      getShaderParameter: vi.fn(() => opts.compileStatus),
      getShaderInfoLog: vi.fn(() => opts.infoLog),
      deleteShader: vi.fn(),
    } as unknown as WebGL2RenderingContext;
  }

  it('returns null on successful compilation', () => {
    const gl = createMockGL({ compileStatus: true, infoLog: '' });
    const result = validateFragmentShader(gl, 'precision highp float;\nvoid main() {}');
    expect(result).toBeNull();
    expect(gl.createShader).toHaveBeenCalledWith(gl.FRAGMENT_SHADER);
    expect(gl.deleteShader).toHaveBeenCalled();
  });

  it('returns error log on compilation failure', () => {
    const gl = createMockGL({ compileStatus: false, infoLog: 'ERROR: 0:1: syntax error' });
    const result = validateFragmentShader(gl, 'invalid code');
    expect(result).toBe('ERROR: 0:1: syntax error');
  });

  it('returns "Unknown compilation error" when failure with null log', () => {
    const gl = createMockGL({ compileStatus: false, infoLog: null });
    const result = validateFragmentShader(gl, 'invalid code');
    expect(result).toBe('Unknown compilation error');
  });

  it('returns warning string when success with non-empty non-Success log', () => {
    const gl = createMockGL({ compileStatus: true, infoLog: 'some warning text' });
    const result = validateFragmentShader(gl, 'valid but warning');
    expect(result).toBe('Warning: some warning text');
  });

  it('returns null when success log contains "Success"', () => {
    const gl = createMockGL({ compileStatus: true, infoLog: 'Success' });
    const result = validateFragmentShader(gl, 'valid code');
    expect(result).toBeNull();
  });

  it('returns error string when createShader returns null', () => {
    const gl = createMockGL({ compileStatus: true, infoLog: '', createShaderReturns: null });
    const result = validateFragmentShader(gl, 'some code');
    expect(result).toBe('Failed to create shader object');
  });

  it('prepends #version 300 es to the source before compiling', () => {
    const gl = createMockGL({ compileStatus: true, infoLog: '' });
    validateFragmentShader(gl, 'precision highp float;\nvoid main() {}');
    const calls = vi.mocked(gl.shaderSource).mock.calls;
    expect(calls[0][1]).toMatch(/^#version 300 es\n/);
  });
});
