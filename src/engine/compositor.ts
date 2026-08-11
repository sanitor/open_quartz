import type { Node, Edge } from '@xyflow/react';
import type { ShaderNodeData } from '../types';
import {
  WebGPUExecutionEngine,
  type CanonicalExecutionPlan,
  type RuntimeWorkCommand,
  type WebGPUExecutionPlan,
} from './executionEngine';

export interface FrameInputs {
  time: number;
  delta: number;
  frame: number;
  date: Float32Array;
  mouse: Float32Array;
  resolution: Float32Array;
  videoElements?: Map<string, HTMLVideoElement>;
}

export class Compositor {
  private engine: WebGPUExecutionEngine;
  private plan: WebGPUExecutionPlan | null = null;

  constructor() {
    this.engine = new WebGPUExecutionEngine();
  }

  /** Async init — must be called before prepare/render. */
  async init(canvas: HTMLCanvasElement | OffscreenCanvas): Promise<void> {
    await this.engine.init(canvas);
  }

  get device(): GPUDevice | null {
    return this.engine.device;
  }

  prepare(
    nodes: Node<ShaderNodeData>[],
    edges: Edge[],
    onNodeError?: (nodeId: string, error: string) => void,
    onOutputSize?: (nodeId: string, width: number, height: number) => void,
    onOutputData?: (nodeId: string, data: unknown) => void,
    onOutput?: (nodeId: string, dataUrl: string) => void,
    onOnnxComplete?: () => void,
    onBackendDetected?: (nodeId: string, backend: 'webgpu' | 'wasm') => void,
    canonicalPlan?: CanonicalExecutionPlan,
  ): Promise<void>[] {
    this.plan = this.engine.prepare(nodes, edges, onNodeError, onOutputSize, onOutputData, onOutput, onOnnxComplete, this.plan, onBackendDetected, canonicalPlan);
    return this.plan?.pendingTextures ?? [];
  }

  render(inputs: FrameInputs, commands?: readonly RuntimeWorkCommand[]): void {
    if (!this.plan) return;
    this.engine.runFrame(this.plan, inputs, commands);
  }

  async readOutputs(onOutput: (nodeId: string, dataUrl: string) => void): Promise<void> {
    if (!this.plan) return;
    await this.engine.readOutputs(this.plan, onOutput);
  }

  renderRendererToScreen(rendererNodeId: string): void {
    if (!this.plan) return;
    this.engine.renderRendererToScreen(this.plan, rendererNodeId);
  }

  /** Read back a single node's output as a data URL. */
  async readNodeOutput(nodeId: string, onOutput: (nodeId: string, dataUrl: string) => void): Promise<void> {
    if (!this.plan) return;
    // For now, use readOutputs filtered to the single node
    await this.engine.readOutputs(this.plan, (id, url) => {
      if (id === nodeId) onOutput(id, url);
    });
  }

  async captureScreenshot(rendererNodeId: string): Promise<string | null> {
    let screenshot: string | null = null;
    await this.readNodeOutput(rendererNodeId, (_nodeId, dataUrl) => {
      screenshot = dataUrl;
    });
    return screenshot;
  }

  getCanvas(): HTMLCanvasElement | OffscreenCanvas | null {
    return this.engine.canvas;
  }

  dispose(): void {
    this.engine.stop();
    this.plan = null;
  }
}
