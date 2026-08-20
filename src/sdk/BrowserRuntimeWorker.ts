/// <reference lib="webworker" />

import type { Edge, Node } from '@xyflow/react';
import type { DataType, ShaderNodeData } from '../types';
import { initializeSdk } from './runtime';
import type { WasmBrowserPlayerContract } from './WasmSdkClient';
import type { BrowserWorkerIncoming, BrowserWorkerRequest, BrowserWorkerResponse } from './browserWorkerProtocol';
import type { OutputDeliveryBatch } from './contract';
import { BrowserInferenceProvider, type BrowserInferenceTask } from './internal/BrowserInferenceProvider';
import { compactRuntimeText, runtimeLog } from './runtimeLog';
import { cacheOnnxModel } from './internal/OnnxResourceRegistry';
import { blobToDataUrl } from './internal/browserPreviewEncoding';

const scope = self as DedicatedWorkerGlobalScope;
const inference = new BrowserInferenceProvider();
const PREVIEW_SUBSCRIPTION_ID = 'browser-preview';
const FRAME_INTERVAL_MS = 1000 / 60;
const RENDERER_PREVIEW_INTERVAL_MS = 1000 / 15;

let player: WasmBrowserPlayerContract | null = null;
let frameTimer: number | null = null;
let running = false;
let previewNodeId: string | null = null;
let previewSubscribed = false;
let previewPending = false;
let resolution = new Float32Array([512, 512, 1]);
const dateInput = new Float32Array(4);
const mouseInput = new Float32Array(4);
let nextFrameAt = 0;
let previousTimelineNs = 0;
let activeVideoNodeIds = new Set<string>();
let rendererIds = new Set<string>();
const outputPortByNode = new Map<string, string>();
const rendererSourceByNode = new Map<string, string>();
const logicalSubscriptionByNode = new Map<string, {
  subscriptionId: string;
  portId: string;
  dataType: DataType;
}>();
const rendererPreviewAt = new Map<string, number>();
const rendererPreviewPending = new Set<string>();
type PendingVideoFrame = { frameId: number; frame: ImageBitmap };
let pendingVideoFrames = new Map<string, PendingVideoFrame>();

function post(message: BrowserWorkerResponse): void {
  scope.postMessage(message);
}

function requirePlayer(): WasmBrowserPlayerContract {
  if (!player) throw new Error('Browser Player is not initialized');
  return player;
}

function releaseVideoFrame(pending: PendingVideoFrame): void {
  pending.frame.close();
}

function clearVideoFrames(): void {
  for (const pending of pendingVideoFrames.values()) releaseVideoFrame(pending);
  pendingVideoFrames.clear();
}

function applyGraphContract(nodes: Node<ShaderNodeData>[], edges: Edge[]): void {
  const core = requirePlayer();
  activeVideoNodeIds = new Set(
    nodes
      .filter((node) => node.data.type === 'input' && node.data.inputMode === 'video')
      .map((node) => node.id),
  );
  rendererIds = new Set(
    nodes.filter((node) => node.data.type === 'renderer').map((node) => node.id),
  );
  inference.reconcile(nodes);
  for (const [nodeId, pending] of pendingVideoFrames) {
    if (activeVideoNodeIds.has(nodeId)) continue;
    releaseVideoFrame(pending);
    pendingVideoFrames.delete(nodeId);
  }

  outputPortByNode.clear();
  rendererSourceByNode.clear();
  const desiredLogical = new Map<string, {
    subscriptionId: string;
    portId: string;
    dataType: DataType;
  }>();
  for (const node of nodes) {
    const textureOutput = node.data.outputs.find(
      (port) => port.dataType === 'sampler2D' || port.dataType === 'samplerCube',
    );
    if (textureOutput) outputPortByNode.set(node.id, textureOutput.id);
    const logicalOutput = node.data.outputs.find(
      (port) => port.dataType !== 'sampler2D' && port.dataType !== 'samplerCube',
    );
    if (logicalOutput) {
      desiredLogical.set(node.id, {
        subscriptionId: `browser-value-${node.id}-${logicalOutput.id}`,
        portId: logicalOutput.id,
        dataType: logicalOutput.dataType,
      });
    }
  }
  for (const [nodeId, subscription] of logicalSubscriptionByNode) {
    if (desiredLogical.has(nodeId)) continue;
    core.unsubscribeOutput(subscription.subscriptionId);
    logicalSubscriptionByNode.delete(nodeId);
  }
  for (const [nodeId, desired] of desiredLogical) {
    const current = logicalSubscriptionByNode.get(nodeId);
    if (current?.subscriptionId === desired.subscriptionId) continue;
    if (current) core.unsubscribeOutput(current.subscriptionId);
    core.subscribeOutput({
      subscriptionId: desired.subscriptionId,
      output: { nodeId, portId: desired.portId },
      delivery: 'on-change',
      transport: 'value',
    });
    logicalSubscriptionByNode.set(nodeId, desired);
  }
  for (const rendererId of rendererIds) {
    const sourceNodeId = edges.find((edge) => edge.target === rendererId)?.source;
    if (sourceNodeId) rendererSourceByNode.set(rendererId, sourceNodeId);
  }
  for (const node of nodes) {
    if (node.data.type === 'shader' || node.data.type === 'renderer') {
      post({ type: 'backend', nodeId: node.id, backend: 'webgpu' });
    }
  }
}

