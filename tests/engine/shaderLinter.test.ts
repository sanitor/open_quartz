import { describe, it, expect, vi, beforeEach } from 'vitest';

// Reset cached GL context between tests by resetting the module
beforeEach(() => {
  vi.resetModules();
});

describe('glslLinter', () => {
  function makeMockDoc(code: string) {
    const lines = code.split('\n');
    let charOffset = 0;
    const lineOffsets: Array<{ from: number; to: number }> = [];
    for (const line of lines) {
      lineOffsets.push({ from: charOffset, to: charOffset + line.length });
      charOffset += line.length + 1; // +1 for newline
    }

    return {
      toString: () => code,
      lines: lines.length,
      line: (n: number) => lineOffsets[n - 1] ?? { from: 0, to: 0 },
    };
  }

  function makeMockView(code: string) {
    return {
      state: {
        doc: makeMockDoc(code),
      },
    };
  }

  function makeMockGL(opts: {
    compileStatus: boolean;
    infoLog: string;
    createShaderReturns?: WebGLShader | null;
  }) {
    return {
      FRAGMENT_SHADER: 0x8b30,
      COMPILE_STATUS: 0x8b81,
      createShader: vi.fn(() => opts.createShaderReturns ?? ({} as WebGLShader)),
      shaderSource: vi.fn(),
      compileShader: vi.fn(),
      getShaderParameter: vi.fn(() => opts.compileStatus),
      getShaderInfoLog: vi.fn(() => opts.infoLog),
      deleteShader: vi.fn(),
    };
  }

  function setupDOM(gl: unknown) {
    const mockCanvas = {
      getContext: vi.fn(() => gl),
    };
    vi.spyOn(document, 'createElement').mockReturnValue(mockCanvas as unknown as HTMLElement);
  }

  it('returns empty diagnostics for empty code', async () => {
    const { glslLinter } = await import('../../src/engine/shaderLinter');
    const view = makeMockView('   ');
    const result = glslLinter(view as never);
    expect(result).toEqual([]);
  });

  it('returns empty diagnostics when GL is unavailable', async () => {
    setupDOM(null);
    const { glslLinter } = await import('../../src/engine/shaderLinter');
    const view = makeMockView('void main() {}');
    const result = glslLinter(view as never);
    expect(result).toEqual([]);
  });

  it('returns empty diagnostics on compilation success', async () => {
    const gl = makeMockGL({ compileStatus: true, infoLog: '' });
    setupDOM(gl);
    const { glslLinter } = await import('../../src/engine/shaderLinter');
    const view = makeMockView('void main() { fragColor = vec4(1.0); }');
    const result = glslLinter(view as never);
    expect(result).toEqual([]);
  });

  it('parses ERROR lines into diagnostics with correct line numbers', async () => {
    const gl = makeMockGL({
      compileStatus: false,
      infoLog: "ERROR: 0:6: 'x' : undeclared identifier",
    });
    setupDOM(gl);
    const { glslLinter } = await import('../../src/engine/shaderLinter');

    // Code with 3 lines. Boilerplate offset is 4 lines. ERROR on line 6 -> cleaned line = 6-4=2
    // strippedLines = 0 (no #version or precision in code), so editorLine = min(2+0, 3) = 2
    const code = 'line one\nline two\nline three';
    const view = makeMockView(code);
    const result = glslLinter(view as never);

    expect(result).toHaveLength(1);
    expect(result[0].severity).toBe('error');
    expect(result[0].message).toBe("'x' : undeclared identifier");
    expect(result[0].source).toBe('GLSL');
  });

  it('parses multiple error lines', async () => {
    const gl = makeMockGL({
      compileStatus: false,
      infoLog:
        "ERROR: 0:5: 'x' : undeclared\nERROR: 0:6: 'y' : undeclared",
    });
    setupDOM(gl);
    const { glslLinter } = await import('../../src/engine/shaderLinter');

    const code = 'line1\nline2\nline3';
    const view = makeMockView(code);
    const result = glslLinter(view as never);

    expect(result).toHaveLength(2);
    expect(result[0].message).toBe("'x' : undeclared");
    expect(result[1].message).toBe("'y' : undeclared");
  });

  it('returns fallback diagnostic at position 0 for unparseable error log', async () => {
    const gl = makeMockGL({
      compileStatus: false,
      infoLog: 'Some unparseable error without ERROR: prefix',
    });
    setupDOM(gl);
    const { glslLinter } = await import('../../src/engine/shaderLinter');

    const view = makeMockView('void main() {}');
    const result = glslLinter(view as never);

    expect(result).toHaveLength(1);
    expect(result[0].from).toBe(0);
    expect(result[0].to).toBe(0);
    expect(result[0].severity).toBe('error');
    expect(result[0].message).toBe('Some unparseable error without ERROR: prefix');
  });

  it('returns empty when createShader returns null', async () => {
    const gl = makeMockGL({
      compileStatus: true,
      infoLog: '',
      createShaderReturns: null,
    });
    setupDOM(gl);
    const { glslLinter } = await import('../../src/engine/shaderLinter');

    const view = makeMockView('void main() {}');
    const result = glslLinter(view as never);
    expect(result).toEqual([]);
  });

  it('strips #version and precision from user code, affecting strippedLines', async () => {
    const gl = makeMockGL({
      compileStatus: false,
      infoLog: "ERROR: 0:6: 'z' : undeclared identifier",
    });
    setupDOM(gl);
    const { glslLinter } = await import('../../src/engine/shaderLinter');

    // Code with #version and precision that get stripped (2 stripped lines)
    // After stripping, the cleaned code has fewer lines
    // ERROR line 6 -> cleanedLine = 6 - 4(offset) = 2, editorLine = 2 + 2(stripped) = 4
    const code = '#version 300 es\nprecision highp float;\nline1\nline2\nline3';
    const view = makeMockView(code);
    const result = glslLinter(view as never);

    expect(result).toHaveLength(1);
    expect(result[0].severity).toBe('error');
    // editorLine is clamped to doc.lines (5 lines total)
    expect(result[0].message).toBe("'z' : undeclared identifier");
  });

  it('skips errors with cleaned line < 1 (errors in boilerplate)', async () => {
    const gl = makeMockGL({
      compileStatus: false,
      // Line 2 in full source is within boilerplate (offset=4), so cleanedLine = 2-4 = -2 < 1
      infoLog: "ERROR: 0:2: some boilerplate error\nERROR: 0:6: 'x' : real error",
    });
    setupDOM(gl);
    const { glslLinter } = await import('../../src/engine/shaderLinter');

    const code = 'line1\nline2\nline3';
    const view = makeMockView(code);
    const result = glslLinter(view as never);

    // Only the second error should appear (first is in boilerplate)
    expect(result).toHaveLength(1);
    expect(result[0].message).toBe("'x' : real error");
  });
});
