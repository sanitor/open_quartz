import { describe, it, expect, vi } from 'vitest';

// Mock @codemirror/autocomplete
vi.mock('@codemirror/autocomplete', () => {
  class CompletionContext {
    state: unknown;
    pos: number;
    explicit: boolean;
    private _matchResult: { from: number; to: number; text: string } | null;

    constructor(
      state: unknown,
      pos: number,
      explicit: boolean,
      matchResult: { from: number; to: number; text: string } | null = null,
    ) {
      this.state = state;
      this.pos = pos;
      this.explicit = explicit;
      this._matchResult = matchResult;
    }

    matchBefore(_re: RegExp) {
      return this._matchResult;
    }
  }
  return { CompletionContext };
});

// Mock @codemirror/language
vi.mock('@codemirror/language', () => {
  return {
    syntaxTree: vi.fn(() => ({
      cursor: () => ({
        iterate: vi.fn(),
      }),
    })),
  };
});

// Mock @codemirror/state (EditorState is used as a type)
vi.mock('@codemirror/state', () => ({}));

import { glslCompletions } from '../../src/engine/shaderCompletions';
import { CompletionContext } from '@codemirror/autocomplete';
import { syntaxTree } from '@codemirror/language';

describe('glslCompletions', () => {
  function makeContext(opts: {
    word?: { from: number; to: number; text: string } | null;
    explicit?: boolean;
    pos?: number;
  }) {
    const state = { sliceDoc: vi.fn(() => '') };
    return new CompletionContext(
      state,
      opts.pos ?? 10,
      opts.explicit ?? false,
      opts.word ?? null,
    );
  }

  it('returns completion options including keywords, types, functions, builtins', () => {
    const ctx = makeContext({ word: { from: 5, to: 8, text: 'vec' } });
    const result = glslCompletions(ctx);

    expect(result).not.toBeNull();
    const labels = result!.options.map((o: { label: string }) => o.label);
    // Check keywords
    expect(labels).toContain('if');
    expect(labels).toContain('for');
    expect(labels).toContain('return');
    // Check types
    expect(labels).toContain('float');
    expect(labels).toContain('vec2');
    expect(labels).toContain('mat4');
    expect(labels).toContain('sampler2D');
    // Check functions
    expect(labels).toContain('sin');
    expect(labels).toContain('texture');
    expect(labels).toContain('normalize');
    // Check builtins
    expect(labels).toContain('gl_FragCoord');
    expect(labels).toContain('gl_Position');
  });

  it('sets from = word.from when word matches', () => {
    const ctx = makeContext({ word: { from: 3, to: 7, text: 'floa' } });
    const result = glslCompletions(ctx);
    expect(result).not.toBeNull();
    expect(result!.from).toBe(3);
  });

  it('sets from = context.pos when no word match and explicit=true', () => {
    const ctx = makeContext({ word: null, explicit: true, pos: 42 });
    const result = glslCompletions(ctx);
    expect(result).not.toBeNull();
    expect(result!.from).toBe(42);
  });

  it('returns null when no word match and explicit=false', () => {
    const ctx = makeContext({ word: null, explicit: false });
    const result = glslCompletions(ctx);
    expect(result).toBeNull();
  });

  it('includes correct type labels for each category', () => {
    const ctx = makeContext({ word: { from: 0, to: 1, text: 'a' } });
    const result = glslCompletions(ctx);
    expect(result).not.toBeNull();

    const options = result!.options as Array<{ label: string; type: string }>;

    const ifOpt = options.find((o) => o.label === 'if');
    expect(ifOpt?.type).toBe('keyword');

    const vecOpt = options.find((o) => o.label === 'vec3');
    expect(vecOpt?.type).toBe('type');

    const sinOpt = options.find((o) => o.label === 'sin');
    expect(sinOpt?.type).toBe('function');

    const glOpt = options.find((o) => o.label === 'gl_FragCoord');
    expect(glOpt?.type).toBe('constant');
  });

  it('includes validFor regex in result', () => {
    const ctx = makeContext({ word: { from: 0, to: 1, text: 'a' } });
    const result = glslCompletions(ctx);
    expect(result).not.toBeNull();
    expect(result!.validFor).toEqual(/^\w*$/);
  });
});

describe('extractUserVariables (via glslCompletions)', () => {
  it('adds user-defined variables with type "variable"', () => {
    // Mock syntaxTree to return a cursor with IdentifierDefinition nodes
    vi.mocked(syntaxTree).mockReturnValueOnce({
      cursor: () => ({
        iterate: (callback: (node: { name: string; from: number; to: number }) => void) => {
          callback({ name: 'IdentifierDefinition', from: 0, to: 5 });
          callback({ name: 'IdentifierDefinition', from: 10, to: 18 });
        },
      }),
    } as never);

    const state = {
      sliceDoc: vi.fn((from: number, _to: number) => {
        if (from === 0) return 'myVar';
        if (from === 10) return 'otherVar';
        return '';
      }),
    };

    const ctx = new CompletionContext(state, 20, true, null);
    const result = glslCompletions(ctx);

    expect(result).not.toBeNull();
    const userVars = result!.options.filter(
      (o: { label: string; type: string }) => o.type === 'variable',
    );
    const userLabels = userVars.map((o: { label: string }) => o.label);
    expect(userLabels).toContain('myVar');
    expect(userLabels).toContain('otherVar');
  });

  it('excludes builtin names from user variables', () => {
    vi.mocked(syntaxTree).mockReturnValueOnce({
      cursor: () => ({
        iterate: (callback: (node: { name: string; from: number; to: number }) => void) => {
          // 'float' is a builtin type name, should be excluded
          callback({ name: 'IdentifierDefinition', from: 0, to: 5 });
          callback({ name: 'IdentifierDefinition', from: 10, to: 16 });
        },
      }),
    } as never);

    const state = {
      sliceDoc: vi.fn((from: number, _to: number) => {
        if (from === 0) return 'float'; // builtin type
        if (from === 10) return 'custom'; // user var
        return '';
      }),
    };

    const ctx = new CompletionContext(state, 20, true, null);
    const result = glslCompletions(ctx);

    expect(result).not.toBeNull();
    const userVars = result!.options.filter(
      (o: { label: string; type: string }) => o.type === 'variable',
    );
    const userLabels = userVars.map((o: { label: string }) => o.label);
    expect(userLabels).toContain('custom');
    expect(userLabels).not.toContain('float');
  });

  it('handles zero IdentifierDefinition nodes', () => {
    vi.mocked(syntaxTree).mockReturnValueOnce({
      cursor: () => ({
        iterate: vi.fn(), // no calls to callback
      }),
    } as never);

    const state = { sliceDoc: vi.fn() };
    const ctx = new CompletionContext(state, 5, true, null);
    const result = glslCompletions(ctx);

    expect(result).not.toBeNull();
    const userVars = result!.options.filter(
      (o: { label: string; type: string }) => o.type === 'variable',
    );
    expect(userVars).toHaveLength(0);
  });
});
