// Mock onnxRegistry
vi.mock('../../src/engine/onnxRegistry', () => ({
  ONNX_MODELS: {
    yolov8n: {
      id: 'yolov8n',
      label: 'YOLOv8n Detector',
      description: 'Test ONNX model',
      inputs: [{ id: 'in', label: 'image', dataType: 'sampler2D', direction: 'input' }],
      outputs: [{ id: 'out', label: 'detections', dataType: 'roi', direction: 'output' }],
    },
  },
}));

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
const mockLoadGraph = vi.fn();
const mockClearGraph = vi.fn();
const mockUndo = vi.fn();
const mockRedo = vi.fn();
const mockPushHistory = vi.fn();
const mockPlay = vi.fn();
const mockPause = vi.fn();
const mockResume = vi.fn();
const mockStop = vi.fn();
const mockSetFps = vi.fn();
const mockSetCurrentTime = vi.fn();
const mockSetCurrentFrame = vi.fn();
const mockSetActiveRenderer = vi.fn();
const mockAddRendererNode = vi.fn();
const mockSetCaptureScreenshot = vi.fn();

const defaultStoreState = {
  nodes: [],
  edges: [],
  projectName: 'Untitled',
  savedFilePath: null as string | null,
  loopState: 'stopped' as string,
  fps: 0,
  currentTime: 0,
  currentFrame: 0,
  activeRendererId: null as string | null,
  undoStack: [] as unknown[],
  redoStack: [] as unknown[],
  setProjectName: mockSetProjectName,
  setSavedFilePath: mockSetSavedFilePath,
  loadGraph: mockLoadGraph,
  clearGraph: mockClearGraph,
  undo: mockUndo,
  redo: mockRedo,
  pushHistory: mockPushHistory,
  addNode: mockAddNode,
  addInputNode: mockAddInputNode,
  addShaderNode: mockAddShaderNode,
  addRendererNode: mockAddRendererNode,
  fitView: vi.fn(),
  play: mockPlay,
  pause: mockPause,
  resume: mockResume,
  stop: mockStop,
  setFps: mockSetFps,
  setCurrentTime: mockSetCurrentTime,
  setCurrentFrame: mockSetCurrentFrame,
  setActiveRenderer: mockSetActiveRenderer,
  captureScreenshot: null as string | null,
  setCaptureScreenshot: mockSetCaptureScreenshot,
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

  it('renders SOURCE button', () => {
    renderHeader();
    expect(screen.getByText('SOURCE')).toBeInTheDocument();
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

  // --- PLAY/PAUSE/STOP transport ---

  it('renders PLAY button when stopped', () => {
    renderHeader({ loopState: 'stopped' });
    expect(screen.getByText('PLAY')).toBeInTheDocument();
  });

  it('PLAY button click calls play', () => {
    renderHeader({ loopState: 'stopped' });
    const playBtn = screen.getByText('PLAY').closest('button')!;
    fireEvent.click(playBtn);
    expect(mockPlay).toHaveBeenCalled();
  });

  it('renders PAUSE when playing', () => {
    renderHeader({ loopState: 'playing' });
    expect(screen.getByText('PAUSE')).toBeInTheDocument();
    expect(screen.queryByText('PLAY')).not.toBeInTheDocument();
  });

  it('PAUSE button click calls pause', () => {
    renderHeader({ loopState: 'playing' });
    const pauseBtn = screen.getByText('PAUSE').closest('button')!;
    fireEvent.click(pauseBtn);
    expect(mockPause).toHaveBeenCalled();
  });

  it('renders STOP when playing', () => {
    renderHeader({ loopState: 'playing' });
    expect(screen.getByText('STOP')).toBeInTheDocument();
  });

  it('STOP button click calls stop', () => {
    renderHeader({ loopState: 'playing' });
    const stopBtn = screen.getByText('STOP').closest('button')!;
    fireEvent.click(stopBtn);
    expect(mockStop).toHaveBeenCalled();
  });

  it('renders RESUME when paused', () => {
    renderHeader({ loopState: 'paused' });
    expect(screen.getByText('RESUME')).toBeInTheDocument();
  });

  it('RESUME button click calls resume', () => {
    renderHeader({ loopState: 'paused' });
    const resumeBtn = screen.getByText('RESUME').closest('button')!;
    fireEvent.click(resumeBtn);
    expect(mockResume).toHaveBeenCalled();
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
    expect(screen.getByText('CUSTOM 2IN-1')).toBeInTheDocument();
  });

  it('shader dropdown shows category groups', () => {
    renderHeader();
    const shaderBtn = screen.getByText('SHADER').closest('button')!;
    fireEvent.click(shaderBtn);
    expect(screen.getByText('FILTER')).toBeInTheDocument();
    expect(screen.getByText('COLOR')).toBeInTheDocument();
    expect(screen.getByText('GENERATOR')).toBeInTheDocument();
    expect(screen.getByText('BLEND')).toBeInTheDocument();
    expect(screen.getByText('DISTORTION')).toBeInTheDocument();
  });

  it('clicking a shader item calls addShaderNode and closes dropdown', () => {
    renderHeader();
    const shaderBtn = screen.getByText('SHADER').closest('button')!;
    fireEvent.click(shaderBtn);
    const customShader = screen.getByText('CUSTOM SHADER');
    fireEvent.click(customShader);
    expect(mockAddShaderNode).toHaveBeenCalled();
    expect(screen.queryByText('CUSTOM 2IN-1')).not.toBeInTheDocument();
  });

  // --- SOURCE dropdown ---

  it('clicking SOURCE button opens source dropdown', () => {
    renderHeader();
    const sourceBtn = screen.getByText('SOURCE').closest('button')!;
    fireEvent.click(sourceBtn);
    expect(screen.getByText('SYSTEM')).toBeInTheDocument();
    expect(screen.getByText('CONSTANTS')).toBeInTheDocument();
    expect(screen.getByText('EXTERNAL')).toBeInTheDocument();
  });

  it('hovering over source group shows sub-items', () => {
    renderHeader();
    const sourceBtn = screen.getByText('SOURCE').closest('button')!;
    fireEvent.click(sourceBtn);
    const constantsGroup = screen.getByText('CONSTANTS');
    fireEvent.mouseEnter(constantsGroup.closest('.relative')!);
    expect(screen.getByText('FLOAT')).toBeInTheDocument();
    expect(screen.getByText('INT')).toBeInTheDocument();
  });

  it('clicking source sub-item calls addInputNode', () => {
    renderHeader();
    const sourceBtn = screen.getByText('SOURCE').closest('button')!;
    fireEvent.click(sourceBtn);
    const constantsGroup = screen.getByText('CONSTANTS');
    fireEvent.mouseEnter(constantsGroup.closest('.relative')!);
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

  it('renders play icon as ▶ when stopped', () => {
    renderHeader({ loopState: 'stopped' });
    expect(screen.getByText('▶')).toBeInTheDocument();
  });

  it('renders pause icon as ⏸ when playing', () => {
    renderHeader({ loopState: 'playing' });
    expect(screen.getByText('⏸')).toBeInTheDocument();
  });

  it('renders stop icon as ■ when playing', () => {
    renderHeader({ loopState: 'playing' });
    expect(screen.getByText('■')).toBeInTheDocument();
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

  // --- RENDERER button ---

  it('renders RENDERER button', () => {
    renderHeader();
    expect(screen.getByText('RENDERER')).toBeInTheDocument();
  });

  it('RENDERER button click calls addRendererNode', () => {
    renderHeader();
    const btn = screen.getByText('RENDERER').closest('button')!;
    fireEvent.click(btn);
    expect(mockAddRendererNode).toHaveBeenCalled();
  });

  // --- ONNX button ---

  it('renders ONNX button', () => {
    renderHeader();
    expect(screen.getByText('ONNX')).toBeInTheDocument();
  });

  it('clicking ONNX button opens onnx dropdown', () => {
    renderHeader();
    const btn = screen.getByText('ONNX').closest('button')!;
    fireEvent.click(btn);
    expect(screen.getByText('YOLOV8N DETECTOR')).toBeInTheDocument();
  });

  // --- FPS display ---

  it('FPS display hidden when stopped', () => {
    renderHeader({ loopState: 'stopped' });
    expect(screen.queryByText(/FPS/)).not.toBeInTheDocument();
  });

  it('FPS display shown when playing', () => {
    renderHeader({ loopState: 'playing', fps: 60 });
    expect(screen.getByText('60 FPS')).toBeInTheDocument();
  });

  it('FPS display shows placeholder when fps=0 and playing', () => {
    renderHeader({ loopState: 'playing', fps: 0 });
    expect(screen.getByText('-- FPS')).toBeInTheDocument();
  });

  // --- Time display ---

  it('time display hidden when stopped', () => {
    renderHeader({ loopState: 'stopped' });
    expect(screen.queryByText(/\ds$/)).not.toBeInTheDocument();
  });

  it('time display shown when playing', () => {
    renderHeader({ loopState: 'playing', currentTime: 3.7 });
    expect(screen.getByText('3.7s')).toBeInTheDocument();
  });

  it('time display shown when paused', () => {
    renderHeader({ loopState: 'paused', currentTime: 1.5 });
    expect(screen.getByText('1.5s')).toBeInTheDocument();
  });

  // --- STOP shown when paused ---

  it('STOP button shown when paused', () => {
    renderHeader({ loopState: 'paused' });
    expect(screen.getByText('STOP')).toBeInTheDocument();
  });

  it('STOP button click when paused calls stop', () => {
    renderHeader({ loopState: 'paused' });
    const stopBtn = screen.getByText('STOP').closest('button')!;
    fireEvent.click(stopBtn);
    expect(mockStop).toHaveBeenCalled();
  });
});
