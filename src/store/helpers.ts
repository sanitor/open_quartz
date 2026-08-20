import type { Node, Edge } from '@xyflow/react';
import type { ShaderNodeData } from '../types';
import type { OnnxModelDescriptor } from '../sdk/catalog';
import { Resource } from '../sdk';
import type { GraphState } from './index';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface HistoryEntry {
  /** Rust graph revision represented by this UI history marker. */
  revision: number;
  nodes: Node<ShaderNodeData>[];
  edges: Edge[];
}

export async function downloadCatalogModel(
  nodeId: string,
  entry: OnnxModelDescriptor,
  set: (fn: (state: GraphState) => void) => void,
): Promise<void> {
  set((state) => {
    const node = state.nodes.find((candidate) => candidate.id === nodeId);
    if (node) node.data.onnxStatus = 'downloading';
  });
  try {
    const prepared = await Resource.prepareCatalogOnnx(entry, (progress) => {
      set((state) => {
        const node = state.nodes.find((candidate) => candidate.id === nodeId);
        if (node) node.data.onnxProgress = progress;
      });
    });
    set((state) => {
      const node = state.nodes.find((candidate) => candidate.id === nodeId);
      if (!node) return;
      node.data.onnxStatus = 'ready';
      node.data.onnxProgress = 1;
      if (prepared.backend) node.data.onnxBackend = prepared.backend;
    });
  } catch (error) {
    set((state) => {
      const node = state.nodes.find((candidate) => candidate.id === nodeId);
      if (!node) return;
      node.data.onnxStatus = 'error';
      node.data.onnxError = error instanceof Error ? error.message : String(error);
    });
  }
}
