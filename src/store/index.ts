import { create } from 'zustand';
import { immer } from 'zustand/middleware/immer';
import type { Node, Edge, OnNodesChange, OnEdgesChange, OnConnect } from '@xyflow/react';
import type { ShaderNodeData, DataType, InputMode } from '../types';
import type { HistoryEntry } from './helpers';
import { graphSlice } from './graphSlice';
import { transportSlice } from './transportSlice';
import { projectSlice } from './projectSlice';
import { uiSlice } from './uiSlice';

export type { HistoryEntry } from './helpers';
export { modelManager } from './helpers';

export interface GraphState {
  nodes: Node<ShaderNodeData>[];
  edges: Edge[];
  selectedNodeId: string | null;
  projectName: string;
  savedFilePath: string | null;
  outputPreviews: Record<string, string>;
  outputData: Record<string, unknown>;
  nodeErrors: Record<string, string>;
  loopState: 'stopped' | 'playing' | 'paused';
  fps: number;
  currentTime: number;
  currentFrame: number;
  activeRendererId: string | null;

  undoStack: HistoryEntry[];
  redoStack: HistoryEntry[];

  pushHistory: () => void;
  undo: () => void;
  redo: () => void;
  onNodesChange: OnNodesChange;
  onEdgesChange: OnEdgesChange;
  onConnect: OnConnect;
  addNode: (type: ShaderNodeData['type'], position?: { x: number; y: number }) => void;
  addInputNode: (dataType: DataType, position?: { x: number; y: number }, inputMode?: InputMode) => void;
  addSystemNode: (source: NonNullable<ShaderNodeData['systemSource']>, position?: { x: number; y: number }) => void;
  addShaderNode: (code: string, label: string, position?: { x: number; y: number }) => void;
  addOnnxNode: (catalogId: string, position?: { x: number; y: number }) => void;
  addCustomOnnxNode: (position?: { x: number; y: number }) => void;
  loadCustomOnnxModel: (nodeId: string, buffer: ArrayBuffer, fileName: string) => void;
  addMathNode: (mathOp: string, position?: { x: number; y: number }) => void;
  removeNode: (id: string) => void;
  removeSelectedElements: () => void;
  updateNodeData: (id: string, data: Partial<ShaderNodeData>) => void;
  updateNodeInputType: (id: string, dataType: DataType) => void;
  setSelectedNode: (id: string | null) => void;
  setProjectName: (name: string) => void;
  setSavedFilePath: (path: string | null) => void;
  setOutputPreview: (nodeId: string, dataUrl: string) => void;
  setOutputData: (nodeId: string, data: unknown) => void;
  clearOutputPreviews: () => void;
  play: () => void;
  pause: () => void;
  resume: () => void;
  setFps: (fps: number) => void;
  setCurrentTime: (t: number) => void;
  setCurrentFrame: (frame: number) => void;
  setActiveRenderer: (id: string | null) => void;
  addRendererNode: (position?: { x: number; y: number }) => void;
  stop: () => void;
  setNodeError: (nodeId: string, error: string | null) => void;
  clearNodeErrors: () => void;
  loadGraph: (nodes: Node<ShaderNodeData>[], edges: Edge[]) => void;
  clearGraph: () => void;
  captureScreenshot: ((rendererId: string) => string | null) | null;
  setCaptureScreenshot: (fn: ((rendererId: string) => string | null) | null) => void;
}

export const useGraphStore = create<GraphState>()(
  immer((set, get) => ({
    ...graphSlice(set, get),
    ...transportSlice(set, get),
    ...projectSlice(set, get),
    ...uiSlice(set, get),
  })),
);
