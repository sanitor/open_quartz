/** PipelineService is the only bridge between Zustand and a host runtime. */
import { BrowserPipelineRuntime } from '../sdk/BrowserPipelineRuntime';
import { NativePipelineRuntime, type NativeOutputImage } from '../sdk/NativePipelineRuntime';
import type { PipelineHostRuntime, PipelineRuntimeCallbacks } from '../sdk/PipelineRuntime';
import { useGraphStore } from '../store/useGraphStore';
import { checkIsTauri } from '../utils/tauri';
import { runtimeLog } from '../sdk/runtimeLog';

export class PipelineService {
  private runtime: PipelineHostRuntime | null = null;
  private runtimePromise: Promise<PipelineHostRuntime> | null = null;
  private unsub: (() => void) | null = null;
  private operations: Promise<void> = Promise.resolve();
  private generation = 0;
  private nativeStartedAt = 0;
  private nativeFpsWindowAt = 0;
  private nativeFpsWindowFrame = 0;
  private nativeFps = 0;
  private lastUiFrameAt = Number.NEGATIVE_INFINITY;
  private nativePreviewCanvas: HTMLCanvasElement | null = null;
  private lastNativeRendererFrame: { nodeId: string; frame: NativeOutputImage } | null = null;
  private rendererStream: { nodeId: string; stream: MediaStream } | null = null;
  private readonly rendererFrameMetrics = new Map<string, { windowAt: number; frames: number }>();
  private rendererDrawMetrics = {
    windowAt: performance.now(),
    frames: 0,
    imageDataMs: 0,
    mirrorDrawMs: 0,
    totalMs: 0,
    maxMs: 0,
  };

  attach(canvas: HTMLCanvasElement): void {
    window.addEventListener('renderer-remount', this.handleRendererRemount);
    this.unsub = useGraphStore.subscribe((state, previous) => {
      if (state.loopState === 'playing' && previous.loopState === 'stopped') {
        this.resetNativeFrameMetrics();
        const store = useGraphStore.getState();
        this.rendererFrameMetrics.clear();
        store.clearRendererFps();
        store.clearOutputPreviews();
        store.clearNodeErrors();
        this.enqueue(async () => {
          const runtime = await this.ensureRuntime(canvas);
          if (useGraphStore.getState().loopState !== 'playing') return;
          useGraphStore.getState().setCaptureScreenshot(
            async (nodeId) => await runtime.captureScreenshot(nodeId),
          );
          const current = useGraphStore.getState();
          runtime.setPreviewNode(current.selectedNodeId);
          await runtime.play(current.nodes, current.edges);
        });
      }

      if (state.loopState === 'paused' && previous.loopState === 'playing') {
        this.enqueue(async () => { await this.runtime?.pause(); });
      }

      if (state.loopState === 'playing' && previous.loopState === 'paused') {
        this.enqueue(async () => { await this.runtime?.resume(); });
      }

      if (state.loopState === 'stopped' && previous.loopState !== 'stopped') {
        this.enqueue(async () => { await this.runtime?.stop(); });
      }

      if (
        state.loopState === 'playing'
        && (state.edges !== previous.edges || nodesChangedForRuntime(state.nodes, previous.nodes))
      ) {
        this.enqueue(async () => {
          await this.runtime?.updateGraph(state.nodes, state.edges);
        });
      }

      if (state.selectedNodeId !== previous.selectedNodeId) {
        runtimeLog('browser-host', 'info', 'preview-selection', {
          previous: previous.selectedNodeId,
          selected: state.selectedNodeId,
          playing: state.loopState,
          runtime: this.runtime?.constructor.name ?? null,
          runtimeReady: this.runtime !== null,
        });
        const runtime = this.runtime;
        runtime?.setPreviewNode(state.selectedNodeId);
        if (runtime?.requestPreviewRefresh) {
          requestAnimationFrame(() => runtime.requestPreviewRefresh?.());
        }
      }
    });
  }

