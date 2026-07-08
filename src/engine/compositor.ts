import * as THREE from 'three';
import type { Node, Edge } from '@xyflow/react';
import type { ShaderNodeData } from '../types';
import { ExecutionEngine, type ExecutionPlan } from './executionEngine';

export interface FrameInputs {
  time: number;
  delta: number;
  frame: number;
  date: Float32Array;
  mouse: Float32Array;
  resolution: Float32Array;
  videoTextures?: Map<string, THREE.Texture>;
}

export class Compositor {
  private engine: ExecutionEngine;
  private plan: ExecutionPlan | null = null;

  constructor(canvas: HTMLCanvasElement) {
    this.engine = new ExecutionEngine(canvas);
  }

  prepare(
    nodes: Node<ShaderNodeData>[],
    edges: Edge[],
    onNodeError?: (nodeId: string, error: string) => void,
    onOutputSize?: (nodeId: string, width: number, height: number) => void,
  ): boolean {
    this.plan = this.engine.prepare(nodes, edges, onNodeError, onOutputSize);
    return this.plan !== null;
  }

  render(inputs: FrameInputs): void {
    if (!this.plan) return;
    this.engine.runFrame(this.plan, inputs);
  }

  readOutputs(onOutput: (nodeId: string, dataUrl: string) => void): void {
    if (!this.plan) return;
    this.engine.readOutputs(this.plan, onOutput);
  }

  dispose(): void {
    this.engine.stop();
    this.plan = null;
  }
}
