import type { Edge, Node } from '@xyflow/react';
import type { ShaderNodeData } from '../types';

export type BrowserWorkerRequest =
  | { id: number; type: 'initialize'; canvas: OffscreenCanvas }
  | { id: number; type: 'play'; nodes: Node<ShaderNodeData>[]; edges: Edge[] }
  | { id: number; type: 'update-graph'; nodes: Node<ShaderNodeData>[]; edges: Edge[] }
  | { id: number; type: 'pause' | 'resume' | 'stop' | 'close' }
  | { id: number; type: 'set-preview'; nodeId: string | null }
  | { id: number; type: 'capture'; nodeId: string };


export type BrowserWorkerRequestPayload = BrowserWorkerRequest extends infer Request
  ? Request extends { id: number }
    ? Omit<Request, 'id'>
    : never
  : never;
export type BrowserWorkerEvent =
  | { type: 'frame'; frame: number; time: number; fps: number }
  | { type: 'output'; nodeId: string; dataUrl: string }
  | { type: 'output-size'; nodeId: string; width: number; height: number }
  | { type: 'output-data'; nodeId: string; data: unknown }
  | { type: 'node-error'; nodeId: string | null; error: string }
  | { type: 'backend'; nodeId: string; backend: 'webgpu' | 'wasm' };

export type BrowserWorkerResponse =
  | { id: number; ok: true; value?: unknown }
  | { id: number; ok: false; error: string }
  | BrowserWorkerEvent;