  detach(): void {
    this.unsub?.();
    this.unsub = null;
    this.generation += 1;
    useGraphStore.getState().setCaptureScreenshot(null);
    const runtime = this.runtime;
    this.runtime = null;
    this.runtimePromise = null;
    window.removeEventListener('renderer-remount', this.handleRendererRemount);
    this.rendererStream = null;
    useGraphStore.setState({ nativeRendererStreams: {} });
    this.enqueue(async () => { await runtime?.close(); });
    this.lastNativeRendererFrame = null;
    this.rendererFrameMetrics.clear();
  }

  private async ensureRuntime(canvas: HTMLCanvasElement): Promise<PipelineHostRuntime> {
    if (this.runtime) return this.runtime;
    if (this.runtimePromise) return await this.runtimePromise;
    const generation = this.generation;
    this.runtimePromise = this.createRuntime(canvas).then((runtime) => {
      if (generation !== this.generation) {
        void runtime.close();
        throw new Error('Pipeline runtime initialization was superseded');
      }
      this.runtime = runtime;
      return runtime;
    }).finally(() => {
      this.runtimePromise = null;
    });
    return await this.runtimePromise;
  }

  private async createRuntime(canvas: HTMLCanvasElement): Promise<PipelineHostRuntime> {
    if (await checkIsTauri()) {
      this.resetNativeFrameMetrics();
      const runtime = new NativePipelineRuntime({
        onFrame: (frame) => {
          const now = performance.now();
          if (frame.frame < this.nativeFpsWindowFrame) this.resetNativeFrameMetrics(now);
          const elapsed = now - this.nativeFpsWindowAt;
          if (elapsed >= 500) {
            this.nativeFps = (frame.frame - this.nativeFpsWindowFrame) * 1000 / elapsed;
            this.nativeFpsWindowAt = now;
            this.nativeFpsWindowFrame = frame.frame;
          }
          this.handleFrame({
            frame: frame.frame,
            time: (now - this.nativeStartedAt) / 1000,
            fps: this.nativeFps,
          });
        },
        onRendererFrame: (nodeId, frame) => {
          this.lastNativeRendererFrame = { nodeId, frame };
          if (this.drawRendererFrame(nodeId, frame)) this.recordRendererPresentation(nodeId);
        },
        onRendererStream: (nodeId, stream) => {
          this.rendererStream = stream ? { nodeId, stream } : null;
          useGraphStore.getState().setNativeRendererStream(nodeId, stream);
          this.mountRendererStream(nodeId, stream);
        },
        onRendererVideoFrame: (nodeId) => {
          this.recordRendererPresentation(nodeId);
        },
        onError: (error) => this.handleError(null, error),
        onOutput: (nodeId, dataUrl) => useGraphStore.getState().setOutputPreview(nodeId, dataUrl),
        onOutputSize: (nodeId, width, height) => this.handleOutputSize(nodeId, width, height),
        onOutputData: (nodeId, data) => useGraphStore.getState().setOutputData(nodeId, data),
        onBackendDetected: (nodeId) => this.handleBackend(nodeId, 'native'),
        onNativeBackendDetected: (nodeId, backend) => {
          const store = useGraphStore.getState();
          store.updateNodeData(nodeId, { onnxBackend: 'native', onnxNativeBackend: backend });
        },
      });
      await runtime.initialize(canvas);
      return runtime;
    }

    const runtime = new BrowserPipelineRuntime(this.callbacks());
    await runtime.initialize(canvas);
    return runtime;
  }

  private callbacks(): PipelineRuntimeCallbacks {
    return {
      onFrame: (frame) => this.handleFrame(frame),
      onRendererPresented: (nodeId) => this.recordRendererPresentation(nodeId),
      onOutput: (nodeId, dataUrl) => useGraphStore.getState().setOutputPreview(nodeId, dataUrl),
      onNodeError: (nodeId, error) => this.handleError(nodeId, error),
      onOutputSize: (nodeId, width, height) => this.handleOutputSize(nodeId, width, height),
      onOutputData: (nodeId, data) => useGraphStore.getState().setOutputData(nodeId, data),
      onBackendDetected: (nodeId, backend) => this.handleBackend(nodeId, backend),
    };
  }

