import type { ShaderNodeData } from '../../types';
import { OnnxInferenceSession, isGpuAllocError } from '../../engine/onnx/inference';
import { loadOnnxModel } from './OnnxResourceRegistry';
import type { WasmBrowserPlayerContract } from '../WasmSdkClient';
import type { WasmSdkClient } from '../WasmSdkClient';
import type { FrameStamp } from '../contract';
import { getOnnxModelDescriptor } from '../catalog';
import { requireSdk } from '../runtime';

export interface BrowserInferenceTask {
  nodeId: string;
  outputPortId?: string;
  textureInputs: Record<string, string>;
  graphRevision: number;
  nodeGeneration: number;
  inputStamp: FrameStamp;
}

interface InferenceResult {
  rgba?: Uint8Array<ArrayBufferLike> | Uint8ClampedArray<ArrayBufferLike>;
  width?: number;
  height?: number;
  data?: unknown;
}

interface TensorDescriptor {
  shape: number[];
}

type OnnxExecutionPlan =
  | { mode: 'single'; input: TensorDescriptor; output: TensorDescriptor }
  | {
      mode: 'tiled';
      outputWidth: number;
      outputHeight: number;
      scale: number;
      tileSize: number;
      minTileSize: number;
      fixedSize?: number | null;
      tiles: Array<{ input: TensorDescriptor }>;
    };

interface BrowserOnnxPlan {
  task: string;
  family: string;
  execution: OnnxExecutionPlan;
  classLabels?: string[];
}

interface BrowserOnnxTensor {
  tensor: number[];
  descriptor: TensorDescriptor;
}

interface BrowserOnnxDecodedOutput {
  rgba?: number[];
  width: number;
  height: number;
  dstX?: number;
  dstY?: number;
  data?: unknown;
}

export class BrowserInferenceProvider {
  private readonly nodes = new Map<string, ShaderNodeData>();
  private readonly sessions = new Map<string, OnnxInferenceSession>();
  private readonly inFlight = new Set<string>();

  reconcile(nodes: Array<{ id: string; data: ShaderNodeData }>): void {
    const nextIds = new Set(nodes.map((node) => node.id));
    for (const [nodeId, session] of this.sessions) {
      if (nextIds.has(nodeId)) continue;
      session.dispose();
      this.sessions.delete(nodeId);
    }
    this.nodes.clear();
    for (const node of nodes) this.nodes.set(node.id, node.data);
  }

  async execute(
    task: BrowserInferenceTask,
    player: WasmBrowserPlayerContract,
    onBackend: (nodeId: string, backend: 'webgpu' | 'wasm') => void,
  ): Promise<void> {
    if (this.inFlight.has(task.nodeId)) return;
    this.inFlight.add(task.nodeId);
    try {
      const node = this.nodes.get(task.nodeId);
      if (!node) throw new Error(`ONNX node ${task.nodeId} is not available`);
      const sourceNodeId = Object.values(task.textureInputs)[0];
      if (!sourceNodeId) throw new Error(`ONNX node ${task.nodeId} has no texture input`);
      const { width, height } = player.outputInfo(sourceNodeId);
      const rgba = new Uint8ClampedArray(await player.readOutputRgba(sourceNodeId));
      const session = await this.session(task.nodeId, node);
      onBackend(task.nodeId, session.isWasmFallback ? 'wasm' : 'webgpu');
      const sdk = requireSdk();
      const result = await this.runTask(sdk, session, node, rgba, width, height);
      if (result.rgba && result.width && result.height) {
        player.uploadRgba(task.nodeId, new Uint8Array(result.rgba), result.width, result.height);
      }
      const completion = sdk.buildBrowserOnnxCompletion({
        nodeId: task.nodeId,
        graphRevision: task.graphRevision,
        nodeGeneration: task.nodeGeneration,
        inputStamp: task.inputStamp,
        data: result.data ?? null,
        outputs: node.outputs.map((port) => ({ id: port.id, dataType: port.dataType })),
      });
      player.submitCompletion(completion);
    } finally {
      this.inFlight.delete(task.nodeId);
    }
  }

  close(): void {
    for (const session of this.sessions.values()) session.dispose();
    this.sessions.clear();
    this.nodes.clear();
  }

