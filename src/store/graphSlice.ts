import type { Connection, Edge, Node, NodeChange, EdgeChange } from '@xyflow/react';
import {
  applyEdgeChanges,
  applyNodeChanges,
} from '@xyflow/react';
import type { ShaderNodeData, DataType, InputMode } from '../types';
import { SHADER_TEMPLATES } from '../catalog/predefinedShaders';
import { ONNX_CATALOG } from '../catalog/onnxCatalog';
import { Resource } from '../sdk';
import { getOnnxModelDescriptor } from '../sdk/catalog';
import type { GraphState } from './index';
import { StoreGraphAdapter } from './graphAdapter';
import { downloadCatalogModel } from './helpers';

let graphDomain: StoreGraphAdapter | null = null;

function domainFor(state: GraphState): StoreGraphAdapter {
  if (!graphDomain) graphDomain = new StoreGraphAdapter(state.projectName);
  graphDomain.ensureMatches({ nodes: state.nodes, edges: state.edges });
  return graphDomain;
}

function projectDomain(
  set: (fn: (state: GraphState) => void) => void,
  domain: StoreGraphAdapter,
): void {
  const snapshot = domain.snapshot();
  set((state) => {
    const previous = new Map(state.nodes.map((node) => [node.id, node]));
    state.nodes = snapshot.nodes.map((node) => {
      const existing = previous.get(node.id);
      return existing
        ? { ...node, position: existing.position, selected: existing.selected }
        : node;
    });
    state.edges = snapshot.edges;
  });
}

function pushHistory(
  set: (fn: (state: GraphState) => void) => void,
  revision = graphDomain?.graph.revision ?? 0,
): void {
  set((state) => {
    state.undoStack.push({ revision, nodes: [], edges: [] });
    state.redoStack = [];
    if (state.undoStack.length > 50) state.undoStack.shift();
  });
}

function runCommand(
  set: (fn: (state: GraphState) => void) => void,
  get: () => GraphState,
  command: Parameters<StoreGraphAdapter['apply']>[0],
  record = true,
): boolean {
  const domain = domainFor(get());
  try {
    domain.apply(command);
    if (record) pushHistory(set, domain.graph.revision);
    projectDomain(set, domain);
    return true;
  } catch {
    return false;
  }
}

function runNodeFactory(
  set: (fn: (state: GraphState) => void) => void,
  get: () => GraphState,
  request: Parameters<StoreGraphAdapter['createNode']>[0],
): Node<ShaderNodeData> | null {
  const domain = domainFor(get());
  try {
    const node = domain.createNode(request);
    pushHistory(set, domain.graph.revision);
    projectDomain(set, domain);
    return node;
  } catch {
    return null;
  }
}

function updateNodeData(
  set: (fn: (state: GraphState) => void) => void,
  get: () => GraphState,
  id: string,
  data: Partial<ShaderNodeData>,
): boolean {
  const current = get().nodes.find((node) => node.id === id);
  if (!current) return false;
  const nextData = { ...current.data, ...data } as ShaderNodeData;
  const command = data.shaderCode !== undefined
    ? { kind: 'updateShaderCode' as const, nodeId: id, shaderCode: data.shaderCode }
    : { kind: 'updateNodeData' as const, nodeId: id, data: nextData };
  const changed = runCommand(set, get, command, data.shaderCode !== undefined);
  if (changed) {
    const projected = get().nodes.find((node) => node.id === id);
    const parseError = projected?.data.parseError;
    set((state) => {
      if (parseError) state.nodeErrors[id] = String(parseError);
      else delete state.nodeErrors[id];
    });
  }
  return changed;
}