  private rendererMirrors(nodeId: string): NodeListOf<HTMLCanvasElement> {
    return document.querySelectorAll<HTMLCanvasElement>(
      `canvas[id^="renderer-mirror-"][id$="-${nodeId}"], canvas#renderer-mirror-${nodeId}`,
    );
  }

  private rendererVideos(nodeId: string): NodeListOf<HTMLVideoElement> {
    return document.querySelectorAll<HTMLVideoElement>(
      `video[id^="renderer-stream-"][id$="-${nodeId}"], video#renderer-stream-${nodeId}`,
    );
  }

  private mountRendererStream(nodeId: string, stream: MediaStream | null): void {
    for (const video of this.rendererVideos(nodeId)) {
      if (video.srcObject === stream) continue;
      video.srcObject = stream;
      if (stream) void video.play();
    }
  }


  private drawRendererFrame(nodeId: string, frame: NativeOutputImage): boolean {
    const startedAt = performance.now();
    const source = this.nativePreviewCanvas ??= document.createElement('canvas');
    if (source.width !== frame.width) source.width = frame.width;
    if (source.height !== frame.height) source.height = frame.height;
    const sourceContext = source.getContext('2d');
    if (!sourceContext) throw new Error('Cannot create native renderer preview canvas');
    const pixels = new Uint8ClampedArray(frame.rgba);
    sourceContext.putImageData(new ImageData(pixels, frame.width, frame.height), 0, 0);
    const imageDataMs = performance.now() - startedAt;
    const mirrorStartedAt = performance.now();
    const presented = this.drawRendererSource(nodeId, source);
    const mirrorDrawMs = performance.now() - mirrorStartedAt;
    this.recordRendererDraw(frame, imageDataMs, mirrorDrawMs, performance.now() - startedAt);
    return presented;
  }

  private recordRendererDraw(
    frame: NativeOutputImage,
    imageDataMs: number,
    mirrorDrawMs: number,
    totalMs: number,
  ): void {
    const now = performance.now();
    const metrics = this.rendererDrawMetrics;
    if (metrics.frames === 0) metrics.windowAt = now;
    metrics.frames += 1;
    metrics.imageDataMs += imageDataMs;
    metrics.mirrorDrawMs += mirrorDrawMs;
    metrics.totalMs += totalMs;
    metrics.maxMs = Math.max(metrics.maxMs, totalMs);
    const windowMs = now - metrics.windowAt;
    if (windowMs < 1000) return;
    runtimeLog('native', 'info', 'renderer-draw-perf', {
      rate: Number((metrics.frames * 1000 / windowMs).toFixed(1)),
      avgImageDataMs: Number((metrics.imageDataMs / Math.max(metrics.frames, 1)).toFixed(3)),
      avgMirrorDrawMs: Number((metrics.mirrorDrawMs / Math.max(metrics.frames, 1)).toFixed(3)),
      avgTotalMs: Number((metrics.totalMs / Math.max(metrics.frames, 1)).toFixed(3)),
      maxMs: Number(metrics.maxMs.toFixed(3)),
      width: frame.width,
      height: frame.height,
    });
    this.rendererDrawMetrics = {
      windowAt: now,
      frames: 0,
      imageDataMs: 0,
      mirrorDrawMs: 0,
      totalMs: 0,
      maxMs: 0,
    };
  }

  private drawRendererSource(nodeId: string, source: CanvasImageSource): boolean {
    let presented = false;
    for (const mirror of this.rendererMirrors(nodeId)) {
      const context = mirror.getContext('2d');
      if (!context) continue;
      context.clearRect(0, 0, mirror.width, mirror.height);
      context.drawImage(source, 0, 0, mirror.width, mirror.height);
      presented = true;
    }
    return presented;
  }

  private handleRendererRemount = (): void => {
    if (this.rendererStream) {
      this.mountRendererStream(this.rendererStream.nodeId, this.rendererStream.stream);
    }
    if (this.lastNativeRendererFrame) {
      this.drawRendererFrame(this.lastNativeRendererFrame.nodeId, this.lastNativeRendererFrame.frame);
    }
    this.runtime?.requestPreviewRefresh?.();
  };