  private async session(nodeId: string, node: ShaderNodeData): Promise<OnnxInferenceSession> {
    const existing = this.sessions.get(nodeId);
    if (existing) return existing;
    const modelId = node.onnxCatalogId ?? node.onnxModelId;
    const entry = node.onnxCatalogId ? getOnnxModelDescriptor(node.onnxCatalogId) : undefined;
    const buffer = await loadOnnxModel(modelId, entry, node.onnxCustomPath);
    const session = new OnnxInferenceSession();
    await session.loadFromBuffer(buffer);
    this.sessions.set(nodeId, session);
    return session;
  }

  private async runTask(
    sdk: WasmSdkClient,
    session: OnnxInferenceSession,
    node: ShaderNodeData,
    rgba: Uint8ClampedArray,
    width: number,
    height: number,
  ): Promise<InferenceResult> {
    const entry = node.onnxCatalogId ? getOnnxModelDescriptor(node.onnxCatalogId) : null;
    const params: Record<string, unknown> = { ...(node.onnxParams ?? {}) };
    if (params.scoreThreshold === undefined && node.onnxScoreThreshold !== undefined) {
      params.scoreThreshold = node.onnxScoreThreshold;
    }
    if (params.iouThreshold === undefined && node.onnxIouThreshold !== undefined) {
      params.iouThreshold = node.onnxIouThreshold;
    }
    const request = {
      modelId: entry?.id ?? node.onnxModelId ?? 'custom',
      task: entry?.task ?? 'generic',
      sourceWidth: width,
      sourceHeight: height,
      targetSize: node.onnxTargetSize,
      params,
      inputShape: session.inputShape,
      outputShape: session.outputShape,
    };
    let tileSize: number | undefined;
    for (;;) {
      const plan = sdk.planBrowserOnnxTask<BrowserOnnxPlan>({ ...request, tileSize });
      try {
        return await runPlannedTask(sdk, session, plan, rgba);
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        if (plan.execution.mode !== 'tiled' || !isGpuAllocError(message)) throw error;
        if (plan.execution.fixedSize || (tileSize !== undefined && tileSize <= plan.execution.minTileSize)) {
          if (session.isWasmFallback) throw error;
          await session.fallbackToWasm();
          tileSize = plan.execution.tileSize;
          continue;
        }
        tileSize = Math.max(plan.execution.minTileSize, plan.execution.tileSize >> 1);
      }
    }
  }
}

async function runPlannedTask(
  sdk: WasmSdkClient,
  session: OnnxInferenceSession,
  plan: BrowserOnnxPlan,
  rgba: Uint8ClampedArray,
): Promise<InferenceResult> {
  if (plan.execution.mode === 'single') {
    const encoded = sdk.encodeBrowserOnnxInput<BrowserOnnxTensor>(
      new Uint8Array(rgba),
      { plan },
    );
    const raw = await session.run(new Float32Array(encoded.tensor), encoded.descriptor.shape);
    const decoded = sdk.decodeBrowserOnnxOutput<BrowserOnnxDecodedOutput>(
      new Uint8Array(rgba),
      raw,
      { plan, outputShape: session.outputShape },
    );
    if (plan.task === 'detection') {
      const detections = Array.isArray(decoded.data) ? decoded.data : [];
      return {
        rgba: drawDetectionOverlay(rgba, decoded.width, decoded.height, detections, plan.classLabels ?? []),
        width: decoded.width,
        height: decoded.height,
        data: detections,
      };
    }
    if (plan.task === 'segmentation' && decoded.rgba) {
      return {
        rgba: drawSegmentationOverlay(
          rgba,
          decoded.width,
          decoded.height,
          Uint8Array.from(decoded.rgba),
          decoded.width,
          decoded.height,
        ),
        width: decoded.width,
        height: decoded.height,
      };
    }
    return {
      rgba: decoded.rgba ? Uint8Array.from(decoded.rgba) : undefined,
      width: decoded.width,
      height: decoded.height,
      data: decoded.data,
    };
  }

  const output = new Uint8ClampedArray(plan.execution.outputWidth * plan.execution.outputHeight * 4);
  for (let tileIndex = 0; tileIndex < plan.execution.tiles.length; tileIndex++) {
    const encoded = sdk.encodeBrowserOnnxInput<BrowserOnnxTensor>(
      new Uint8Array(rgba),
      { plan, tileIndex },
    );
    const raw = await session.run(new Float32Array(encoded.tensor), encoded.descriptor.shape);
    const decoded = sdk.decodeBrowserOnnxOutput<BrowserOnnxDecodedOutput>(
      new Uint8Array(rgba),
      raw,
      { plan, tileIndex },
    );
    if (!decoded.rgba) continue;
    blitRgba(
      output,
      plan.execution.outputWidth,
      Uint8Array.from(decoded.rgba),
      decoded.width,
      decoded.height,
      decoded.dstX ?? 0,
      decoded.dstY ?? 0,
    );
  }
  return {
    rgba: output,
    width: plan.execution.outputWidth,
    height: plan.execution.outputHeight,
  };
}

