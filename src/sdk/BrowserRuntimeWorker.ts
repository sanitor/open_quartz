/// <reference lib="webworker" />

import { Compositor, type FrameInputs } from '../engine/compositor';
import type { RuntimeWorkCommand } from '../engine/executionEngine';
import { initializeSdk, type WasmRuntimeContract } from './index';
import type { BrowserWorkerRequest, BrowserWorkerResponse } from './browserWorkerProtocol';
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
  }
  if (previewNodeId && !previewPending) {
    previewPending = true;
    void requireCompositor().readNodeOutput(previewNodeId, (nodeId, dataUrl) => {
      previewPending = false;
      post({ type: 'output', nodeId, dataUrl });
    }).catch((error) => {
      previewPending = false;
      runtimeLog('browser-worker', 'warn', 'preview-readback-failed', { error: compactRuntimeText(error) });
    });
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
      rendererIds = new Set(message.nodes.filter((node) => node.data.type === 'renderer').map((node) => node.id));
      const pending = requireCompositor().prepare(
        message.nodes, message.edges,
        (nodeId, error) => post({ type: 'node-error', nodeId, error }),
        (nodeId, width, height) => post({ type: 'output-size', nodeId, width, height }),
        (nodeId, data) => post({ type: 'output-data', nodeId, data }),
        (nodeId, dataUrl) => post({ type: 'output', nodeId, dataUrl }),
        scheduleFrame,
        (nodeId, backend) => post({ type: 'backend', nodeId, backend }),
      );
      await Promise.all(pending);
      core.play(Math.round(performance.now() * 1_000_000));
      running = true;
      scheduleFrame();
      return undefined;
    }
    case 'update-graph':
      rendererIds = new Set(message.nodes.filter((node) => node.data.type === 'renderer').map((node) => node.id));
      requireRuntime().setGraph(message.nodes, message.edges);
      await Promise.all(requireCompositor().prepare(message.nodes, message.edges));
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
      const requested = message.nodeId;
      previewNodeId = requested && !rendererIds.has(requested) ? requested : null;
      previewPending = false;
      runtimeLog('browser-worker', 'info', 'set-preview-node', {
        requested, effective: previewNodeId, rendererSelection: !!requested && rendererIds.has(requested),
      });
      return undefined;
    }
    case 'capture':
      return await requireCompositor().captureScreenshot(message.nodeId);
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