  private recordRendererPresentation(nodeId: string, now = performance.now()): void {
    const metric = this.rendererFrameMetrics.get(nodeId) ?? { windowAt: now, frames: 0 };
    metric.frames += 1;
    const elapsed = now - metric.windowAt;
    if (elapsed >= 500) {
      useGraphStore.getState().setRendererFps(nodeId, metric.frames * 1000 / elapsed);
      metric.windowAt = now;
      metric.frames = 0;
    }
    this.rendererFrameMetrics.set(nodeId, metric);
  }

  private resetNativeFrameMetrics(now = performance.now()): void {
    this.nativeStartedAt = now;
    this.nativeFpsWindowAt = now;
    this.nativeFpsWindowFrame = 0;
    this.nativeFps = 0;
    this.lastUiFrameAt = Number.NEGATIVE_INFINITY;
  }

  private handleFrame(frame: { frame: number; time: number; fps: number }): void {
    const now = performance.now();
    if (now - this.lastUiFrameAt < 100) return;
    this.lastUiFrameAt = now;
    useGraphStore.setState({
      fps: frame.fps,
      currentTime: frame.time,
      currentFrame: frame.frame,
    });
  }

  private handleOutputSize(nodeId: string, width: number, height: number): void {
    const store = useGraphStore.getState();
    const node = store.nodes.find((candidate) => candidate.id === nodeId);
    if (!node) return;
    if (node.data.type === 'input' && node.data.inputMode === 'video') {
      if (
        node.data.imageWidth === width
        && node.data.imageHeight === height
        && node.data.resolvedWidth === width
        && node.data.resolvedHeight === height
      ) return;
      store.updateNodeData(nodeId, {
        imageWidth: width,
        imageHeight: height,
        resolvedWidth: width,
        resolvedHeight: height,
      });
      return;
    }
    if (node.data.resolvedWidth === width && node.data.resolvedHeight === height) return;
    store.updateNodeData(nodeId, { resolvedWidth: width, resolvedHeight: height });
  }

  private handleBackend(nodeId: string, backend: 'webgpu' | 'wasm' | 'native'): void {
    const store = useGraphStore.getState();
    const node = store.nodes.find((candidate) => candidate.id === nodeId);
    if (node?.data.onnxBackend !== backend) store.updateNodeData(nodeId, { onnxBackend: backend });
  }

  private handleError(nodeId: string | null, error: string): void {
    const store = useGraphStore.getState();
    store.setNodeError(nodeId ?? store.selectedNodeId ?? store.activeRendererId ?? 'runtime', error);
  }

  private enqueue(operation: () => Promise<void>): void {
    this.operations = this.operations.then(operation).catch((error: unknown) => {
      this.handleError(null, error instanceof Error ? error.message : String(error));
    });
  }
}

type RuntimeNodeSnapshot = {
  id: string;
  position: { x: number; y: number };
  data: Record<string, unknown>;
};

function nodesChangedForRuntime(
  current: readonly RuntimeNodeSnapshot[],
  previous: readonly RuntimeNodeSnapshot[],
): boolean {
  if (current.length !== previous.length) return true;
  return current.some((node, index) => {
    const oldNode = previous[index];
    if (
      !oldNode
      || node.id !== oldNode.id
      || node.position.x !== oldNode.position.x
      || node.position.y !== oldNode.position.y
    ) return true;
    const keys = new Set([...Object.keys(node.data), ...Object.keys(oldNode.data)]);
    keys.delete('expanded');
    return Array.from(keys).some((key) => node.data[key] !== oldNode.data[key]);
  });
}

export async function listAvailableVideoDevices(): Promise<Array<{ id: string; label: string }>> {
  if (await checkIsTauri()) return await new NativePipelineRuntime().listVideoDevices();
  const devices = await navigator.mediaDevices?.enumerateDevices?.() ?? [];
  return devices
    .filter((device) => device.kind === 'videoinput')
    .map((device, index) => ({
      id: device.deviceId,
      label: device.label || `Camera ${index + 1}`,
    }));
}
