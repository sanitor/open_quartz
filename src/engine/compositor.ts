import type { Node, Edge } from '@xyflow/react';
import type { ShaderNodeData } from '../types';
import { WebGPUExecutionEngine, type WebGPUExecutionPlan } from './executionEngine';

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
  async init(canvas: HTMLCanvasElement): Promise<void> {
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
  ): Promise<void>[] {
    this.plan = this.engine.prepare(nodes, edges, onNodeError, onOutputSize, onOutputData, onOutput, onOnnxComplete, this.plan, onBackendDetected);
    return this.plan?.pendingTextures ?? [];
  }

  render(inputs: FrameInputs): void {
    if (!this.plan) return;
    this.engine.runFrame(this.plan, inputs);
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
    // TODO: implement proper screenshot capture via WebGPU readback
    return null;
  }

  getCanvas(): HTMLCanvasElement | null {
    return this.engine.canvas;
  }

  dispose(): void {
    this.engine.stop();
    this.plan = null;
  }
}
