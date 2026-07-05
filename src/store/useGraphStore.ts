import { create } from 'zustand';
import { immer } from 'zustand/middleware/immer';
import type { Node, Edge, OnNodesChange, OnEdgesChange, OnConnect } from '@xyflow/react';
import {
  applyNodeChanges,
  applyEdgeChanges,
  addEdge,
} from '@xyflow/react';
import type { ShaderNodeData, DataType, Port } from '../types';
import { parseShader } from '../engine/shaderParser';

interface HistoryEntry {
  nodes: Node<ShaderNodeData>[];
  edges: Edge[];
}

interface GraphState {
  nodes: Node<ShaderNodeData>[];
  edges: Edge[];
  selectedNodeId: string | null;
  isRunning: boolean;
  projectName: string;
  outputPreviews: Record<string, string>;
  nodeErrors: Record<string, string>;

  undoStack: HistoryEntry[];
  redoStack: HistoryEntry[];

  pushHistory: () => void;
  undo: () => void;
  redo: () => void;
  onNodesChange: OnNodesChange;
  onEdgesChange: OnEdgesChange;
  onConnect: OnConnect;
  addNode: (type: ShaderNodeData['type'], position?: { x: number; y: number }) => void;
  addInputNode: (dataType: DataType, position?: { x: number; y: number }) => void;
  addShaderNode: (code: string, label: string, position?: { x: number; y: number }) => void;
  removeNode: (id: string) => void;
  removeSelectedElements: () => void;
  updateNodeData: (id: string, data: Partial<ShaderNodeData>) => void;
  updateNodeInputType: (id: string, dataType: DataType) => void;
  setSelectedNode: (id: string | null) => void;
  setRunning: (running: boolean) => void;
  setProjectName: (name: string) => void;
  setOutputPreview: (nodeId: string, dataUrl: string) => void;
  clearOutputPreviews: () => void;
  setNodeError: (nodeId: string, error: string | null) => void;
  clearNodeErrors: () => void;
  loadGraph: (nodes: Node<ShaderNodeData>[], edges: Edge[]) => void;
  clearGraph: () => void;
}

let nodeCounter = 0;

function createInputShader(dataType: DataType): string {
  switch (dataType) {
    case 'float':   return `uniform float value;\nout float outputValue;\nvoid main() { outputValue = value; }`;
    case 'int':     return `uniform int value;\nout int outputValue;\nvoid main() { outputValue = value; }`;
    case 'bool':    return `uniform bool value;\nout bool outputValue;\nvoid main() { outputValue = value; }`;
    case 'vec2':    return `uniform vec2 value;\nout vec2 outputValue;\nvoid main() { outputValue = value; }`;
    case 'vec3':    return `uniform vec3 value;\nout vec3 outputValue;\nvoid main() { outputValue = value; }`;
    case 'vec4':    return `uniform vec4 value;\nout vec4 outputValue;\nvoid main() { outputValue = value; }`;
    case 'sampler2D': return `uniform sampler2D value;\nout vec4 outputValue;\nvoid main() { outputValue = texture(value, v_uv); }`;
    default:        return `uniform float value;\nout float outputValue;\nvoid main() { outputValue = value; }`;
  }
}

function createDefaultShaderCode(type: ShaderNodeData['type'], inputDataType?: DataType): string {
  switch (type) {
    case 'shader':
      return [
        'uniform sampler2D inputImage;',
        'uniform float intensity;',
        '',
        'out vec4 fragColor;',
        '',
        'void main() {',
        '  vec4 color = texture(inputImage, v_uv);',
        '  color.rgb *= intensity;',
        '  fragColor = color;',
        '}',
      ].join('\n');
    case 'input':
      return createInputShader(inputDataType ?? 'float');
    case 'output':
      return [
        'uniform sampler2D inputImage;',
        'out vec4 fragColor;',
        'void main() { fragColor = texture(inputImage, v_uv); }',
      ].join('\n');
    case 'constant':
      return 'uniform vec4 color;\nout vec4 fragColor;\nvoid main() { fragColor = color; }';
  }
}

let nodeCascade = 0;
const ID_RE = /^(.+?)_(\d+)$/;

function syncCounters(nodes: Node<ShaderNodeData>[]) {
  let maxNum = 0;
  let maxCascade = 0;
  for (const n of nodes) {
    const m = ID_RE.exec(n.id);
    if (m) {
      const num = parseInt(m[2], 10);
      if (num > maxNum) maxNum = num;
    }
    const cx = Math.floor((n.position.x - 100) / 28);
    const cy = Math.floor((n.position.y - 100) / 28);
    if (cx > maxCascade) maxCascade = cx;
    if (cy > maxCascade) maxCascade = cy;
  }
  nodeCounter = maxNum;
  nodeCascade = maxCascade + 1;
}

