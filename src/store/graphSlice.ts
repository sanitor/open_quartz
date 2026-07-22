import type { Node, Edge } from '@xyflow/react';
import {
  applyNodeChanges,
  applyEdgeChanges,
  addEdge,
} from '@xyflow/react';
import type { ShaderNodeData, DataType, InputMode } from '../types';
import { parseShader } from '../engine/shaderParser';
import { SHADER_TEMPLATES } from '../catalog/predefinedShaders';
import { MATH_OPS, getMathPorts } from '../catalog/mathOps';
import { ONNX_CATALOG } from '../catalog/onnxCatalog';
import { OnnxInferenceSession } from '../engine/onnx/inference';
import { introspectOnnxModel, metaToDefaultPorts } from '../engine/onnx/introspect';
import type { GraphState } from './index';
import {
  counters,
  makeNode,
  syncCounters,
  createInputShader,
  modelManager,
  downloadCatalogModel,
  SYSTEM_SOURCES,
} from './helpers';

export function graphSlice(
  set: (fn: (state: GraphState) => void) => void,
  get: () => GraphState,
) {
  function saveSnapshot() {
    const { nodes, edges } = get();
    const entry = { nodes: structuredClone(nodes), edges: structuredClone(edges) };
    set((state) => {
      state.undoStack.push(entry);
      state.redoStack = [];
      if (state.undoStack.length > 50) state.undoStack.shift();
    });
  }

  return {
    nodes: [] as Node<ShaderNodeData>[],
    edges: [] as Edge[],
    undoStack: [] as { nodes: Node<ShaderNodeData>[]; edges: Edge[] }[],
    redoStack: [] as { nodes: Node<ShaderNodeData>[]; edges: Edge[] }[],

    pushHistory: saveSnapshot,

    undo: () => {
      const { undoStack } = get();
      if (undoStack.length === 0) return;
      set((state) => {
        const prev = state.undoStack.pop()!;
        state.redoStack.push({ nodes: structuredClone(get().nodes), edges: structuredClone(get().edges) });
        state.nodes = prev.nodes;
        state.edges = prev.edges;
        state.selectedNodeId = null;
      });
      syncCounters(get().nodes);
    },

    redo: () => {
      const { redoStack } = get();
      if (redoStack.length === 0) return;
      set((state) => {
        const next = state.redoStack.pop()!;
        state.undoStack.push({ nodes: structuredClone(get().nodes), edges: structuredClone(get().edges) });
        state.nodes = next.nodes;
        state.edges = next.edges;
        state.selectedNodeId = null;
      });
      syncCounters(get().nodes);
    },

    onNodesChange: (changes: Parameters<GraphState['onNodesChange']>[0]) => {
      set((state) => {
        state.nodes = applyNodeChanges(changes, state.nodes) as unknown as Node<ShaderNodeData>[];
      });
    },

    onEdgesChange: (changes: Parameters<GraphState['onEdgesChange']>[0]) => {
      set((state) => {
        state.edges = applyEdgeChanges(changes, state.edges);
      });
    },

    onConnect: (connection: Parameters<GraphState['onConnect']>[0]) => {
      const { nodes } = get();
      const sourceNode = nodes.find((n) => n.id === connection.source);
      const targetNode = nodes.find((n) => n.id === connection.target);
      if (sourceNode && targetNode) {
        const sourcePort = sourceNode.data.outputs.find((p) => p.id === connection.sourceHandle);
        const targetPort = targetNode.data.inputs.find((p) => p.id === connection.targetHandle);
        if (sourcePort && targetPort) {
          const sourceIsAuto = sourcePort.dataType === 'auto';
          const targetIsAuto = targetPort.dataType === 'auto';
          const targetIsSampler = targetPort.dataType === 'sampler2D' || targetPort.dataType === 'samplerCube';
          const sourceIsSampler = sourcePort.dataType === 'sampler2D' || sourcePort.dataType === 'samplerCube';
          if (targetIsSampler) {
            // Reject auto→sampler and sampler→auto
            if (sourceIsAuto) return;
            const srcType = sourceNode.data.type;
            const srcIsTextureProducer = sourceIsSampler
              || srcType === 'shader' || srcType === 'constant'
              || (srcType === 'input' && sourceNode.data.inputDataType === 'sampler2D');
            if (!srcIsTextureProducer) return;
          } else if (sourceIsSampler && targetIsAuto) {
            // Reject sampler→auto
            return;
          } else if (sourceIsAuto || targetIsAuto) {
            // Allow any scalar/vector ↔ auto connection
          } else if (sourcePort.dataType !== targetPort.dataType) {
            return;
          }
        }

      }
      saveSnapshot();
      set((state) => {
        state.edges = addEdge({ ...connection, type: 'bezier' }, state.edges);
      });
    },

    addNode: (type: ShaderNodeData['type'], position?: { x: number; y: number }) => {
      saveSnapshot();
      const node = makeNode(type, position);
      set((state) => { state.nodes.push(node); });
    },

    addInputNode: (dataType: DataType, position?: { x: number; y: number }, inputMode?: InputMode) => {
      saveSnapshot();
      const node = makeNode('input', position, dataType, undefined, undefined, inputMode);
      set((state) => { state.nodes.push(node); });
    },

    addShaderNode: (code: string, label: string, position?: { x: number; y: number }) => {
      saveSnapshot();
      const node = makeNode('shader', position, undefined, code, label);
      if (SHADER_TEMPLATES.has(label)) {
        node.data.shaderTemplateId = label;
      }
      set((state) => { state.nodes.push(node); });
    },

    addSystemNode: (source: NonNullable<ShaderNodeData['systemSource']>, position?: { x: number; y: number }) => {
      saveSnapshot();
      const def = SYSTEM_SOURCES[source];
      const node = makeNode('input', position, def.dataType, def.code, def.label, 'system');
      node.data.inputMode = 'system';
      node.data.systemSource = source;
      set((state) => { state.nodes.push(node); });
    },

    addOnnxNode: (catalogId: string, position?: { x: number; y: number }) => {
      const entry = ONNX_CATALOG[catalogId];
      if (!entry) return;
      saveSnapshot();
      counters.node++;
      const id = `onnx_${counters.node}`;
      const cascade = counters.cascade++ * 28;
      const pos = position ?? { x: 100 + cascade, y: 100 + cascade };
      const onnxParams: Record<string, number | boolean> = {};
      let onnxScoreThreshold: number | undefined;
      let onnxIouThreshold: number | undefined;
      if (entry.defaultParams) {
        for (const [key, desc] of Object.entries(entry.defaultParams)) {
          onnxParams[key] = desc.default;
        }
        if ('scoreThreshold' in entry.defaultParams) {
          onnxScoreThreshold = entry.defaultParams.scoreThreshold.default as number;
        }
        if ('iouThreshold' in entry.defaultParams) {
          onnxIouThreshold = entry.defaultParams.iouThreshold.default as number;
        }
      }
      const node: Node<ShaderNodeData> = {
        id,
        type: 'onnx',
        position: pos,
        data: {
          type: 'onnx',
          label: `${entry.label.toLowerCase().replace(/\s+/g, '_')}_${counters.node}`,
          templateName: entry.label,
          shaderCode: '',
          inputs: entry.expectedIO.inputs.map((p) => ({ ...p, id: `${id}_${p.label}` })),
          outputs: entry.expectedIO.outputs.map((p) => ({ ...p, id: `${id}_${p.label}` })),
          uniforms: {},
          onnxModelId: catalogId,
          onnxSource: 'catalog',
          onnxCatalogId: catalogId,
          onnxStatus: 'not-downloaded',
          onnxParams: Object.keys(onnxParams).length > 0 ? onnxParams : undefined,
          onnxScoreThreshold,
          onnxIouThreshold,
        },
      };
      set((state) => { state.nodes.push(node); });
      // Fire-and-forget: download model, update node status/progress.
      void downloadCatalogModel(id, entry, set);
    },

    addCustomOnnxNode: (position?: { x: number; y: number }) => {
      saveSnapshot();
      counters.node++;
      const id = `onnx_${counters.node}`;
      const cascade = counters.cascade++ * 28;
      const pos = position ?? { x: 100 + cascade, y: 100 + cascade };
      const node: Node<ShaderNodeData> = {
        id,
        type: 'onnx',
        position: pos,
        data: {
          type: 'onnx',
          label: 'Custom ONNX',
          shaderCode: '',
          inputs: [],
          outputs: [],
          uniforms: {},
          onnxSource: 'custom',
          onnxStatus: undefined,
        },
      };
      set((state) => { state.nodes.push(node); });
    },

    loadCustomOnnxModel: (nodeId: string, buffer: ArrayBuffer, fileName: string) => {
      const modelId = `custom_${nodeId}`;
      set((state) => {
        const node = state.nodes.find((n) => n.id === nodeId);
        if (node) {
          node.data.onnxStatus = 'introspecting';
          node.data.onnxCustomFileName = fileName;
        }
      });
      void (async () => {
        try {
          // Introspect model → derive ports and task
          const meta = await introspectOnnxModel(buffer);
          const ports = metaToDefaultPorts(meta);
          // Prefix port IDs with node ID
          const inputs = ports.inputs.map((p) => ({ ...p, id: `${nodeId}_${p.label}` }));
          const outputs = ports.outputs.map((p) => ({ ...p, id: `${nodeId}_${p.label}` }));
          // Cache buffer so the execution engine can load it
          modelManager.cacheBuffer(modelId, buffer);
          set((state) => {
            const node = state.nodes.find((n) => n.id === nodeId);
            if (node) {
              node.data.label = fileName.replace(/\.onnx$/i, '');
              node.data.inputs = inputs;
              node.data.outputs = outputs;
              node.data.onnxModelId = modelId;
              node.data.onnxStatus = 'ready';
            }
          });
          // Probe backend
          const session = new OnnxInferenceSession();
          try {
            await session.loadFromBuffer(buffer);
            const backend = await session.probeBackend(3);
            set((state) => {
              const node = state.nodes.find((n) => n.id === nodeId);
              if (node) node.data.onnxBackend = backend;
            });
          } finally {
            session.dispose();
          }
        } catch (err) {
          set((state) => {
            const node = state.nodes.find((n) => n.id === nodeId);
            if (node) {
              node.data.onnxStatus = 'error';
              node.data.onnxError = err instanceof Error ? err.message : String(err);
            }
          });
        }
      })();
    },

    addMathNode: (mathOp: string, position?: { x: number; y: number }) => {
      saveSnapshot();
      counters.node++;
      const id = `math_${counters.node}`;
      const cascade = counters.cascade++ * 28;
      const pos = position ?? { x: 100 + cascade, y: 100 + cascade };
      const op = MATH_OPS[mathOp];
      if (!op) return;
      const ports = getMathPorts(op);
      const node: Node<ShaderNodeData> = {
        id,
        type: 'math',
        position: pos,
        data: {
          type: 'math',
          label: `${op.label.toLowerCase()}_${counters.node}`,
          templateName: op.label,
          shaderCode: '',
          inputs: ports.inputs.map((p) => ({ ...p, id: `${id}_${p.label}` })),
          outputs: ports.outputs.map((p) => ({ ...p, id: `${id}_${p.label}` })),
          uniforms: {},
          mathOp: mathOp,
        },
      };
      set((state) => { state.nodes.push(node); });
    },

    removeNode: (id: string) => {
      saveSnapshot();
      set((state) => {
        state.nodes = state.nodes.filter((n) => n.id !== id);
        state.edges = state.edges.filter((e) => e.source !== id && e.target !== id);
        if (state.selectedNodeId === id) state.selectedNodeId = null;
      });
    },

    removeSelectedElements: () => {
      const { nodes, edges } = get();
      const selectedNodeIds = nodes.filter((n) => n.selected).map((n) => n.id);
      const selectedEdgeIds = edges.filter((e) => e.selected).map((e) => e.id);
      if (selectedNodeIds.length === 0 && selectedEdgeIds.length === 0) return;
      saveSnapshot();
      set((state) => {
        state.nodes = state.nodes.filter((n) => !selectedNodeIds.includes(n.id));
        state.edges = state.edges.filter(
          (e) => !selectedEdgeIds.includes(e.id) && !selectedNodeIds.includes(e.source) && !selectedNodeIds.includes(e.target)
        );
        if (state.selectedNodeId && selectedNodeIds.includes(state.selectedNodeId)) {
          state.selectedNodeId = null;
        }
      });
    },

    updateNodeData: (id: string, data: Partial<ShaderNodeData>) => {
      if (data.shaderCode !== undefined) saveSnapshot();
      set((state) => {
        const node = state.nodes.find((n) => n.id === id);
        if (!node) return;
        if (data.shaderCode !== undefined) {
          const parsed = parseShader(data.shaderCode, node.data.inputs, node.data.outputs);
          node.data = { ...node.data, ...data, inputs: parsed.inputs, outputs: parsed.outputs };
        } else {
          Object.assign(node.data, data);
        }
      });
    },

    updateNodeInputType: (id: string, dataType: DataType) => {
      saveSnapshot();
      set((state) => {
        const node = state.nodes.find((n) => n.id === id);
        if (!node || node.data.type !== 'input') return;
        const shaderCode = createInputShader(dataType);
        const parsed = parseShader(shaderCode, node.data.inputs, node.data.outputs);
        node.data.shaderCode = shaderCode;
        node.data.inputDataType = dataType;
        node.data.inputs = parsed.inputs;
        node.data.outputs = parsed.outputs;
        node.data.uniforms = {};
      });
    },

    addRendererNode: (position?: { x: number; y: number }) => {
      saveSnapshot();
      counters.node++;
      const id = `renderer_${counters.node}`;
      const cascade = counters.cascade++ * 28;
      const pos = position ?? { x: 100 + cascade, y: 100 + cascade };
      const node: Node<ShaderNodeData> = {
        id,
        type: 'renderer',
        position: pos,
        data: {
          type: 'renderer',
          label: `renderer_${counters.node}`,
          shaderCode: '',
          inputs: [{ id: 'input_inputTexture', label: 'inputTexture', dataType: 'sampler2D', direction: 'input' }],
          outputs: [],
          uniforms: {},
          expanded: true,
        },
      };
      set((state) => { state.nodes.push(node); });
    },

    loadGraph: (nodes: Node<ShaderNodeData>[], edges: Edge[]) => {
      saveSnapshot();
      set((state) => {
        state.nodes = nodes;
        state.edges = edges.map((e) => ({ ...e, type: 'bezier' }));
        state.selectedNodeId = null;
      });
      syncCounters(nodes);
    },

    clearGraph: () => {
      saveSnapshot();
      set((state) => {
        state.nodes = [];
        state.edges = [];
        state.selectedNodeId = null;
        state.outputPreviews = {};
        state.outputData = {};
        state.savedFilePath = null;
        state.projectName = 'Untitled';
      });
      counters.node = 0;
      counters.cascade = 0;
    },
  };
}