export function graphSlice(
  set: (fn: (state: GraphState) => void) => void,
  get: () => GraphState,
) {
  function applyUiNodeChanges(changes: NodeChange[]): void {
    set((state) => {
      state.nodes = applyNodeChanges(changes, state.nodes) as Node<ShaderNodeData>[];
    });
  }

  function applyUiEdgeChanges(changes: EdgeChange[]): void {
    set((state) => {
      state.edges = applyEdgeChanges(changes, state.edges);
    });
  }

  return {
    nodes: [] as Node<ShaderNodeData>[],
    edges: [] as Edge[],
    undoStack: [] as { revision: number; nodes: Node<ShaderNodeData>[]; edges: Edge[] }[],
    redoStack: [] as { revision: number; nodes: Node<ShaderNodeData>[]; edges: Edge[] }[],

    pushHistory: () => {
      pushHistory(set, graphDomain?.graph.revision ?? 0);
    },

    undo: () => {
      const { undoStack } = get();
      if (undoStack.length === 0) return;
      const domain = domainFor(get());
      try {
        domain.undo();
      } catch {
        return;
      }
      set((state) => {
        state.undoStack.pop();
        state.redoStack.push({ revision: domain.graph.revision, nodes: [], edges: [] });
        state.selectedNodeId = null;
      });
      projectDomain(set, domain);
    },

    redo: () => {
      const { redoStack } = get();
      if (redoStack.length === 0) return;
      const domain = domainFor(get());
      try {
        domain.redo();
      } catch {
        return;
      }
      set((state) => {
        state.redoStack.pop();
        state.undoStack.push({ revision: domain.graph.revision, nodes: [], edges: [] });
        state.selectedNodeId = null;
      });
      projectDomain(set, domain);
    },

    onNodesChange: (changes: Parameters<GraphState['onNodesChange']>[0]) => {
      const removes = changes.filter((change): change is NodeChange & { type: 'remove'; id: string } =>
        change.type === 'remove');
      for (const change of removes) {
        runCommand(set, get, { kind: 'removeNode', nodeId: change.id });
      }
      const uiChanges = changes.filter((change) => change.type !== 'remove');
      if (uiChanges.length > 0) applyUiNodeChanges(uiChanges);
    },

    onEdgesChange: (changes: Parameters<GraphState['onEdgesChange']>[0]) => {
      const removes = changes.filter((change): change is EdgeChange & { type: 'remove'; id: string } =>
        change.type === 'remove');
      for (const change of removes) {
        if (!runCommand(set, get, { kind: 'disconnect', edgeId: change.id })) {
          applyUiEdgeChanges([change]);
        }
      }
      const uiChanges = changes.filter((change) => change.type !== 'remove');
      if (uiChanges.length > 0) applyUiEdgeChanges(uiChanges);
    },

    onConnect: (connection: Parameters<GraphState['onConnect']>[0]) => {
      if (!connection.sourceHandle || !connection.targetHandle) return;
      runCommand(set, get, {
        kind: 'connect',
        source: { nodeId: connection.source, portId: connection.sourceHandle },
        target: { nodeId: connection.target, portId: connection.targetHandle },
      });
    },

    isConnectionValid: (connection: Connection | Edge) => {
      if (!connection.sourceHandle || !connection.targetHandle) return false;
      const domain = domainFor(get());
      return domain.canConnect(
        connection.source,
        connection.sourceHandle,
        connection.target,
        connection.targetHandle,
      );
    },

    addNode: (type: ShaderNodeData['type'], position?: { x: number; y: number }) => {
      if (type === 'input') {
        runNodeFactory(set, get, { kind: 'input', position, dataType: 'float' });
      } else if (type === 'shader') {
        runNodeFactory(set, get, {
          kind: 'shader',
          position,
          code: defaultShaderCode(),
          label: 'shader',
        });
      } else if (type === 'constant') {
        runNodeFactory(set, get, { kind: 'constant', position });
      } else if (type === 'renderer') {
        runNodeFactory(set, get, { kind: 'renderer', position });
      } else if (type === 'math') {
        runNodeFactory(set, get, { kind: 'math', position, op: 'add' });
      } else if (type === 'onnx') {
        runNodeFactory(set, get, { kind: 'customOnnx', position });
      }
    },

    addInputNode: (
      dataType: DataType,
      position?: { x: number; y: number },
      inputMode?: InputMode,
    ) => {
      runNodeFactory(set, get, { kind: 'input', position, dataType, inputMode });
    },

    addShaderNode: (code: string, label: string, position?: { x: number; y: number }) => {
      runNodeFactory(set, get, {
        kind: 'shader',
        position,
        code,
        label,
        templateName: label,
        shaderTemplateId: SHADER_TEMPLATES.has(label) ? label : undefined,
      });
    },

    addSystemNode: (
      source: NonNullable<ShaderNodeData['systemSource']>,
      position?: { x: number; y: number },
    ) => {
      runNodeFactory(set, get, { kind: 'system', position, source });
    },

    addOnnxNode: (catalogId: string, position?: { x: number; y: number }) => {
      const catalogEntry = ONNX_CATALOG[catalogId];
      const entry = getOnnxModelDescriptor(catalogId);
      if (!catalogEntry || !entry) return;
      const node = runNodeFactory(set, get, {
        kind: 'onnx',
        position,
        label: catalogEntry.label,
        templateName: catalogEntry.label,
        modelId: catalogId,
        catalogId,
        inputs: entry.expectedIO.inputs,
        outputs: entry.expectedIO.outputs,
      });
      if (node && entry.defaultParams) {
        const onnxParams: Record<string, number | boolean> = {};
        for (const [key, desc] of Object.entries(entry.defaultParams)) {
          onnxParams[key] = desc.default;
        }
        set((state) => {
          const current = state.nodes.find((candidate) => candidate.id === node.id);
          if (!current) return;
          current.data.onnxParams = Object.keys(onnxParams).length > 0 ? onnxParams : undefined;
          current.data.onnxScoreThreshold = typeof onnxParams.scoreThreshold === 'number'
            ? onnxParams.scoreThreshold
            : undefined;
          current.data.onnxIouThreshold = typeof onnxParams.iouThreshold === 'number'
            ? onnxParams.iouThreshold
            : undefined;
        });
      }
      if (node) void downloadCatalogModel(node.id, entry, set);
    },

    addCustomOnnxNode: (position?: { x: number; y: number }) => {
      runNodeFactory(set, get, { kind: 'customOnnx', position });
    },

    loadCustomOnnxModel: (nodeId: string, buffer: ArrayBuffer, fileName: string) => {
      updateNodeData(set, get, nodeId, {
        onnxStatus: 'introspecting',
        onnxCustomFileName: fileName,
      });
      void (async () => {
        try {
          const prepared = await Resource.prepareCustomOnnx(`custom_${nodeId}`, buffer);
          const inputs = (prepared.inputs ?? []).map((port) => ({
            ...port,
            id: `${nodeId}_${port.label}`,
          }));
          const outputs = (prepared.outputs ?? []).map((port) => ({
            ...port,
            id: `${nodeId}_${port.label}`,
          }));
          updateNodeData(set, get, nodeId, {
            label: fileName.replace(/\.onnx$/i, ''),
            inputs,
            outputs,
            onnxModelId: `custom_${nodeId}`,
            onnxStatus: 'ready',
            onnxBackend: prepared.backend,
          });
        } catch (error) {
          updateNodeData(set, get, nodeId, {
            onnxStatus: 'error',
            onnxError: error instanceof Error ? error.message : String(error),
          });
        }
      })();
    },

    addMathNode: (mathOp: string, position?: { x: number; y: number }) => {
      runNodeFactory(set, get, { kind: 'math', position, op: mathOp });
    },

    removeNode: (id: string) => {
      if (runCommand(set, get, { kind: 'removeNode', nodeId: id })) {
        set((state) => {
          if (state.selectedNodeId === id) state.selectedNodeId = null;
        });
      }
    },

    removeSelectedElements: () => {
      const { nodes, edges } = get();
      const selectedNodeIds = nodes.filter((node) => node.selected).map((node) => node.id);
      const selectedEdgeIds = edges.filter((edge) => edge.selected).map((edge) => edge.id);
      if (selectedNodeIds.length === 0 && selectedEdgeIds.length === 0) return;
      for (const id of selectedNodeIds) runCommand(set, get, { kind: 'removeNode', nodeId: id });
      for (const id of selectedEdgeIds) {
        if (!runCommand(set, get, { kind: 'disconnect', edgeId: id })) {
          set((state) => {
            state.edges = state.edges.filter((edge) => edge.id !== id);
          });
        }
      }
      set((state) => {
        if (state.selectedNodeId && selectedNodeIds.includes(state.selectedNodeId)) {
          state.selectedNodeId = null;
        }
      });
    },

    updateNodeData: (id: string, data: Partial<ShaderNodeData>) => {
      updateNodeData(set, get, id, data);
    },

    updateNodeInputType: (id: string, dataType: DataType) => {
      const node = get().nodes.find((candidate) => candidate.id === id);
      if (!node || node.data.type !== 'input') return;
      runCommand(set, get, {
        kind: 'updateInputType',
        nodeId: id,
        dataType,
        inputMode: dataType === 'sampler2D' ? 'image' : undefined,
      });
    },

    addRendererNode: (position?: { x: number; y: number }) => {
      runNodeFactory(set, get, { kind: 'renderer', position });
    },

    loadGraph: (nodes: Node<ShaderNodeData>[], edges: Edge[]) => {
      const domain = domainFor(get());
      try {
        domain.replace({ nodes, edges });
        pushHistory(set, domain.graph.revision);
        projectDomain(set, domain);
      } catch {
        domain.reset({ nodes, edges });
        set((state) => {
          state.nodes = nodes;
          state.edges = edges.map((edge) => ({ ...edge, type: 'bezier' }));
          state.selectedNodeId = null;
        });
      }
      set((state) => { state.selectedNodeId = null; });
    },

    clearGraph: () => {
      const domain = domainFor(get());
      try {
        domain.replace({ nodes: [], edges: [] });
        pushHistory(set, domain.graph.revision);
        projectDomain(set, domain);
      } catch {
        domain.reset({ nodes: [], edges: [] });
        set((state) => {
          state.nodes = [];
          state.edges = [];
        });
      }
      set((state) => {
        state.selectedNodeId = null;
        state.outputPreviews = {};
        state.outputData = {};
        state.savedFilePath = null;
        state.projectName = 'Untitled';
      });
    },
  };
}

function defaultShaderCode(): string {
  return [
    '@group(0) @binding(0) var inputImage: texture_2d<f32>;',
    '@group(0) @binding(1) var inputImageSampler: sampler;',
    '@group(0) @binding(2) var<uniform> intensity: f32;',
    '',
    '@fragment',
    'fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {',
    '  var color = textureSample(inputImage, inputImageSampler, v_uv);',
    '  color = vec4f(color.rgb * intensity, color.a);',
    '  return color;',
    '}',
  ].join('\n');
}
