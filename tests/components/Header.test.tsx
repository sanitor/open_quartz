import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

// Mock @xyflow/react
vi.mock('@xyflow/react', () => ({
  useReactFlow: () => ({ fitView: vi.fn() }),
}));

// Mock Tauri API
vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
}));
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    setDecorations: vi.fn(),
    startDragging: vi.fn(),
    minimize: vi.fn(),
    toggleMaximize: vi.fn(),
    close: vi.fn(),
  }),
}));

// Mock projectIO
vi.mock('../../src/utils/projectIO', () => ({
  serializeProject: vi.fn(() => ({ version: '1', name: 'test', graph: { nodes: [], edges: [] } })),
  deserializeProject: vi.fn(() => ({ project: {}, nodes: [], edges: [] })),
  saveFileAs: vi.fn(),
  saveFile: vi.fn(),
  downloadProject: vi.fn(),
}));

// Mock executionEngine
vi.mock('../../src/engine/executionEngine', () => ({
  ExecutionEngine: vi.fn().mockImplementation(() => ({
    run: vi.fn().mockResolvedValue(undefined),
    stop: vi.fn(),
  })),
}));

// Store mock functions
const mockAddNode = vi.fn();
const mockAddInputNode = vi.fn();
const mockAddShaderNode = vi.fn();
const mockSetProjectName = vi.fn();
const mockSetSavedFilePath = vi.fn();
const mockSetRunning = vi.fn();
const mockLoadGraph = vi.fn();
const mockClearGraph = vi.fn();
const mockUndo = vi.fn();
const mockRedo = vi.fn();
const mockPushHistory = vi.fn();

const defaultStoreState = {
  nodes: [],
  edges: [],
  projectName: 'Untitled',
  savedFilePath: null as string | null,
  isRunning: false,
  undoStack: [] as unknown[],
  redoStack: [] as unknown[],
  setProjectName: mockSetProjectName,
  setSavedFilePath: mockSetSavedFilePath,
  setRunning: mockSetRunning,
  loadGraph: mockLoadGraph,
  clearGraph: mockClearGraph,
  undo: mockUndo,
  redo: mockRedo,
  pushHistory: mockPushHistory,
  addNode: mockAddNode,
  addInputNode: mockAddInputNode,
  addShaderNode: mockAddShaderNode,
  fitView: vi.fn(),
};

const mockUseGraphStore = vi.fn(() => defaultStoreState);

vi.mock('../../src/store/useGraphStore', () => ({
  useGraphStore: Object.assign(
    (...args: unknown[]) => mockUseGraphStore(...(args as [])),
    {
      getState: () => defaultStoreState,
    },
  ),
}));

// Mock predefined shaders
vi.mock('../../src/engine/predefinedShaders', () => ({
  CUSTOM_SHADER_CODE: 'uniform sampler2D inputImage;\nout vec4 fragColor;\nvoid main() { fragColor = vec4(1.0); }',
  CUSTOM_2IN1_SHADER: 'uniform sampler2D inputA;\nuniform sampler2D inputB;\nout vec4 fragColor;\nvoid main() { fragColor = vec4(1.0); }',
  predefinedShaders: [
    { label: 'Sobel Edge Detection', code: 'sobel-code' },
    { label: 'Gaussian Blur', code: 'blur-code' },
  ],
}));

// Mock version
vi.mock('../../src/version', () => ({
  VERSION: '0.4.2b',
}));

import { Header } from '../../src/components/Header';

function renderHeader(overrides: Record<string, unknown> = {}) {
  const state = { ...defaultStoreState, ...overrides };
  mockUseGraphStore.mockReturnValue(state);
  Object.assign(mockUseGraphStore, { getState: () => state });
  return render(<Header />);
}