function consumeOutputDeliveries(batch: OutputDeliveryBatch): void {
  for (const invalidation of batch.invalidations) {
    if (invalidation.subscriptionId === PREVIEW_SUBSCRIPTION_ID) {
      previewSubscribed = false;
      previewNodeId = null;
    }
    for (const [nodeId, subscription] of logicalSubscriptionByNode) {
      if (subscription.subscriptionId === invalidation.subscriptionId) {
        logicalSubscriptionByNode.delete(nodeId);
      }
    }
  }
  for (const delivery of batch.deliveries) {
    const logical = logicalSubscriptionByNode.get(delivery.state.output.nodeId);
    if (logical?.subscriptionId === delivery.subscriptionId) {
      post({
        type: 'output-data',
        nodeId: delivery.state.output.nodeId,
        data: delivery.state.payload.value,
      });
    }
    if (delivery.subscriptionId === PREVIEW_SUBSCRIPTION_ID && !previewPending) {
      void publishPreview(delivery.state.output.nodeId);
    }
  }
}

async function readOutputDataUrl(nodeId: string): Promise<string> {
  const core = requirePlayer();
  const { width, height } = core.outputInfo(nodeId);
  const rgba = await core.readOutputRgba(nodeId);
  const canvas = new OffscreenCanvas(width, height);
  const context = canvas.getContext('2d');
  if (!context) throw new Error('Cannot create preview canvas');
  context.putImageData(new ImageData(new Uint8ClampedArray(rgba), width, height), 0, 0);
  return await blobToDataUrl(await canvas.convertToBlob({ type: 'image/png' }));
}

async function publishPreview(nodeId: string): Promise<void> {
  previewPending = true;
  try {
    post({ type: 'output', nodeId, dataUrl: await readOutputDataUrl(nodeId) });
  } catch (error) {
    runtimeLog('browser-worker', 'warn', 'preview-readback-failed', {
      nodeId,
      error: compactRuntimeText(error),
    });
  } finally {
    previewPending = false;
  }
}

function publishRendererPreview(nodeId: string, now: number): void {
  if (rendererPreviewPending.has(nodeId)) return;
  const lastAt = rendererPreviewAt.get(nodeId) ?? 0;
  if (now - lastAt < RENDERER_PREVIEW_INTERVAL_MS) return;
  rendererPreviewAt.set(nodeId, now);
  rendererPreviewPending.add(nodeId);
  void readOutputDataUrl(nodeId).then((dataUrl) => {
    post({ type: 'output', nodeId, dataUrl });
  }).catch((error: unknown) => {
    runtimeLog('browser-worker', 'warn', 'renderer-preview-readback-failed', {
      nodeId,
      error: compactRuntimeText(error),
    });
  }).finally(() => rendererPreviewPending.delete(nodeId));
}

function stopLoop(): void {
  running = false;
  nextFrameAt = 0;
  if (frameTimer !== null) {
    scope.clearTimeout(frameTimer);
    frameTimer = null;
  }
}

function scheduleFrame(): void {
  if (!running || frameTimer !== null) return;
  frameTimer = scope.setTimeout(runFrame, Math.max(0, nextFrameAt - performance.now()));
}

