/// <reference lib="webworker" />

import type { Node } from '@xyflow/react';
import type { DataType, ShaderNodeData } from '../types';
import { Compositor, type FrameInputs } from '../engine/compositor';
import type { CanonicalExecutionPlan, RuntimeWorkCommand } from '../engine/executionEngine';
import { initializeSdk, type WasmRuntimeContract } from './index';
import type { BrowserWorkerRequest, BrowserWorkerResponse } from './browserWorkerProtocol';
import type { OutputDeliveryBatch, OutputPayload } from './contract';
import { compactRuntimeText, runtimeLog } from './runtimeLog';

const scope = self as DedicatedWorkerGlobalScope;
let compositor: Compositor | null = null;
let runtime: WasmRuntimeContract | null = null;
let frameTimer: number | null = null;
let running = false;
let previewNodeId: string | null = null;
let rendererIds = new Set<string>();
let previewPending = false;
let resolution = new Float32Array([512, 512, 1]);
const PREVIEW_SUBSCRIPTION_ID = 'browser-preview';
const outputPortByNode = new Map<string, string>();
const rendererSourceByNode = new Map<string, string>();
const outputGenerationByNode = new Map<string, number>();
const logicalSubscriptionByNode = new Map<string, {
  subscriptionId: string;
  portId: string;
  dataType: DataType;
}>();
let graphRevision = 0;
let previewSubscribed = false;

function post(message: BrowserWorkerResponse): void {
  scope.postMessage(message);
  if ('id' in message) {
    runtimeLog('browser-worker', 'debug', 'response', { id: message.id });
  } else {
    runtimeLog('browser-worker', 'debug', 'event', { type: message.type });
  }
}

function requireCompositor(): Compositor {
  if (!compositor) throw new Error('Browser worker is not initialized');
  return compositor;
}

function requireRuntime(): WasmRuntimeContract {
  if (!runtime) throw new Error('Browser worker runtime is not initialized');
  return runtime;
}

function applyGraphContract(
  core: WasmRuntimeContract,
  nodes: Node<ShaderNodeData>[],
  plan: CanonicalExecutionPlan,
): void {
  graphRevision = plan.revision;
  outputGenerationByNode.clear();
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
    if (current && current.subscriptionId !== desired.subscriptionId) {
      core.unsubscribeOutput(current.subscriptionId);
      logicalSubscriptionByNode.delete(nodeId);
    }
    const subscription = {
      subscriptionId: desired.subscriptionId,
      output: { nodeId, portId: desired.portId },
      delivery: 'on-change' as const,
      transport: 'value' as const,
    };
    if (logicalSubscriptionByNode.has(nodeId)) {
      core.updateOutputSubscription(subscription);
    } else {
      core.subscribeOutput(subscription);
      logicalSubscriptionByNode.set(nodeId, desired);
    }
  }
  for (const node of plan.nodes) {
    if (node.nodeType !== 'renderer') continue;
    const sourceNodeId = Object.values(node.upstream)[0];
    if (sourceNodeId) rendererSourceByNode.set(node.id, sourceNodeId);
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
  }
  const previewDelivery = batch.deliveries.find(
    (item) => item.subscriptionId === PREVIEW_SUBSCRIPTION_ID,
  );
  if (!previewDelivery || previewPending) return;
  previewPending = true;
  void requireCompositor()
    .readNodeOutput(previewDelivery.state.output.nodeId, (nodeId, dataUrl) => {
      previewPending = false;
      post({ type: 'output', nodeId, dataUrl });
    })
    .catch((error) => {
      previewPending = false;
      runtimeLog('browser-worker', 'warn', 'preview-readback-failed', {
        error: compactRuntimeText(error),
      });
    });
}

function publishResourceOutput(nodeId: string, portId: string): OutputDeliveryBatch {
  const core = requireRuntime();
  const clock = core.lastClock;
  const outputGeneration = (outputGenerationByNode.get(nodeId) ?? 0) + 1;
  outputGenerationByNode.set(nodeId, outputGeneration);
  core.publishOutput({
    output: { nodeId, portId },
    graphRevision,
    outputGeneration,
    evaluationStamp: {
      epoch: clock.epoch,
      frame: clock.frame,
      timelineNs: clock.timelineNs,
      deadlineNs: clock.nextDeadlineNs,
    },
    contentStamp: {
      epoch: clock.epoch,
      timelineNs: clock.timelineNs,
    },
    payload: { kind: 'resource', value: { handle: outputGeneration } },
  });
  return core.drainDeliveries();
}

