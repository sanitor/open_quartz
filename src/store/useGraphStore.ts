import { create } from 'zustand';
import { immer } from 'zustand/middleware/immer';
import type { Node, Edge, OnNodesChange, OnEdgesChange, OnConnect } from '@xyflow/react';
import {
  applyNodeChanges,
  applyEdgeChanges,
  addEdge,
} from '@xyflow/react';
import type { ShaderNodeData, DataType, InputMode } from '../types';
import { parseShader } from '../engine/shaderParser';
import { MATH_OPS, getMathPorts } from '../engine/mathOps';
import { ONNX_CATALOG } from '../engine/onnxCatalog';
import type { CatalogEntry } from '../engine/onnxCatalog';
import { OnnxModelManager } from '../engine/onnxModelManager';
import { OnnxInferenceSession } from '../engine/onnxInference';
import { introspectOnnxModel, metaToDefaultPorts } from '../engine/onnxIntrospect';

// Singleton model manager — shared across the app lifetime.
export const modelManager = new OnnxModelManager();

interface HistoryEntry {
  nodes: Node<ShaderNodeData>[];
  edges: Edge[];
}

interface GraphState {
  nodes: Node<ShaderNodeData>[];
  edges: Edge[];
  selectedNodeId: string | null;
  projectName: string;
  savedFilePath: string | null;
  outputPreviews: Record<string, string>;
  outputData: Record<string, unknown>;
  nodeErrors: Record<string, string>;
  loopState: 'stopped' | 'playing' | 'paused';
  fps: number;
  currentTime: number;
  currentFrame: number;
  activeRendererId: string | null;

  undoStack: HistoryEntry[];
  redoStack: HistoryEntry[];

  pushHistory: () => void;
  undo: () => void;
  redo: () => void;
  onNodesChange: OnNodesChange;
  onEdgesChange: OnEdgesChange;
  onConnect: OnConnect;
  addNode: (type: ShaderNodeData['type'], position?: { x: number; y: number }) => void;
  addInputNode: (dataType: DataType, position?: { x: number; y: number }, inputMode?: InputMode) => void;
  addSystemNode: (source: NonNullable<ShaderNodeData['systemSource']>, position?: { x: number; y: number }) => void;
  addShaderNode: (code: string, label: string, position?: { x: number; y: number }) => void;
  addOnnxNode: (catalogId: string, position?: { x: number; y: number }) => void;
  addCustomOnnxNode: (position?: { x: number; y: number }) => void;
  loadCustomOnnxModel: (nodeId: string, buffer: ArrayBuffer, fileName: string) => void;
  addMathNode: (mathOp: string, position?: { x: number; y: number }) => void;
  removeNode: (id: string) => void;
  removeSelectedElements: () => void;
  updateNodeData: (id: string, data: Partial<ShaderNodeData>) => void;
  updateNodeInputType: (id: string, dataType: DataType) => void;
  setSelectedNode: (id: string | null) => void;
  setProjectName: (name: string) => void;
  setSavedFilePath: (path: string | null) => void;
  setOutputPreview: (nodeId: string, dataUrl: string) => void;
  setOutputData: (nodeId: string, data: unknown) => void;
  clearOutputPreviews: () => void;
  play: () => void;
  pause: () => void;
  resume: () => void;
  setFps: (fps: number) => void;
  setCurrentTime: (t: number) => void;
  setCurrentFrame: (frame: number) => void;
  setActiveRenderer: (id: string | null) => void;
  addRendererNode: (position?: { x: number; y: number }) => void;
  stop: () => void;
  setNodeError: (nodeId: string, error: string | null) => void;
  clearNodeErrors: () => void;
  loadGraph: (nodes: Node<ShaderNodeData>[], edges: Edge[]) => void;
  clearGraph: () => void;
  captureScreenshot: ((rendererId: string) => string | null) | null;
  setCaptureScreenshot: (fn: ((rendererId: string) => string | null) | null) => void;
}
const SYSTEM_SOURCES: Record<NonNullable<ShaderNodeData['systemSource']>, { label: string; dataType: DataType; code: string }> = {
  time: { label: 'Time', dataType: 'float', code: 'uniform float value;\nout float outputValue;\nvoid main() { outputValue = value; }' },
  timeDelta: { label: 'Time Delta', dataType: 'float', code: 'uniform float value;\nout float outputValue;\nvoid main() { outputValue = value; }' },
  frame: { label: 'Frame', dataType: 'int', code: 'uniform int value;\nout int outputValue;\nvoid main() { outputValue = value; }' },
  mouse: { label: 'Mouse', dataType: 'vec4', code: 'uniform vec4 value;\nout vec4 outputValue;\nvoid main() { outputValue = value; }' },
  resolution: { label: 'Resolution', dataType: 'vec3', code: 'uniform vec3 value;\nout vec3 outputValue;\nvoid main() { outputValue = value; }' },
};

