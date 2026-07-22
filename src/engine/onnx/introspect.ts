// onnxIntrospect — extract metadata from an ONNX model buffer via onnxruntime-web.
//
// `ort` is loaded at runtime as `globalThis.ort` from a <script> tag,
// so we import only the *types* from the package for compile-time safety.

import type * as OrtModule from 'onnxruntime-web';
import type { Port } from '../../types';
import type { OnnxTask } from '../../catalog/onnxCatalog';
import { ensureOrtLoaded } from './inference';

declare const ort: typeof OrtModule;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface OnnxModelMeta {
  opset: number;
  inputs: Array<{ name: string; shape: (number | string)[]; dtype: string }>;
  outputs: Array<{ name: string; shape: (number | string)[]; dtype: string }>;
  inferredTask?: OnnxTask;
  inferredScale?: number;
  inferredInputChannels?: number;
  inferredOutputChannels?: number;
}

// ---------------------------------------------------------------------------
// Introspection
// ---------------------------------------------------------------------------

/**
 * Create a temporary InferenceSession from a raw ONNX buffer and extract
 * whatever metadata the public ORT-web API exposes.
 *
 * Limitations (MVP):
 *   - onnxruntime-web's JS API does not surface per-tensor shape/dtype
 *     metadata without running inference.  We record input/output *names*
 *     and leave shape arrays empty until a full protobuf-based parser is
 *     added.
 *   - `opset` is not exposed by InferenceSession; we default to 0.
 *
 * TODO: Add onnx-proto (protobuf) introspection for full shape/dtype info.
 */
export async function introspectOnnxModel(
  buffer: ArrayBuffer,
): Promise<OnnxModelMeta> {
  await ensureOrtLoaded();
  const session = await ort.InferenceSession.create(buffer);

  const inMeta = session.inputMetadata ?? [];
  const outMeta = session.outputMetadata ?? [];

  const inputs: OnnxModelMeta['inputs'] = session.inputNames.map((name, i) => {
    const m = inMeta[i];
    const shape: (number | string)[] = (m && 'shape' in m) ? [...m.shape] : [];
    const dtype = (m && 'type' in m) ? String(m.type) : 'float32';
    return { name, shape, dtype };
  });

  const outputs: OnnxModelMeta['outputs'] = session.outputNames.map((name, i) => {
    const m = outMeta[i];
    const shape: (number | string)[] = (m && 'shape' in m) ? [...m.shape] : [];
    const dtype = (m && 'type' in m) ? String(m.type) : 'float32';
    return { name, shape, dtype };
  });

  // Best-effort: release the session to free wasm memory.
  try {
    await session.release();
  } catch {
    // release() may throw in some ORT-web builds; not critical.
  }

  const meta: OnnxModelMeta = { opset: 0, inputs, outputs };
  meta.inferredTask = inferTaskFromMeta(meta);

  return meta;
}

// ---------------------------------------------------------------------------
// Task inference (heuristic, intentionally simple for MVP)
// ---------------------------------------------------------------------------

/**
 * Guess the {@link OnnxTask} from the model's I/O signature.
 *
 * Detection models (YOLO, SSD, etc.) typically emit a single output tensor
 * whose last dimension encodes `[x, y, w, h, score, ...classes]`, giving a
 * column count ≥ 5.  Super-resolution and style-transfer models usually have
 * a single image-shaped output with 3 or 4 channels.
 *
 * Because we lack shape info in the MVP we fall back to output-count
 * heuristics: detection models often have multiple outputs (boxes, scores,
 * classes).
 */
export function inferTaskFromMeta(meta: OnnxModelMeta): OnnxTask {
  const { inputs, outputs } = meta;

  // --- shape-based heuristics (when available) ---------------------------

  const outShape = outputs[0]?.shape;
  if (outShape && outShape.length > 0) {
    const lastDim = outShape[outShape.length - 1];
    if (typeof lastDim === 'number') {
      // Detection: last dim ≥ 5 (bbox + score + classes), but only for
      // 2D/3D outputs. 4D outputs are image tensors (NCHW/NHWC).
      if (lastDim >= 5 && outputs.length === 1 && outShape.length <= 3) return 'detection';
    }

    // Super-resolution: single input, single output, output spatial dims
    // larger than input spatial dims.
    if (inputs.length === 1 && outputs.length === 1) {
      const inShape = inputs[0]?.shape;
      if (inShape && inShape.length === 4 && outShape.length === 4) {
        const inH = inShape[2];
        const inW = inShape[3];
        const outH = outShape[2];
        const outW = outShape[3];
        if (
          typeof inH === 'number' && typeof inW === 'number' &&
          typeof outH === 'number' && typeof outW === 'number' &&
          outH > inH && outW > inW
        ) {
          return 'super-resolution';
        }
      }
    }
  }

  // --- fallback: output-count heuristic ----------------------------------

  if (outputs.length >= 3) return 'detection';  // boxes + scores + classes

  // No shape info available (ORT-web limitation): 1-in 1-out models are
  // assumed image-to-image (most common custom model use case).
  if (inputs.length === 1 && outputs.length === 1) return 'super-resolution';

  return 'generic';
}

// ---------------------------------------------------------------------------
// Port generation
// ---------------------------------------------------------------------------

/** Map {@link DataType} strings appropriate for each task's output. */
function outputDataTypeForTask(task: OnnxTask | undefined): Port['dataType'] {
  switch (task) {
    case 'detection':
      return 'roi';
    case 'depth-estimation':
      return 'sampler2D';  // single-channel depth map rendered as texture
    case 'segmentation':
      return 'sampler2D';  // mask texture
    case 'background-removal':
      return 'sampler2D';  // RGBA with alpha mask
    case 'super-resolution':
    case 'style-transfer':
    case 'denoising':
      return 'sampler2D';  // image-to-image
    default:
      return 'sampler2D';  // generic / unknown → image texture output
  }
}

/**
 * Convert introspected model metadata into the {@link Port} arrays expected
 * by the node graph.
 */
export function metaToDefaultPorts(
  meta: OnnxModelMeta,
): { inputs: Port[]; outputs: Port[] } {
  const inputs: Port[] = meta.inputs.map((inp, i) => ({
    id: `onnx_in_${inp.name || i}`,
    label: inp.name || `input_${i}`,
    dataType: 'sampler2D' as const,  // default assumption: image model
    direction: 'input' as const,
  }));

  const outType = outputDataTypeForTask(meta.inferredTask);

  const outputs: Port[] = meta.outputs.map((out, i) => ({
    id: `onnx_out_${out.name || i}`,
    label: out.name || `output_${i}`,
    dataType: outType,
    direction: 'output' as const,
  }));

  return { inputs, outputs };
}