function publishPreviewDelivery(): void {
  if (!previewSubscribed || !previewNodeId || previewPending) return;
  const portId = outputPortByNode.get(previewNodeId);
  if (!portId) return;
  consumeOutputDeliveries(publishResourceOutput(previewNodeId, portId));
}

function publishLogicalOutput(nodeId: string, data: unknown): void {
  const subscription = logicalSubscriptionByNode.get(nodeId);
  if (!subscription) return;
  let payload: OutputPayload;
  if (subscription.dataType === 'float') {
    payload = { kind: 'float', value: Number(data) };
  } else if (subscription.dataType === 'int') {
    payload = { kind: 'int', value: Number(data) };
  } else if (subscription.dataType === 'uint') {
    payload = { kind: 'uint', value: Number(data) };
  } else if (subscription.dataType === 'bool') {
    payload = { kind: 'bool', value: Boolean(data) };
  } else {
    payload = { kind: 'json', value: data };
  }
  const core = requireRuntime();
  const clock = core.lastClock;
  const outputGeneration = (outputGenerationByNode.get(nodeId) ?? 0) + 1;
  outputGenerationByNode.set(nodeId, outputGeneration);
  core.publishOutput({
    output: { nodeId, portId: subscription.portId },
    graphRevision,
    outputGeneration,
    evaluationStamp: {
      epoch: clock.epoch,
      frame: clock.frame,
      timelineNs: clock.timelineNs,
      deadlineNs: clock.nextDeadlineNs,
    },
    contentStamp: { epoch: clock.epoch, timelineNs: clock.timelineNs },
    payload,
  });
  consumeOutputDeliveries(core.drainDeliveries());
}

function stopLoop(): void {
  running = false;
  if (frameTimer !== null) {
    scope.clearTimeout(frameTimer);
    frameTimer = null;
  }
}

function scheduleFrame(): void {
  if (!running || frameTimer !== null) return;
  frameTimer = scope.setTimeout(runFrame, 0);
}

function runFrame(): void {
  frameTimer = null;
  if (!running) return;
  const now = performance.now();
  const date = new Date();
  const inputs = new Float32Array([
    date.getFullYear(), date.getMonth() + 1, date.getDate(),
    date.getHours() * 3600 + date.getMinutes() * 60 + date.getSeconds(),
  ]);
  try {
    requireRuntime().advance({
      time: now / 1000,
      delta: 0,
      frame: 0,
      date: inputs,
      mouse: new Float32Array(4),
      resolution,
    });
  } catch (error) {
    runtimeLog('browser-worker', 'error', 'frame-advance-failed', { error: compactRuntimeText(error) });
    throw error;
  }
  const work = requireRuntime().drainWork<RuntimeWorkCommand[]>();
  const clock = requireRuntime().lastClock;
  if (clock.frame % 60 === 0) {
    runtimeLog('browser-worker', 'debug', 'frame', {
      now, frame: clock.frame, commands: work.length, previewNodeId,
    });
  }
  const builtins: FrameInputs = {
    time: clock.timelineNs / 1_000_000_000,
    delta: (clock.timelineNs - clock.previousTimelineNs) / 1_000_000_000,
    frame: clock.frame,
    date: inputs,
    mouse: new Float32Array(4),
    resolution,
  };
  requireCompositor().render(builtins, work);
  for (const command of work) {
    if (command.kind === 'renderer') requireCompositor().renderRendererToScreen(command.nodeId);
    if (command.kind === 'math' && command.scalarOutput !== undefined) {
      publishLogicalOutput(command.nodeId, command.scalarOutput);
    }
  }
  if (previewNodeId && work.some((command) => command.nodeId === previewNodeId)) {
    publishPreviewDelivery();
  }
  post({ type: 'frame', frame: clock.frame, time: builtins.time, fps: builtins.delta > 0 ? 1 / builtins.delta : 0 });
  scheduleFrame();
}