let nodeCounter = 0;

function createInputShader(dataType: DataType): string {
  if (dataType === 'sampler2D') {
    return `uniform sampler2D value;\nout vec4 outputValue;\nvoid main() { outputValue = texture(value, v_uv); }`;
  }
  return `uniform ${dataType} value;\nout ${dataType} outputValue;\nvoid main() { outputValue = value; }`;
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
    case 'constant':
      return 'uniform vec4 color;\nout vec4 fragColor;\nvoid main() { fragColor = color; }';
    case 'onnx':
    case 'renderer':
    case 'math':
      return '';
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

function makeNode(type: ShaderNodeData['type'], position?: { x: number; y: number }, inputDataType?: DataType, shaderCodeOverride?: string, labelOverride?: string, inputMode?: InputMode): Node<ShaderNodeData> {
  nodeCounter++;
  const id = `${type}_${nodeCounter}`;
  const shaderCode = shaderCodeOverride ?? createDefaultShaderCode(type, inputDataType);
  const parsed = parseShader(shaderCode);

  const cascade = nodeCascade++ * 28;
  const pos = position ?? { x: 100 + cascade, y: 100 + cascade };

  const baseName = labelOverride ?? type;
  const instanceLabel = `${baseName.toLowerCase().replace(/\s+/g, '_')}_${nodeCounter}`;

  return {
    id,
    type,
    position: pos,
    data: {
      type,
      label: instanceLabel,
      templateName: labelOverride,
      shaderCode,
      inputs: parsed.inputs,
      outputs: parsed.outputs,
      uniforms: {},
      inputDataType: type === 'input' ? (inputDataType ?? 'float') : undefined,
      inputMode: type === 'input' ? (inputMode ?? (inputDataType === 'sampler2D' ? 'image' : undefined)) : undefined,
    },
  };
}

/**
 * Async helper: download a catalog model, updating node status/progress in the store.
 * Called fire-and-forget from addOnnxNode.
 */
async function downloadCatalogModel(
  nodeId: string,
  entry: CatalogEntry,
  set: (fn: (state: GraphState) => void) => void,
): Promise<void> {
  // Check if already cached.
  const cached = await modelManager.loadCachedModel(entry.id);
  if (cached) {
    set((state) => {
      const node = state.nodes.find((n) => n.id === nodeId);
      if (node) {
        node.data.onnxStatus = 'ready';
        node.data.onnxProgress = 1;
      }
    });
    // Probe backend for cached models too (result may be in localStorage).
    void probeModelBackend(nodeId, entry, cached, set);
    return;
  }

  // Start downloading — subscribe to progress.
  set((state) => {
    const node = state.nodes.find((n) => n.id === nodeId);
    if (node) node.data.onnxStatus = 'downloading';
  });

  const unsub = modelManager.subscribe(() => {
    const ms = modelManager.getState(entry.id);
    set((state) => {
      const node = state.nodes.find((n) => n.id === nodeId);
      if (!node) return;
      node.data.onnxProgress = ms.progress;
      if (ms.status === 'error') {
        node.data.onnxStatus = 'error';
        node.data.onnxError = ms.error;
      }
    });
  });

  try {
    const buffer = await modelManager.downloadModel(entry);
    set((state) => {
      const node = state.nodes.find((n) => n.id === nodeId);
      if (node) {
        node.data.onnxStatus = 'ready';
        node.data.onnxProgress = 1;
      }
    });
    // Probe WebGPU compatibility immediately after download.
    void probeModelBackend(nodeId, entry, buffer, set);
  } catch (err) {
    set((state) => {
      const node = state.nodes.find((n) => n.id === nodeId);
      if (node) {
        node.data.onnxStatus = 'error';
        node.data.onnxError = err instanceof Error ? err.message : String(err);
      }
    });
  } finally {
    unsub();
  }
}

// ---------------------------------------------------------------------------
// WebGPU probe — detect backend compatibility before user presses Play
// ---------------------------------------------------------------------------

/** Cache key: modelId + GPU vendor → 'webgpu' | 'wasm'. */
function probeStorageKey(modelId: string, vendor: string): string {
  return `oq:probe:${modelId}:${vendor}`;
}

async function getGpuVendor(): Promise<string> {
  try {
    if (!navigator.gpu) return 'unknown';
    const adapter = await navigator.gpu.requestAdapter();
    return adapter?.info?.vendor ?? 'unknown';
  } catch {
    return 'unknown';
  }
}

/** Detection models (YOLO) run via the Rust wasm path — skip TS ORT probe. */
const SKIP_PROBE_TASKS = new Set(['detection']);

async function probeModelBackend(
  nodeId: string,
  entry: CatalogEntry,
  buffer: ArrayBuffer,
  set: (fn: (state: GraphState) => void) => void,
): Promise<void> {
  if (SKIP_PROBE_TASKS.has(entry.task)) return;

  const vendor = await getGpuVendor();
  const key = probeStorageKey(entry.id, vendor);

  // Check localStorage cache first.
  try {
    const cached = localStorage.getItem(key);
    if (cached === 'webgpu' || cached === 'wasm') {
      set((state) => {
        const node = state.nodes.find((n) => n.id === nodeId);
        if (node) node.data.onnxBackend = cached;
      });
      return;
    }
  } catch { /* localStorage may be unavailable */ }

  // Run probe.
  const session = new OnnxInferenceSession();
  try {
    await session.loadFromBuffer(buffer);
    const inputChannels = entry.task === 'super-resolution' || entry.task === 'background-removal' ? 3 : 3;
    const backend = await session.probeBackend(inputChannels);

    // Cache result.
    try { localStorage.setItem(key, backend); } catch { /* ignore */ }

    set((state) => {
      const node = state.nodes.find((n) => n.id === nodeId);
      if (node) node.data.onnxBackend = backend;
    });
  } catch {
    // Probe itself failed (e.g. ORT load error) — don't block, leave backend unset.
  } finally {
    session.dispose();
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
      projectName: 'Untitled',
      savedFilePath: null,
      outputPreviews: {},
      outputData: {},
      nodeErrors: {},
      undoStack: [],
      redoStack: [],
      loopState: 'stopped',
      fps: 0,
      currentTime: 0,
      currentFrame: 0,
      activeRendererId: null,
      captureScreenshot: null,

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

      addNode: (type, position) => {
        saveSnapshot();
        const node = makeNode(type, position);
        set((state) => { state.nodes.push(node); });
      },

      addInputNode: (dataType, position, inputMode) => {
        saveSnapshot();
        const node = makeNode('input', position, dataType, undefined, undefined, inputMode);
        set((state) => { state.nodes.push(node); });
      },

      addShaderNode: (code, label, position) => {
        saveSnapshot();
        const node = makeNode('shader', position, undefined, code, label);
        set((state) => { state.nodes.push(node); });
      },


      addSystemNode: (source, position) => {
        saveSnapshot();
        const def = SYSTEM_SOURCES[source];
        const node = makeNode('input', position, def.dataType, def.code, def.label, 'system');
        node.data.inputMode = 'system';
        node.data.systemSource = source;
        set((state) => { state.nodes.push(node); });
      },
      addOnnxNode: (catalogId, position) => {
        const entry = ONNX_CATALOG[catalogId];
        if (!entry) return;
        saveSnapshot();
        nodeCounter++;
        const id = `onnx_${nodeCounter}`;
        const cascade = nodeCascade++ * 28;
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
            label: `${entry.label.toLowerCase().replace(/\s+/g, '_')}_${nodeCounter}`,
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
      addCustomOnnxNode: (position) => {
        saveSnapshot();
        nodeCounter++;
        const id = `onnx_${nodeCounter}`;
        const cascade = nodeCascade++ * 28;
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
      loadCustomOnnxModel: (nodeId, buffer, fileName) => {
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
      addMathNode: (mathOp, position) => {
        saveSnapshot();
        nodeCounter++;
        const id = `math_${nodeCounter}`;
        const cascade = nodeCascade++ * 28;
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
            label: `${op.label.toLowerCase()}_${nodeCounter}`,
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
            const parsed = parseShader(data.shaderCode, node.data.inputs, node.data.outputs);
            node.data = { ...node.data, ...data, inputs: parsed.inputs, outputs: parsed.outputs };
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
          const shaderCode = createInputShader(dataType);
          const parsed = parseShader(shaderCode, node.data.inputs, node.data.outputs);
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


      setProjectName: (name) => {
        set((state) => { state.projectName = name; });
      },

      setSavedFilePath: (path) => {
        set((state) => { state.savedFilePath = path; });
      },

      setOutputPreview: (nodeId, dataUrl) => {
        set((state) => { state.outputPreviews[nodeId] = dataUrl; });
      },

      setOutputData: (nodeId, data) => {
        set((state) => { state.outputData[nodeId] = data; });
      },

      clearOutputPreviews: () => {
        set((state) => { state.outputPreviews = {}; state.outputData = {}; });
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
          state.outputData = {};
          state.savedFilePath = null;
          state.projectName = 'Untitled';
        });
        nodeCounter = 0;
        nodeCascade = 0;
      },

      play: () => set((state) => { state.loopState = 'playing'; }),
      pause: () => set((state) => { state.loopState = 'paused'; }),
      resume: () => set((state) => { state.loopState = 'playing'; }),
      setFps: (fps) => set((state) => { state.fps = fps; }),
      setCurrentTime: (t) => set((state) => { state.currentTime = t; }),
      setCurrentFrame: (f) => set((state) => { state.currentFrame = f; }),
      setActiveRenderer: (id) => set((state) => { state.activeRendererId = id; }),
      addRendererNode: (position) => {
        saveSnapshot();
        nodeCounter++;
        const id = `renderer_${nodeCounter}`;
        const cascade = nodeCascade++ * 28;
        const pos = position ?? { x: 100 + cascade, y: 100 + cascade };
        const node: Node<ShaderNodeData> = {
          id,
          type: 'renderer',
          position: pos,
          data: {
            type: 'renderer',
            label: `renderer_${nodeCounter}`,
            shaderCode: '',
            inputs: [{ id: 'input_inputTexture', label: 'inputTexture', dataType: 'sampler2D', direction: 'input' }],
            outputs: [],
            uniforms: {},
            expanded: true,
          },
        };
        set((state) => { state.nodes.push(node); });
      },

      stop: () => set((state) => {
        state.loopState = 'stopped';
        state.fps = 0;
        state.currentTime = 0;
        state.currentFrame = 0;
      }),
      setCaptureScreenshot: (fn) => set((state) => { state.captureScreenshot = fn as never; }),
    };
  }),
);