function blitRgba(
  destination: Uint8ClampedArray,
  destinationWidth: number,
  source: Uint8Array,
  sourceWidth: number,
  sourceHeight: number,
  dstX: number,
  dstY: number,
): void {
  for (let y = 0; y < sourceHeight; y++) {
    const sourceOffset = y * sourceWidth * 4;
    const destinationOffset = ((dstY + y) * destinationWidth + dstX) * 4;
    destination.set(source.subarray(sourceOffset, sourceOffset + sourceWidth * 4), destinationOffset);
  }
}

function sourceCanvas(rgba: Uint8ClampedArray, width: number, height: number): OffscreenCanvas {
  const canvas = new OffscreenCanvas(width, height);
  const context = canvas.getContext('2d');
  if (!context) throw new Error('Browser inference requires OffscreenCanvas 2D');
  const pixels = new Uint8ClampedArray(rgba.length);
  pixels.set(rgba);
  context.putImageData(new ImageData(pixels, width, height), 0, 0);
  return canvas;
}

function drawDetectionOverlay(
  rgba: Uint8ClampedArray,
  width: number,
  height: number,
  detections: Array<{ bbox: [number, number, number, number]; score: number; classId: number }>,
  classLabels: readonly string[],
): Uint8Array {
  const canvas = sourceCanvas(rgba, width, height);
  const context = canvas.getContext('2d')!;
  context.lineWidth = Math.max(1, Math.round(Math.min(width, height) / 320));
  context.font = `${Math.max(10, Math.round(height / 40))}px system-ui, sans-serif`;
  context.textBaseline = 'top';
  for (const detection of detections) {
    const [x1, y1, x2, y2] = detection.bbox;
    const x = Math.round(x1 * width);
    const y = Math.round(y1 * height);
    const boxWidth = Math.round((x2 - x1) * width);
    const boxHeight = Math.round((y2 - y1) * height);
    if (boxWidth <= 0 || boxHeight <= 0) continue;
    const color = `hsl(${(detection.classId * 47) % 360}, 82%, 55%)`;
    context.strokeStyle = color;
    context.strokeRect(x, y, boxWidth, boxHeight);
    const className = classLabels[detection.classId] ?? `class_${detection.classId}`;
    const label = `${className} ${Math.round(detection.score * 100)}%`;
    const textHeight = Math.round(parseInt(context.font, 10) * 1.2);
    context.fillStyle = color;
    context.fillRect(x, Math.max(0, y - textHeight), context.measureText(label).width + 6, textHeight);
    context.fillStyle = '#fff';
    context.fillText(label, x + 3, Math.max(0, y - textHeight) + 1);
  }
  return new Uint8Array(context.getImageData(0, 0, width, height).data);
}

function drawSegmentationOverlay(
  rgba: Uint8ClampedArray,
  width: number,
  height: number,
  maskRgba: Uint8Array,
  maskWidth: number,
  maskHeight: number,
): Uint8Array {
  const canvas = sourceCanvas(rgba, width, height);
  const context = canvas.getContext('2d')!;
  const mask = new OffscreenCanvas(maskWidth, maskHeight);
  const maskContext = mask.getContext('2d');
  if (!maskContext) throw new Error('Browser inference mask requires OffscreenCanvas 2D');
  maskContext.putImageData(new ImageData(new Uint8ClampedArray(maskRgba), maskWidth, maskHeight), 0, 0);
  context.drawImage(mask, 0, 0, width, height);
  return new Uint8Array(context.getImageData(0, 0, width, height).data);
}