async function handle(message: BrowserWorkerRequest): Promise<unknown> {
  switch (message.type) {
    case 'initialize': {
      const sdk = await initializeSdk();
      runtime = sdk.createRuntime();
      compositor = new Compositor();
      await compositor.init(message.canvas);
      resolution = new Float32Array([message.canvas.width, message.canvas.height, 1]);
      return undefined;
    }
    case 'play': {
      const core = requireRuntime();
      core.setGraph(message.nodes, message.edges);
      const plan = core.executionPlan<CanonicalExecutionPlan>();
      consumeOutputDeliveries(core.drainDeliveries());
      applyGraphContract(core, message.nodes, plan);
      rendererIds = new Set(message.nodes.filter((node) => node.data.type === 'renderer').map((node) => node.id));
      const pending = requireCompositor().prepare(
        message.nodes, message.edges,
        (nodeId, error) => post({ type: 'node-error', nodeId, error }),
        (nodeId, width, height) => post({ type: 'output-size', nodeId, width, height }),
        publishLogicalOutput,
        undefined,
        scheduleFrame,
        (nodeId, backend) => post({ type: 'backend', nodeId, backend }),
        plan,
      );
      await Promise.all(pending);
      core.play(Math.round(performance.now() * 1_000_000));
      running = true;
      scheduleFrame();
      return undefined;
    }
    case 'update-graph':
      rendererIds = new Set(message.nodes.filter((node) => node.data.type === 'renderer').map((node) => node.id));
      {
        const core = requireRuntime();
        core.setGraph(message.nodes, message.edges);
        const plan = core.executionPlan<CanonicalExecutionPlan>();
        consumeOutputDeliveries(core.drainDeliveries());
        applyGraphContract(core, message.nodes, plan);
        await Promise.all(requireCompositor().prepare(
          message.nodes,
          message.edges,
          (nodeId, error) => post({ type: 'node-error', nodeId, error }),
          (nodeId, width, height) => post({ type: 'output-size', nodeId, width, height }),
          publishLogicalOutput,
          undefined,
          scheduleFrame,
          (nodeId, backend) => post({ type: 'backend', nodeId, backend }),
          plan,
        ));
      }
      scheduleFrame();
      return undefined;
    case 'pause':
      previewPending = false;
      stopLoop();
      requireRuntime().pause(Math.round(performance.now() * 1_000_000));
      return undefined;
    case 'resume':
      requireRuntime().resume(Math.round(performance.now() * 1_000_000));
      running = true;
      scheduleFrame();
      return undefined;
    case 'stop':
      stopLoop();
      requireRuntime().stop();
      return undefined;
    case 'set-preview': {
      const core = requireRuntime();
      if (previewSubscribed) {
        core.unsubscribeOutput(PREVIEW_SUBSCRIPTION_ID);
        previewSubscribed = false;
      }
      const requested = message.nodeId;
      previewNodeId = requested && !rendererIds.has(requested) && outputPortByNode.has(requested)
        ? requested
        : null;
      previewPending = false;
      if (previewNodeId) {
        core.subscribeOutput({
          subscriptionId: PREVIEW_SUBSCRIPTION_ID,
          output: { nodeId: previewNodeId, portId: outputPortByNode.get(previewNodeId)! },
          delivery: 'latest',
          transport: 'preview',
        });
        previewSubscribed = true;
        publishPreviewDelivery();
      }
      runtimeLog('browser-worker', 'info', 'set-preview-node', {
        requested, effective: previewNodeId, rendererSelection: !!requested && rendererIds.has(requested),
      });
      return undefined;
    }
    case 'capture': {
      const core = requireRuntime();
      const nodeId = rendererSourceByNode.get(message.nodeId) ?? message.nodeId;
      const portId = outputPortByNode.get(nodeId);
      if (!portId) return null;
      const subscriptionId = `browser-capture-${message.id}`;
      core.subscribeOutput({
        subscriptionId,
        output: { nodeId, portId },
        delivery: 'latest',
        transport: 'capture',
      });
      try {
        const batch = publishResourceOutput(nodeId, portId);
        consumeOutputDeliveries(batch);
        if (!batch.deliveries.some((delivery) => delivery.subscriptionId === subscriptionId)) {
          return null;
        }
        return await requireCompositor().captureScreenshot(message.nodeId);
      } finally {
        core.unsubscribeOutput(subscriptionId);
      }
    }
    case 'close':
      stopLoop();
      runtime?.dispose();
      compositor?.dispose();
      runtime = null;
      compositor = null;
      scope.close();
      return undefined;
  }
}

scope.onmessage = (event: MessageEvent<BrowserWorkerRequest>) => {
  const message = event.data;
  void handle(message).then(
    (value) => post({ id: message.id, ok: true, value }),
    (error: unknown) => post({
      id: message.id,
      ok: false,
      error: error instanceof Error ? error.message : String(error),
    }),
  );
};
