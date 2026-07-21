import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { Node, Edge, NodeChange, EdgeChange, Connection } from '@xyflow/react';

// Mock OnnxModelManager to prevent real downloads from fire-and-forget call
vi.mock('../../src/engine/onnxModelManager', () => ({
  OnnxModelManager: class {
    loadCachedModel = vi.fn().mockResolvedValue(null);
    subscribe = vi.fn().mockReturnValue(() => {});
    getState = vi.fn().mockReturnValue({ status: 'not-downloaded', progress: 0 });
    downloadModel = vi.fn().mockResolvedValue(undefined);
  },
}));

// Mock @xyflow/react (same pattern as useGraphStore.test.ts)
vi.mock('@xyflow/react', () => ({
  applyNodeChanges: (changes: NodeChange[], nodes: Node[]) => {
    const removeIds = new Set(
      changes.filter((c): c is NodeChange & { type: 'remove'; id: string } => c.type === 'remove').map(c => c.id),
    );
    if (removeIds.size > 0) return nodes.filter(n => !removeIds.has(n.id));
    return [...nodes];
  },
  applyEdgeChanges: (changes: EdgeChange[], edges: Edge[]) => {
    const removeIds = new Set(
      changes.filter((c): c is EdgeChange & { type: 'remove'; id: string } => c.type === 'remove').map(c => c.id),
    );
    if (removeIds.size > 0) return edges.filter(e => !removeIds.has(e.id));
    return [...edges];
  },
  addEdge: (connection: Connection & { type?: string }, edges: Edge[]) => {
    const newEdge: Edge = {
      id: `e_${connection.source}_${connection.target}`,
      source: connection.source!,
      target: connection.target!,
      sourceHandle: connection.sourceHandle ?? null,
      targetHandle: connection.targetHandle ?? null,
      type: connection.type,
    };
    return [...edges, newEdge];
  },
}));

import { useGraphStore } from '../../src/store/useGraphStore';
import { ONNX_CATALOG } from '../../src/engine/onnxCatalog';

function resetStore() {
  useGraphStore.setState({
    nodes: [],
    edges: [],
    selectedNodeId: null,
    loopState: 'stopped' as const,
    projectName: 'Untitled',
    savedFilePath: null,
    outputPreviews: {},
    nodeErrors: {},
    undoStack: [],
    redoStack: [],
  });
}

describe('addOnnxNode', () => {
  beforeEach(() => {
    resetStore();
  });

  it('creates a node with correct type, label, ports, and catalog metadata for yolov8n', () => {
    const entry = ONNX_CATALOG['yolov8n'];
    useGraphStore.getState().addOnnxNode('yolov8n');
    const { nodes } = useGraphStore.getState();

    expect(nodes).toHaveLength(1);
    const node = nodes[0];

    expect(node.type).toBe('onnx');
    expect(node.data.type).toBe('onnx');
      expect(node.data.templateName).toBe(entry.label);
      expect(node.data.label).toMatch(/^yolov8n_detector_\d+$/);

    // Ports match catalog expectedIO
    expect(node.data.inputs).toHaveLength(entry.expectedIO.inputs.length);
    expect(node.data.inputs[0].label).toBe('image');
    expect(node.data.inputs[0].dataType).toBe('sampler2D');
    expect(node.data.inputs[0].direction).toBe('input');

    expect(node.data.outputs).toHaveLength(entry.expectedIO.outputs.length);
    expect(node.data.outputs[0].label).toBe('detections');
    expect(node.data.outputs[0].dataType).toBe('roi');
    expect(node.data.outputs[1].label).toBe('overlay');
    expect(node.data.outputs[1].dataType).toBe('sampler2D');

    // Port ids are namespaced to the node
    expect(node.data.inputs[0].id).toBe(`${node.id}_image`);
    expect(node.data.outputs[0].id).toBe(`${node.id}_detections`);

    // ONNX metadata
    expect(node.data.onnxModelId).toBe('yolov8n');
    expect(node.data.onnxSource).toBe('catalog');
    expect(node.data.onnxCatalogId).toBe('yolov8n');
    expect(node.data.onnxStatus).toBe('not-downloaded');
  });

  it('populates default params including scoreThreshold and iouThreshold from catalog', () => {
    useGraphStore.getState().addOnnxNode('yolov8n');
    const node = useGraphStore.getState().nodes[0];

    // yolov8n has scoreThreshold=0.25 and iouThreshold=0.45
    expect(node.data.onnxParams).toEqual({
      scoreThreshold: 0.25,
      iouThreshold: 0.45,
    });
    expect(node.data.onnxScoreThreshold).toBe(0.25);
    expect(node.data.onnxIouThreshold).toBe(0.45);
  });

  it('leaves onnxParams undefined when catalog entry has no defaultParams', () => {
    useGraphStore.getState().addOnnxNode('super-resolution-3x');
    const node = useGraphStore.getState().nodes[0];

    expect(node.data.onnxParams).toBeUndefined();
    expect(node.data.onnxScoreThreshold).toBeUndefined();
    expect(node.data.onnxIouThreshold).toBeUndefined();
  });

  it('does nothing when catalogId is unknown', () => {
    useGraphStore.getState().addOnnxNode('nonexistent-model-id');
    const { nodes, undoStack } = useGraphStore.getState();
    expect(nodes).toHaveLength(0);
    expect(undoStack).toHaveLength(0);
  });

  it('uses custom position when provided', () => {
    useGraphStore.getState().addOnnxNode('yolov8n', { x: 300, y: 450 });
    const node = useGraphStore.getState().nodes[0];
    expect(node.position).toEqual({ x: 300, y: 450 });
  });

  it('pushes history', () => {
    useGraphStore.getState().addOnnxNode('yolov8n');
    expect(useGraphStore.getState().undoStack.length).toBeGreaterThanOrEqual(1);
  });

  it('produces distinct node ids on consecutive calls', () => {
    useGraphStore.getState().addOnnxNode('yolov8n');
    useGraphStore.getState().addOnnxNode('super-resolution-3x');
    const { nodes } = useGraphStore.getState();
    expect(nodes).toHaveLength(2);
    expect(nodes[0].id).not.toBe(nodes[1].id);
  });
});

describe('addCustomOnnxNode', () => {
  beforeEach(() => {
    resetStore();
  });

  it('creates a node with type onnx, label Custom ONNX, empty ports, and custom source', () => {
    useGraphStore.getState().addCustomOnnxNode();
    const { nodes } = useGraphStore.getState();

    expect(nodes).toHaveLength(1);
    const node = nodes[0];

    expect(node.type).toBe('onnx');
    expect(node.data.type).toBe('onnx');
    expect(node.data.label).toBe('Custom ONNX');
    expect(node.data.inputs).toEqual([]);
    expect(node.data.outputs).toEqual([]);
    expect(node.data.onnxSource).toBe('custom');
    expect(node.data.onnxStatus).toBeUndefined();
  });

  it('uses custom position when provided', () => {
    useGraphStore.getState().addCustomOnnxNode({ x: 200, y: 600 });
    const node = useGraphStore.getState().nodes[0];
    expect(node.position).toEqual({ x: 200, y: 600 });
  });

  it('pushes history', () => {
    useGraphStore.getState().addCustomOnnxNode();
    expect(useGraphStore.getState().undoStack.length).toBeGreaterThanOrEqual(1);
  });
});
