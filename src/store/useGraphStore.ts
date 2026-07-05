import { create } from 'zustand';
import { immer } from 'zustand/middleware/immer';
import type { Node, Edge, OnNodesChange, OnEdgesChange, OnConnect } from '@xyflow/react';
import {
  applyNodeChanges,
  applyEdgeChanges,
  addEdge,
} from '@xyflow/react';
import type { ShaderNodeData, DataType } from '../types';
import { parseShader } from '../engine/shaderParser';

interface GraphState {
  nodes: Node<ShaderNodeData>[];
  edges: Edge[];
  selectedNodeId: string | null;
  isRunning: boolean;
  projectName: string;
  outputPreviews: Record<string, string>;

  onNodesChange: OnNodesChange;
  onEdgesChange: OnEdgesChange;
  onConnect: OnConnect;
  addNode: (type: ShaderNodeData['type'], position?: { x: number; y: number }) => void;
  addInputNode: (dataType: DataType, position?: { x: number; y: number }) => void;
  removeNode: (id: string) => void;
  updateNodeData: (id: string, data: Partial<ShaderNodeData>) => void;
  updateNodeInputType: (id: string, dataType: DataType) => void;
  setSelectedNode: (id: string | null) => void;
  setRunning: (running: boolean) => void;
  setProjectName: (name: string) => void;
  setOutputPreview: (nodeId: string, dataUrl: string) => void;
  clearOutputPreviews: () => void;
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

function makeNode(type: ShaderNodeData['type'], position?: { x: number; y: number }, inputDataType?: DataType): Node<ShaderNodeData> {
  nodeCounter++;
  const id = `${type}_${nodeCounter}`;
  const shaderCode = createDefaultShaderCode(type, inputDataType);
  const parsed = parseShader(shaderCode);

  return {
    id,
    type,
    position: position ?? { x: 100 + Math.random() * 300, y: 100 + Math.random() * 300 },
    data: {
      type,
      label: `${type}_${nodeCounter}`,
      shaderCode,
      inputs: parsed.inputs,
      outputs: parsed.outputs,
      uniforms: {},
      inputDataType: type === 'input' ? (inputDataType ?? 'float') : undefined,
    },
  };
}

export const useGraphStore = create<GraphState>()(
  immer((set) => ({
    nodes: [],
    edges: [],
    selectedNodeId: null,
    isRunning: false,
    projectName: 'Untitled',
    outputPreviews: {},

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
      set((state) => {
        state.edges = addEdge({ ...connection, type: 'bezier' }, state.edges);
      });
    },

    addNode: (type, position) => {
      const node = makeNode(type, position);
      set((state) => { state.nodes.push(node); });
    },

    addInputNode: (dataType, position) => {
      const node = makeNode('input', position, dataType);
      set((state) => { state.nodes.push(node); });
    },

    removeNode: (id) => {
      set((state) => {
        state.nodes = state.nodes.filter((n) => n.id !== id);
        state.edges = state.edges.filter((e) => e.source !== id && e.target !== id);
        if (state.selectedNodeId === id) state.selectedNodeId = null;
      });
    },

    updateNodeData: (id, data) => {
      set((state) => {
        const node = state.nodes.find((n) => n.id === id);
        if (!node) return;
        if (data.shaderCode !== undefined) {
          const parsed = parseShader(data.shaderCode);
          node.data = { ...node.data, ...data, inputs: parsed.inputs, outputs: parsed.outputs };
        } else {
          Object.assign(node.data, data);
        }
      });
    },

    updateNodeInputType: (id, dataType) => {
      set((state) => {
        const node = state.nodes.find((n) => n.id === id);
        if (!node || node.data.type !== 'input') return;
        const shaderCode = createInputShader(dataType);
        const parsed = parseShader(shaderCode);
        node.data.shaderCode = shaderCode;
        node.data.inputDataType = dataType;
        node.data.inputs = parsed.inputs;
        node.data.outputs = parsed.outputs;
        node.data.uniforms = {};
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

    loadGraph: (nodes, edges) => {
      set((state) => {
        state.nodes = nodes;
        state.edges = edges.map((e) => ({ ...e, type: 'bezier' }));
        state.selectedNodeId = null;
      });
    },

    clearGraph: () => {
      set((state) => {
        state.nodes = [];
        state.edges = [];
        state.selectedNodeId = null;
        state.outputPreviews = {};
      });
    },
  })),
);
