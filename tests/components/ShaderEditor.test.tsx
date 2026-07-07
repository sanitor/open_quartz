import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render } from '@testing-library/react';

// Use vi.hoisted so these are available in vi.mock factories
const { mockEditorViewDestroy, mockEditorViewDispatch, MockEditorView, getCapturedParent, getCapturedUpdateListener, resetCaptures } = vi.hoisted(() => {
  const destroy = vi.fn();
  const dispatch = vi.fn();
  let _capturedParent: HTMLElement | undefined;
  let _capturedUpdateListener: ((update: { docChanged: boolean; state: { doc: { toString: () => string } } }) => void) | null = null;

  const Ctor = vi.fn(function(this: Record<string, unknown>, config: { state: unknown; parent: HTMLElement }) {
    _capturedParent = config.parent;
    this.destroy = destroy;
    this.dispatch = dispatch;
    this.state = { doc: { toString: () => 'initial code' } };
    return this;
  }) as unknown as {
    new (config: { state: unknown; parent: HTMLElement }): { destroy: typeof destroy; dispatch: typeof dispatch; state: { doc: { toString: () => string } } };
    lineWrapping: symbol;
    updateListener: { of: (fn: (update: { docChanged: boolean; state: { doc: { toString: () => string } } }) => void) => symbol };
    theme: (spec: Record<string, Record<string, string>>) => symbol;
  };

  (Ctor as Record<string, unknown>).lineWrapping = Symbol('lineWrapping');
  (Ctor as Record<string, unknown>).updateListener = {
    of: (fn: (update: { docChanged: boolean; state: { doc: { toString: () => string } } }) => void) => {
      _capturedUpdateListener = fn;
      return Symbol('updateListener');
    },
  };
  (Ctor as Record<string, unknown>).theme = () => Symbol('theme');

  return {
    mockEditorViewDestroy: destroy,
    mockEditorViewDispatch: dispatch,
    MockEditorView: Ctor,
    getCapturedParent: () => _capturedParent,
    getCapturedUpdateListener: () => _capturedUpdateListener,
    resetCaptures: () => { _capturedParent = undefined; _capturedUpdateListener = null; },
  };
});

vi.mock('codemirror', () => ({
  EditorView: MockEditorView,
  basicSetup: Symbol('basicSetup'),
}));

vi.mock('@codemirror/state', () => ({
  EditorState: {
    create: vi.fn(() => ({ doc: { toString: () => 'initial code' } })),
  },
}));

vi.mock('codemirror-lang-glsl', () => ({
  glsl: vi.fn(() => Symbol('glsl')),
}));

vi.mock('@codemirror/lint', () => ({
  linter: vi.fn(() => Symbol('linter')),
}));

vi.mock('@codemirror/autocomplete', () => ({
  autocompletion: vi.fn(() => Symbol('autocompletion')),
}));

vi.mock('../../src/engine/shaderLinter', () => ({
  glslLinter: vi.fn(),
}));

vi.mock('../../src/engine/shaderCompletions', () => ({
  glslCompletions: vi.fn(),
}));

import { ShaderEditor } from '../../src/components/SidePanel/ShaderEditor';

describe('ShaderEditor', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetCaptures();
  });

  it('renders editor container div', () => {
    const { container } = render(<ShaderEditor code="void main() {}" onChange={vi.fn()} />);
    const editorDiv = container.querySelector('.h-full.w-full');
    expect(editorDiv).toBeTruthy();
  });

  it('creates EditorView on mount', () => {
    render(<ShaderEditor code="void main() {}" onChange={vi.fn()} />);
    expect(MockEditorView).toHaveBeenCalledTimes(1);
    expect(getCapturedParent()).toBeTruthy();
  });

  it('destroys EditorView on unmount', () => {
    const { unmount } = render(<ShaderEditor code="void main() {}" onChange={vi.fn()} />);
    unmount();
    expect(mockEditorViewDestroy).toHaveBeenCalledTimes(1);
  });

  it('calls onChange when document changes via update listener', () => {
    const onChange = vi.fn();
    render(<ShaderEditor code="void main() {}" onChange={onChange} />);
    expect(getCapturedUpdateListener()).toBeTruthy();
    const listener = getCapturedUpdateListener();
    if (listener) {
      listener({
        docChanged: true,
        state: { doc: { toString: () => 'modified code' } },
      });
    }
    expect(onChange).toHaveBeenCalledWith('modified code');
  });

  it('does not call onChange when doc not changed', () => {
    const onChange = vi.fn();
    render(<ShaderEditor code="void main() {}" onChange={onChange} />);
    const listener = getCapturedUpdateListener();
    if (listener) {
      listener({
        docChanged: false,
        state: { doc: { toString: () => 'same' } },
      });
    }
    expect(onChange).not.toHaveBeenCalled();
  });
});