function makeNode(type: ShaderNodeData['type'], position?: { x: number; y: number }, inputDataType?: DataType, shaderCodeOverride?: string, labelOverride?: string): Node<ShaderNodeData> {
  nodeCounter++;
  const id = `${type}_${nodeCounter}`;
  const shaderCode = shaderCodeOverride ?? createDefaultShaderCode(type, inputDataType);
  const parsed = parseShader(shaderCode);

  const cascade = nodeCascade++ * 28;
  const pos = position ?? { x: 100 + cascade, y: 100 + cascade };

  return {
    id,
    type,
    position: pos,
    data: {
      type,
      label: labelOverride ?? `${type}_${nodeCounter}`,
      shaderCode,
      inputs: parsed.inputs,
      outputs: parsed.outputs,
      uniforms: {},
      inputDataType: type === 'input' ? (inputDataType ?? 'float') : undefined,
    },
  };
}

function remapEdgePorts(
  edges: Edge[],
  nodeId: string,
  oldInputs: Port[],
  oldOutputs: Port[],
  newInputs: Port[],
  newOutputs: Port[],
): void {
  for (let i = 0; i < edges.length; i++) {
    const e = edges[i];
    if (e.target === nodeId) {
      const old = oldInputs.find((p) => p.id === e.targetHandle);
      if (old) {
        const match = newInputs.find((p) => p.label === old.label);
        if (match) edges[i] = { ...e, targetHandle: match.id };
      }
    }
    if (e.source === nodeId) {
      const old = oldOutputs.find((p) => p.id === e.sourceHandle);
      if (old) {
        const match = newOutputs.find((p) => p.label === old.label);
        if (match) edges[i] = { ...e, sourceHandle: match.id };
      }
    }
  }
}

export const useGraphStore = create<GraphState>()(
  immer((set, get) => {
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
      nodes: [],
      edges: [],
      selectedNodeId: null,
      isRunning: false,
      projectName: 'Untitled',
      outputPreviews: {},
      nodeErrors: {},
      undoStack: [],
      redoStack: [],

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

      onNodesChange: (changes) => {
        set((state) => {
          state.nodes = applyNodeChanges(changes, state.nodes) as unknown as Node<ShaderNodeData>[];
        });
      },

      onEdgesChange: (changes) => {
        set((state) => {
          state.edges = applyEdgeChanges(changes, state.edges);
        });
      },

      onConnect: (connection) => {
        saveSnapshot();
        set((state) => {
          state.edges = addEdge({ ...connection, type: 'bezier' }, state.edges);
        });
      },

      addNode: (type, position) => {
        saveSnapshot();
        const node = makeNode(type, position);
        set((state) => { state.nodes.push(node); });
      },

      addInputNode: (dataType, position) => {
        saveSnapshot();
        const node = makeNode('input', position, dataType);
        set((state) => { state.nodes.push(node); });
      },

      addShaderNode: (code, label, position) => {
        saveSnapshot();
        const node = makeNode('shader', position, undefined, code, label);
        set((state) => { state.nodes.push(node); });
      },

      removeNode: (id) => {
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

      updateNodeData: (id, data) => {
        if (data.shaderCode !== undefined) saveSnapshot();
        set((state) => {
          const node = state.nodes.find((n) => n.id === id);
          if (!node) return;
          if (data.shaderCode !== undefined) {
            const oldInputs = node.data.inputs;
            const oldOutputs = node.data.outputs;
            const parsed = parseShader(data.shaderCode);
            node.data = { ...node.data, ...data, inputs: parsed.inputs, outputs: parsed.outputs };
            remapEdgePorts(state.edges, id, oldInputs, oldOutputs, parsed.inputs, parsed.outputs);
          } else {
            Object.assign(node.data, data);
          }
        });
      },

      updateNodeInputType: (id, dataType) => {
        saveSnapshot();
        set((state) => {
          const node = state.nodes.find((n) => n.id === id);
          if (!node || node.data.type !== 'input') return;
          const oldInputs = node.data.inputs;
          const oldOutputs = node.data.outputs;
          const shaderCode = createInputShader(dataType);
          const parsed = parseShader(shaderCode);
          node.data.shaderCode = shaderCode;
          node.data.inputDataType = dataType;
          node.data.inputs = parsed.inputs;
          node.data.outputs = parsed.outputs;
          node.data.uniforms = {};
          remapEdgePorts(state.edges, id, oldInputs, oldOutputs, parsed.inputs, parsed.outputs);
        });
      },

      setSelectedNode: (id) => {
        set((state) => { state.selectedNodeId = id; });
      },

      setRunning: (running) => {
        set((state) => { state.isRunning = running; });
      },

      setProjectName: (name) => {
        set((state) => { state.projectName = name; });
      },

      setOutputPreview: (nodeId, dataUrl) => {
        set((state) => { state.outputPreviews[nodeId] = dataUrl; });
      },

      clearOutputPreviews: () => {
        set((state) => { state.outputPreviews = {}; });
      },

      setNodeError: (nodeId, error) => {
        set((state) => {
          if (error === null) {
            delete state.nodeErrors[nodeId];
          } else {
            state.nodeErrors[nodeId] = error;
          }
        });
      },

      clearNodeErrors: () => {
        set((state) => { state.nodeErrors = {}; });
      },

      loadGraph: (nodes, edges) => {
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
        });
        nodeCounter = 0;
        nodeCascade = 0;
      },
    };
  }),
);