describe('Header', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockUseGraphStore.mockReturnValue(defaultStoreState);
    Object.assign(mockUseGraphStore, { getState: () => defaultStoreState });
  });

  // --- Basic rendering ---

  it('renders without crashing', () => {
    renderHeader();
    expect(screen.getByText('OPENQUARTZ')).toBeInTheDocument();
  });

  it('renders version text', () => {
    renderHeader();
    expect(screen.getByText('v0.4.2b')).toBeInTheDocument();
  });

  it('renders project name', () => {
    renderHeader({ projectName: 'MyProject' });
    expect(screen.getByText('MyProject')).toBeInTheDocument();
  });

  it('renders favicon image', () => {
    renderHeader();
    const img = document.querySelector('img[src="/favicon.svg"]');
    expect(img).toBeInTheDocument();
  });

  // --- Project name editing ---

  it('shows project name as span by default', () => {
    renderHeader({ projectName: 'TestProj' });
    const nameSpan = screen.getByText('TestProj');
    expect(nameSpan.tagName).toBe('SPAN');
  });

  it('double-click on project name enables edit mode', () => {
    renderHeader({ projectName: 'TestProj' });
    const nameSpan = screen.getByText('TestProj');
    fireEvent.doubleClick(nameSpan);
    const input = document.querySelector('input[class*="border-[#007aff]"]');
    expect(input).toBeInTheDocument();
  });

  it('project name input has correct default value', () => {
    renderHeader({ projectName: 'EditMe' });
    const nameSpan = screen.getByText('EditMe');
    fireEvent.doubleClick(nameSpan);
    const input = document.querySelector('input[class*="border-[#007aff]"]') as HTMLInputElement | null;
    expect(input?.defaultValue).toBe('EditMe');
  });

  // --- Buttons ---

  it('renders SHADER button', () => {
    renderHeader();
    expect(screen.getByText('SHADER')).toBeInTheDocument();
  });

  it('renders INPUT button', () => {
    renderHeader();
    expect(screen.getByText('INPUT')).toBeInTheDocument();
  });

  it('renders OUTPUT button', () => {
    renderHeader();
    expect(screen.getByText('OUTPUT')).toBeInTheDocument();
  });

  it('OUTPUT button click calls addNode', () => {
    renderHeader();
    const outputBtn = screen.getByText('OUTPUT').closest('button')!;
    fireEvent.click(outputBtn);
    expect(mockAddNode).toHaveBeenCalledWith('output');
  });

  it('renders SAVE button', () => {
    renderHeader();
    expect(screen.getByText('SAVE')).toBeInTheDocument();
  });

  it('renders SAVE AS button', () => {
    renderHeader();
    expect(screen.getByText('SAVE AS')).toBeInTheDocument();
  });

  it('renders LOAD button', () => {
    renderHeader();
    expect(screen.getByText('LOAD')).toBeInTheDocument();
  });

  it('SAVE button is disabled when no savedFilePath', () => {
    renderHeader({ savedFilePath: null });
    const saveBtn = screen.getByText('SAVE').closest('button')!;
    expect(saveBtn).toBeDisabled();
  });

  it('SAVE button is enabled when savedFilePath exists', () => {
    renderHeader({ savedFilePath: 'test.quartz.json' });
    const saveBtn = screen.getByText('SAVE').closest('button')!;
    expect(saveBtn).not.toBeDisabled();
  });

  it('SAVE AS button is disabled when no nodes', () => {
    renderHeader({ nodes: [] });
    const saveAsBtn = screen.getByText('SAVE AS').closest('button')!;
    expect(saveAsBtn).toBeDisabled();
  });

  it('SAVE AS button is enabled when nodes exist', () => {
    renderHeader({ nodes: [{ id: '1', type: 'shader', position: { x: 0, y: 0 }, data: {} }] });
    const saveAsBtn = screen.getByText('SAVE AS').closest('button')!;
    expect(saveAsBtn).not.toBeDisabled();
  });

  // --- RUN/STOP button ---

  it('renders RUN button when not running', () => {
    renderHeader({ isRunning: false });
    expect(screen.getByText('RUN')).toBeInTheDocument();
  });

  it('RUN button click calls setRunning', () => {
    renderHeader({ isRunning: false });
    const runBtn = screen.getByText('RUN').closest('button')!;
    fireEvent.click(runBtn);
    expect(mockSetRunning).toHaveBeenCalledWith(true);
  });

  it('renders STOP when running', () => {
    renderHeader({ isRunning: true });
    expect(screen.getByText('STOP')).toBeInTheDocument();
    expect(screen.queryByText('RUN')).not.toBeInTheDocument();
  });

  it('STOP button click calls setRunning(false)', () => {
    renderHeader({ isRunning: true });
    const stopBtn = screen.getByText('STOP').closest('button')!;
    fireEvent.click(stopBtn);
    expect(mockSetRunning).toHaveBeenCalledWith(false);
  });

  // --- CLEAR ---

  it('renders CLEAR button', () => {
    renderHeader();
    expect(screen.getByText('CLEAR')).toBeInTheDocument();
  });

  it('CLEAR button click calls clearGraph', () => {
    renderHeader();
    const clearBtn = screen.getByText('CLEAR').closest('button')!;
    fireEvent.click(clearBtn);
    expect(mockClearGraph).toHaveBeenCalled();
  });

  // --- UNDO / REDO buttons ---

  it('renders UNDO button', () => {
    renderHeader();
    expect(screen.getByText('UNDO')).toBeInTheDocument();
  });

  it('renders REDO button', () => {
    renderHeader();
    expect(screen.getByText('REDO')).toBeInTheDocument();
  });

  it('UNDO button is disabled when undoStack is empty', () => {
    renderHeader({ undoStack: [] });
    const undoBtn = screen.getByText('UNDO').closest('button')!;
    expect(undoBtn).toBeDisabled();
  });

  it('REDO button is disabled when redoStack is empty', () => {
    renderHeader({ redoStack: [] });
    const redoBtn = screen.getByText('REDO').closest('button')!;
    expect(redoBtn).toBeDisabled();
  });

  it('UNDO button is enabled when undoStack has entries', () => {
    renderHeader({ undoStack: [{ nodes: [], edges: [] }] });
    const undoBtn = screen.getByText('UNDO').closest('button')!;
    expect(undoBtn).not.toBeDisabled();
  });

  it('REDO button is enabled when redoStack has entries', () => {
    renderHeader({ redoStack: [{ nodes: [], edges: [] }] });
    const redoBtn = screen.getByText('REDO').closest('button')!;
    expect(redoBtn).not.toBeDisabled();
  });

  it('UNDO button click calls undo', () => {
    renderHeader({ undoStack: [{ nodes: [], edges: [] }] });
    const undoBtn = screen.getByText('UNDO').closest('button')!;
    fireEvent.click(undoBtn);
    expect(mockUndo).toHaveBeenCalled();
  });

  it('REDO button click calls redo', () => {
    renderHeader({ redoStack: [{ nodes: [], edges: [] }] });
    const redoBtn = screen.getByText('REDO').closest('button')!;
    fireEvent.click(redoBtn);
    expect(mockRedo).toHaveBeenCalled();
  });

  // --- Keyboard shortcuts ---

  it('Ctrl+Z calls undo', () => {
    renderHeader();
    fireEvent.keyDown(document, { key: 'z', ctrlKey: true });
    expect(mockUndo).toHaveBeenCalled();
  });

  it('Ctrl+Shift+Z calls redo', () => {
    renderHeader();
    fireEvent.keyDown(document, { key: 'z', ctrlKey: true, shiftKey: true });
    expect(mockRedo).toHaveBeenCalled();
  });

  it('Ctrl+Y calls redo', () => {
    renderHeader();
    fireEvent.keyDown(document, { key: 'y', ctrlKey: true });
    expect(mockRedo).toHaveBeenCalled();
  });

  // --- SHADER dropdown ---

  it('clicking SHADER button opens shader dropdown', () => {
    renderHeader();
    const shaderBtn = screen.getByText('SHADER').closest('button')!;
    fireEvent.click(shaderBtn);
    expect(screen.getByText('CUSTOM SHADER')).toBeInTheDocument();
    expect(screen.getByText('CUSTOM 2IN-1OUT')).toBeInTheDocument();
  });

  it('shader dropdown shows predefined shaders', () => {
    renderHeader();
    const shaderBtn = screen.getByText('SHADER').closest('button')!;
    fireEvent.click(shaderBtn);
    expect(screen.getByText('Sobel Edge Detection')).toBeInTheDocument();
    expect(screen.getByText('Gaussian Blur')).toBeInTheDocument();
  });

  it('clicking a shader item calls addShaderNode and closes dropdown', () => {
    renderHeader();
    const shaderBtn = screen.getByText('SHADER').closest('button')!;
    fireEvent.click(shaderBtn);
    const customShader = screen.getByText('CUSTOM SHADER');
    fireEvent.click(customShader);
    expect(mockAddShaderNode).toHaveBeenCalled();
    expect(screen.queryByText('CUSTOM 2IN-1OUT')).not.toBeInTheDocument();
  });

  // --- INPUT dropdown ---

  it('clicking INPUT button opens input dropdown', () => {
    renderHeader();
    const inputBtn = screen.getByText('INPUT').closest('button')!;
    fireEvent.click(inputBtn);
    expect(screen.getByText('SCALAR')).toBeInTheDocument();
    expect(screen.getByText('VECTOR')).toBeInTheDocument();
    expect(screen.getByText('SAMPLER2D')).toBeInTheDocument();
  });

  it('hovering over input group shows sub-items', () => {
    renderHeader();
    const inputBtn = screen.getByText('INPUT').closest('button')!;
    fireEvent.click(inputBtn);
    const scalarGroup = screen.getByText('SCALAR');
    fireEvent.mouseEnter(scalarGroup.closest('.relative')!);
    expect(screen.getByText('FLOAT')).toBeInTheDocument();
    expect(screen.getByText('INT')).toBeInTheDocument();
    expect(screen.getByText('BOOL')).toBeInTheDocument();
  });

  it('clicking input sub-item calls addInputNode', () => {
    renderHeader();
    const inputBtn = screen.getByText('INPUT').closest('button')!;
    fireEvent.click(inputBtn);
    const scalarGroup = screen.getByText('SCALAR');
    fireEvent.mouseEnter(scalarGroup.closest('.relative')!);
    const floatBtn = screen.getByText('FLOAT');
    fireEvent.click(floatBtn);
    expect(mockAddInputNode).toHaveBeenCalledWith('float', undefined, undefined);
  });

  // --- SAVE AS dialog ---

  it('clicking SAVE AS opens save-as dialog', () => {
    renderHeader({ nodes: [{ id: '1', type: 'shader', position: { x: 0, y: 0 }, data: {} }] });
    const saveAsBtn = screen.getByText('SAVE AS').closest('button')!;
    fireEvent.click(saveAsBtn);
    expect(screen.getByText('SAVE AS', { selector: 'div' })).toBeInTheDocument();
    expect(screen.getByText('CANCEL')).toBeInTheDocument();
  });

  // --- LOAD button triggers file input ---

  it('LOAD button click triggers hidden file input', () => {
    renderHeader();
    const fileInput = document.querySelector('input[type="file"]') as HTMLInputElement | null;
    const clickSpy = vi.spyOn(fileInput!, 'click');
    const loadBtn = screen.getByText('LOAD').closest('button')!;
    fireEvent.click(loadBtn);
    expect(clickSpy).toHaveBeenCalled();
  });

  it('renders file input with correct accept attribute', () => {
    renderHeader();
    const fileInput = document.querySelector('input[type="file"]') as HTMLInputElement | null;
    expect(fileInput).toBeInTheDocument();
    expect(fileInput?.accept).toBe('.quartz.json,.json');
  });

  // --- Header structure ---

  it('renders header element', () => {
    renderHeader();
    const header = document.querySelector('header');
    expect(header).toBeInTheDocument();
  });

  it('renders run icon as ▷ when not running', () => {
    renderHeader({ isRunning: false });
    expect(screen.getByText('▷')).toBeInTheDocument();
  });

  it('renders stop icon as □ when running', () => {
    renderHeader({ isRunning: true });
    expect(screen.getByText('□')).toBeInTheDocument();
  });

  it('renders ↩ icon for undo', () => {
    renderHeader();
    expect(screen.getByText('↩')).toBeInTheDocument();
  });

  it('renders ↪ icon for redo', () => {
    renderHeader();
    expect(screen.getByText('↪')).toBeInTheDocument();
  });

  it('renders ✕ icon for clear', () => {
    renderHeader();
    expect(screen.getByText('✕')).toBeInTheDocument();
  });
});
