import type { Node, Edge } from '@xyflow/react';
import type { ShaderNodeData, DataType, InputMode } from '../types';
import { parseShader } from '../engine/shaderParser';
import type { CatalogEntry } from '../engine/onnxCatalog';
import { OnnxModelManager } from '../engine/onnxModelManager';
import { OnnxInferenceSession } from '../engine/onnxInference';
import type { GraphState } from './index';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface HistoryEntry {
  nodes: Node<ShaderNodeData>[];
  edges: Edge[];
}

// ---------------------------------------------------------------------------
// Singleton model manager — shared across the app lifetime.
// ---------------------------------------------------------------------------

export const modelManager = new OnnxModelManager();

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

export const SYSTEM_SOURCES: Record<
  NonNullable<ShaderNodeData['systemSource']>,
  { label: string; dataType: DataType; code: string }
> = {
  time: { label: 'Time', dataType: 'float', code: 'uniform float value;\nout float outputValue;\nvoid main() { outputValue = value; }' },
  timeDelta: { label: 'Time Delta', dataType: 'float', code: 'uniform float value;\nout float outputValue;\nvoid main() { outputValue = value; }' },
  frame: { label: 'Frame', dataType: 'int', code: 'uniform int value;\nout int outputValue;\nvoid main() { outputValue = value; }' },
  mouse: { label: 'Mouse', dataType: 'vec4', code: 'uniform vec4 value;\nout vec4 outputValue;\nvoid main() { outputValue = value; }' },
  resolution: { label: 'Resolution', dataType: 'vec3', code: 'uniform vec3 value;\nout vec3 outputValue;\nvoid main() { outputValue = value; }' },
};

export const ID_RE = /^(.+?)_(\d+)$/;

// ---------------------------------------------------------------------------
// Mutable counters — shared across slices via this module-level object.
// ---------------------------------------------------------------------------

export const counters = { node: 0, cascade: 0 };

// ---------------------------------------------------------------------------
// Shader helpers
// ---------------------------------------------------------------------------

export function createInputShader(dataType: DataType): string {
  if (dataType === 'sampler2D') {
    return `uniform sampler2D value;\nout vec4 outputValue;\nvoid main() { outputValue = texture(value, v_uv); }`;
  }
  return `uniform ${dataType} value;\nout ${dataType} outputValue;\nvoid main() { outputValue = value; }`;
}

export function createDefaultShaderCode(type: ShaderNodeData['type'], inputDataType?: DataType): string {
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

// ---------------------------------------------------------------------------
// Node counters & factory
// ---------------------------------------------------------------------------

export function syncCounters(nodes: Node<ShaderNodeData>[]) {
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
  counters.node = maxNum;
  counters.cascade = maxCascade + 1;
}

export function makeNode(
  type: ShaderNodeData['type'],
  position?: { x: number; y: number },
  inputDataType?: DataType,
  shaderCodeOverride?: string,
  labelOverride?: string,
  inputMode?: InputMode,
): Node<ShaderNodeData> {
  counters.node++;
  const id = `${type}_${counters.node}`;
  const shaderCode = shaderCodeOverride ?? createDefaultShaderCode(type, inputDataType);
  const parsed = parseShader(shaderCode);

  const cascade = counters.cascade++ * 28;
  const pos = position ?? { x: 100 + cascade, y: 100 + cascade };

  const baseName = labelOverride ?? type;
  const instanceLabel = `${baseName.toLowerCase().replace(/\s+/g, '_')}_${counters.node}`;

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

export async function probeModelBackend(
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

// ---------------------------------------------------------------------------
// ONNX catalog download helper
// ---------------------------------------------------------------------------

/**
 * Async helper: download a catalog model, updating node status/progress in the store.
 * Called fire-and-forget from addOnnxNode.
 */
export async function downloadCatalogModel(
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