function runFrame(): void {
  frameTimer = null;
  if (!running) return;
  const now = performance.now();
  nextFrameAt = now + FRAME_INTERVAL_MS;
  const date = new Date();
  dateInput[0] = date.getFullYear();
  dateInput[1] = date.getMonth() + 1;
  dateInput[2] = date.getDate();
  dateInput[3] = date.getHours() * 3600 + date.getMinutes() * 60 + date.getSeconds();
  mouseInput.fill(0);
  const frameBatch = pendingVideoFrames;
  pendingVideoFrames = new Map();
  try {
    for (const [nodeId, pending] of frameBatch) {
      requirePlayer().uploadFrame(nodeId, pending.frame, Math.round(now * 1_000_000));
    }
    const result = requirePlayer().frame<BrowserInferenceTask>({
      time: now / 1000,
      delta: 0,
      frame: 0,
      date: dateInput,
      mouse: mouseInput,
      resolution,
    });
    consumeOutputDeliveries(requirePlayer().drainDeliveries());
    for (const task of result.inferenceTasks) {
      void inference.execute(task, requirePlayer(), (nodeId, backend) => {
        post({ type: 'backend', nodeId, backend });
      }).then(scheduleFrame).catch((error: unknown) => {
        post({ type: 'node-error', nodeId: task.nodeId, error: compactRuntimeText(error) });
      });
    }
    for (const rendererId of rendererIds) publishRendererPreview(rendererId, now);
    const deltaNs = result.clock.timelineNs - previousTimelineNs;
    previousTimelineNs = result.clock.timelineNs;
    post({
      type: 'frame',
      frame: result.clock.frame,
      time: result.clock.timelineNs / 1_000_000_000,
      fps: deltaNs > 0 ? 1_000_000_000 / deltaNs : 0,
    });
  } catch (error) {
    post({ type: 'node-error', nodeId: null, error: compactRuntimeText(error) });
  } finally {
    for (const pending of frameBatch.values()) releaseVideoFrame(pending);
  }
  scheduleFrame();
}

async function handle(message: BrowserWorkerRequest): Promise<unknown> {
  switch (message.type) {
    case 'initialize': {
      const sdk = await initializeSdk();
      player = await sdk.createBrowserPlayer(message.canvas);
      resolution = new Float32Array([message.canvas.width, message.canvas.height, 1]);
      return undefined;
    }
    case 'register-onnx-model':
      cacheOnnxModel(message.modelId, message.buffer);
      return undefined;
    case 'play': {
      const core = requirePlayer();
      core.setGraph(message.nodes, message.edges);
      applyGraphContract(message.nodes, message.edges);
      core.play(Math.round(performance.now() * 1_000_000));
      previousTimelineNs = 0;
      running = true;
      scheduleFrame();
      return undefined;
    }
    case 'update-graph':
      requirePlayer().setGraph(message.nodes, message.edges);
      applyGraphContract(message.nodes, message.edges);
      scheduleFrame();
      return undefined;
    case 'pause':
      stopLoop();
      requirePlayer().pause(Math.round(performance.now() * 1_000_000));
      return undefined;
    case 'resume':
      requirePlayer().resume(Math.round(performance.now() * 1_000_000));
      running = true;
      scheduleFrame();
      return undefined;
    case 'stop':
      stopLoop();
      clearVideoFrames();
      requirePlayer().stop();
      return undefined;
    case 'set-preview': {
      const core = requirePlayer();
      if (previewSubscribed) core.unsubscribeOutput(PREVIEW_SUBSCRIPTION_ID);
      const requested = message.nodeId;
      previewNodeId = requested && !rendererIds.has(requested) && outputPortByNode.has(requested)
        ? requested
        : null;
      previewSubscribed = false;
      previewPending = false;
      if (previewNodeId) {
        core.subscribeOutput({
          subscriptionId: PREVIEW_SUBSCRIPTION_ID,
          output: { nodeId: previewNodeId, portId: outputPortByNode.get(previewNodeId)! },
          delivery: 'latest',
          transport: 'preview',
        });
        previewSubscribed = true;
        void publishPreview(previewNodeId);
      }
      return undefined;
    }
    case 'capture': {
      const nodeId = rendererSourceByNode.get(message.nodeId) ?? message.nodeId;
      if (!outputPortByNode.has(nodeId) && !rendererIds.has(message.nodeId)) return null;
      return await readOutputDataUrl(message.nodeId);
    }
    case 'close':
      stopLoop();
      clearVideoFrames();
      inference.close();
      player?.close();
      player = null;
      scope.close();
      return undefined;
  }
}

scope.onmessage = (event: MessageEvent<BrowserWorkerIncoming>) => {
  const message = event.data;
  if (message.type === 'video-frame') {
    if (!activeVideoNodeIds.has(message.nodeId)) {
      message.frame.close();
      post({ type: 'video-frame-consumed', nodeId: message.nodeId, frameId: message.frameId });
      return;
    }
    const previous = pendingVideoFrames.get(message.nodeId);
    if (previous) releaseVideoFrame(previous);
    pendingVideoFrames.set(message.nodeId, { frameId: message.frameId, frame: message.frame });
    post({ type: 'video-frame-consumed', nodeId: message.nodeId, frameId: message.frameId });
    scheduleFrame();
    return;
  }
  void handle(message).then(
    (value) => post({ id: message.id, ok: true, value }),
    (error: unknown) => post({
      id: message.id,
      ok: false,
      error: error instanceof Error ? error.message : String(error),
    }),
  );
};
