import type { Edge, Node } from '@xyflow/react';
import type { ShaderNodeData } from '../types';
import type { EngineEvent, EngineState } from './contract';

export interface FrameInput {
  time: number;
  delta: number;
  frame: number;
  date: Float32Array;
  mouse: Float32Array;
  resolution: Float32Array;
}


export interface StatefulEngineCore {
  readonly state: EngineState;
  readonly revision: number;
  readonly lastFrame: number | null;
  readonly pendingCommandCount: number;
  setGraph(nodes: Node<ShaderNodeData>[], edges: Edge[]): number;
  markDirty(nodeId: string): void;
  runFrame(input: FrameInput): void;
  setVideoNodes(nodeIds: readonly string[]): void;
  nodeGeneration(nodeId: string): number;
  drainEvents(): EngineEvent[];
  pause(): void;
  resume(): void;
  stop(): void;
  dispose(): void;
}


export interface RuntimeFrame {
  frame: number;
  time: number;
  fps: number;
}

export interface RuntimeVideoDevice {
  id: string;
  label: string;
}

export interface PipelineRuntimeCallbacks {
  onFrame?: (frame: RuntimeFrame) => void;
  onOutput?: (nodeId: string, dataUrl: string) => void;
  onRendererPresented?: (nodeId: string) => void;
  onNodeError?: (nodeId: string | null, error: string) => void;
  onOutputSize?: (nodeId: string, width: number, height: number) => void;
  onOutputData?: (nodeId: string, data: unknown) => void;
  onBackendDetected?: (nodeId: string, backend: 'webgpu' | 'wasm' | 'native') => void;
}

/** Host-level runtime contract used by PipelineService for browser/native selection. */
export interface PipelineHostRuntime {
  readonly frameScheduling: 'client' | 'runtime';
  initialize(canvas: HTMLCanvasElement): Promise<unknown>;
  play(nodes: Node<ShaderNodeData>[], edges: Edge[]): Promise<void>;
  updateGraph(nodes: Node<ShaderNodeData>[], edges: Edge[]): Promise<unknown> | void;
  pause(): Promise<void> | void;
  resume(): Promise<void> | void;
  stop(): Promise<void> | void;
  setPreviewNode(nodeId: string | null): void;
  requestPreviewRefresh?(): void;
  captureScreenshot(nodeId: string): Promise<string | null>;
  listVideoDevices?(): Promise<RuntimeVideoDevice[]>;
  close(): Promise<void> | void;
}
