import type { Edge, Node } from '@xyflow/react';
import type { ShaderNodeData } from '../types';
import type { EngineEvent, EngineState, SdkCapabilities } from './contract';

export interface FrameInput {
  time: number;
  delta: number;
  frame: number;
  date: Float32Array;
  mouse: Float32Array;
  resolution: Float32Array;
}

export interface ModelInfo {
  inputNames: readonly string[];
  outputNames: readonly string[];
  backend: 'webgpu' | 'wasm' | 'native';
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

/** Target runtime contract. Implementations become production-ready only when gpuExecution is true. */
export interface PipelineRuntime extends StatefulEngineCore {
  readonly capabilities: SdkCapabilities;
  initialize(canvas: HTMLCanvasElement): Promise<void>;
  setPreviewNode(nodeId: string | null): void;
  uploadImage(nodeId: string, rgba: Uint8Array, width: number, height: number): void;
  attachVideo(nodeId: string, video: HTMLVideoElement): void;
  loadOnnxModel(nodeId: string, model: Uint8Array): Promise<ModelInfo>;
  readOutput(nodeId: string): Promise<Uint8Array>;
}
