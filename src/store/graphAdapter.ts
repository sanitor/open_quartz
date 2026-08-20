import type { Edge, Node as FlowNode } from '@xyflow/react';
import {
  Graph,
  OpenQuartzClient,
  Project,
  type GraphCommand,
  type NodeFactoryRequest,
} from '../sdk';
import type { ShaderNodeData } from '../types';

type FlowGraph = {
  nodes: FlowNode<ShaderNodeData>[];
  edges: Edge[];
};

/**
 * Store-facing adapter for the Rust graph aggregate.
 *
 * The adapter owns only transport lifetime and React Flow projection. It does
 * not implement graph invariants or maintain a second revision counter.
 */
export class StoreGraphAdapter {
  private project: Project;

  constructor(name = 'Untitled') {
    this.project = new OpenQuartzClient().createProject(name);
  }

  get graph(): Graph {
    return this.project.graph;
  }

  ensureMatches(flow: FlowGraph): void {
    const current = this.project.graph.snapshot();
    if (sameGraph(current, flow)) return;
    this.project.close();
    try {
      this.project = new Project('Untitled', flow.nodes, flow.edges);
    } catch {
      // A UI event may briefly contain an incomplete edge (for example while
      // React Flow is emitting a remove batch). Keep the executable aggregate
      // valid and let the next command project the cleaned UI graph.
      this.project = new Project('Untitled', flow.nodes, []);
    }
  }

  reset(flow: FlowGraph): void {
    this.project.close();
    try {
      this.project = new Project('Untitled', flow.nodes, flow.edges);
    } catch {
      this.project = new Project('Untitled', flow.nodes, []);
    }
  }

  apply(command: GraphCommand): void {
    this.graph.apply(command);
  }

  canConnect(
    sourceNodeId: string,
    sourcePortId: string,
    targetNodeId: string,
    targetPortId: string,
  ): boolean {
    return this.graph.canConnect(sourceNodeId, sourcePortId, targetNodeId, targetPortId);
  }

  createNode(request: NodeFactoryRequest): FlowNode<ShaderNodeData> {
    return this.graph.createNode(request).toFlowNode() as FlowNode<ShaderNodeData>;
  }

  replace(flow: FlowGraph): void {
    this.graph.replace(flow.nodes, flow.edges);
  }

  undo(): void {
    this.graph.rollback();
  }

  redo(): void {
    this.graph.redo();
  }

  snapshot(): FlowGraph {
    const snapshot = this.graph.snapshot();
    return {
      nodes: snapshot.nodes.map((node) => ({
        ...node,
        type: node.type ?? node.data.type,
      })) as FlowNode<ShaderNodeData>[],
      edges: snapshot.edges.map((edge) => ({
        ...edge,
        sourceHandle: edge.sourceHandle ?? null,
        targetHandle: edge.targetHandle ?? null,
        type: 'bezier',
      })),
    };
  }
}

function sameGraph(
  rustGraph: { nodes: unknown[]; edges: unknown[] },
  flow: FlowGraph,
): boolean {
  return JSON.stringify(normalize(rustGraph)) === JSON.stringify(normalize(flow));
}

function normalize(graph: { nodes: unknown[]; edges: unknown[] }): unknown {
  return {
    nodes: graph.nodes.map((node) => {
      const value = node as Record<string, unknown>;
      return {
        id: value.id,
        type: value.type ?? (value.data as Record<string, unknown> | undefined)?.type,
        data: value.data,
      };
    }),
    edges: graph.edges.map((edge) => {
      const value = edge as Record<string, unknown>;
      return {
        id: value.id,
        source: value.source,
        sourceHandle: value.sourceHandle ?? null,
        target: value.target,
        targetHandle: value.targetHandle ?? null,
      };
    }),
  };
}
