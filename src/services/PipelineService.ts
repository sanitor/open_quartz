/** PipelineService projects Zustand intent onto the public SDK object graph. */
import {
  OpenQuartzClient,
  Player,
  Project,
  type PlayerEvents,
} from '../sdk';
import { useGraphStore } from '../store/useGraphStore';
import { runtimeLog } from '../sdk/runtimeLog';



interface NativeOutputImage {
  rgba: Uint8Array;
  width: number;
  height: number;
}
interface RendererRemountDetail {
  nodeId?: string;
  fullscreen?: boolean;
}

let captureOutput: ((nodeId: string) => Promise<string | null>) | null = null;

export class PipelineService {
  private readonly sdk = new OpenQuartzClient();
  private player: Player | null = null;
  private playerPromise: Promise<Player> | null = null;
  private project: Project | null = null;
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
  private rendererStream: { nodeId: string; video: HTMLVideoElement } | null = null;
  private readonly rendererFrameMetrics = new Map<string, { windowAt: number; frames: number }>();
  private rendererDrawMetrics = {
    windowAt: performance.now(),
    frames: 0,
    imageDataMs: 0,
    mirrorDrawMs: 0,
    totalMs: 0,
    maxMs: 0,
  };

  constructor() {}

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
          const player = await this.ensurePlayer(canvas);
          if (useGraphStore.getState().loopState !== 'playing') return;
          const current = useGraphStore.getState();
          player.setPreview(current.selectedNodeId ? player.output(current.selectedNodeId) : null);
          await player.play();
        });
      }

      if (state.loopState === 'paused' && previous.loopState === 'playing') {
        this.enqueue(async () => { await this.player?.pause(); });
      }

      if (state.loopState === 'playing' && previous.loopState === 'paused') {
        this.enqueue(async () => { await this.player?.resume(); });
      }

      if (state.loopState === 'stopped' && previous.loopState !== 'stopped') {
        useGraphStore.getState().clearOutputPreviews();
        this.player?.setPreview(null);
        this.enqueue(async () => { await this.player?.stop(); });
      }

      if (
        state.loopState === 'playing'
        && (state.edges !== previous.edges || nodesChangedForRuntime(state.nodes, previous.nodes))
      ) {
        this.project?.graph.replace(state.nodes, state.edges);
        this.enqueue(async () => { await this.player?.apply(); });
      }

      if (state.selectedNodeId !== previous.selectedNodeId) {
        runtimeLog('browser-host', 'info', 'preview-selection', {
          previous: previous.selectedNodeId,
          selected: state.selectedNodeId,
          playing: state.loopState,
          playerReady: this.player !== null,
        });
        if (state.loopState !== 'stopped') {
          const player = this.player;
          player?.setPreview(state.selectedNodeId ? player.output(state.selectedNodeId) : null);
          if (player) requestAnimationFrame(() => player.refreshPreview());
        }
      }
    });
  }

  detach(): void {
    this.unsub?.();
    this.unsub = null;
    this.generation += 1;
    captureOutput = null;
    const player = this.player;
    this.player = null;
    this.playerPromise = null;
    this.project = null;
    window.removeEventListener('renderer-remount', this.handleRendererRemount);
    this.rendererStream = null;
    useGraphStore.setState({ rendererStreamActive: {} });
    this.enqueue(async () => { await player?.close(); });
    this.lastNativeRendererFrame = null;
    this.rendererFrameMetrics.clear();
  }

  private async ensurePlayer(canvas: HTMLCanvasElement): Promise<Player> {
    if (this.player) return this.player;
    if (this.playerPromise) return await this.playerPromise;
    const current = useGraphStore.getState();
    this.project = new Project('Live composition', current.nodes, current.edges);
    const generation = this.generation;
    this.playerPromise = this.sdk.player(this.project, {
      canvas,
      events: this.playerEvents(),
    }).then((player) => {
      if (generation !== this.generation) {
        void player.close();
        throw new Error('Player initialization was superseded');
      }
      this.player = player;
      captureOutput = async (nodeId) => await player.output(nodeId).capture();
      return player;
    }).finally(() => {
      this.playerPromise = null;
    });
    return await this.playerPromise;
  }

  private playerEvents(): PlayerEvents {
    return {
      onFrame: (frame) => {
        if (Number.isFinite(frame.time) && Number.isFinite(frame.fps)) {
          this.handleFrame(frame);
          return;
        }
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
      onRendererStream: (nodeId, video) => {
        this.rendererStream = video ? { nodeId, video } : null;
        useGraphStore.getState().setRendererStreamActive(nodeId, video !== null);
        this.mountRendererStream(nodeId, video);
      },
      onRendererVideoFrame: (nodeId) => this.recordRendererPresentation(nodeId),
      onRendererCadence: (nodeId, cadence) => {
        useGraphStore.getState().setRendererCadence(nodeId, {
          graphFps: this.nativeFps,
          ...cadence,
        });
      },
      onRendererPresented: (nodeId) => this.recordRendererPresentation(nodeId),
      onOutput: (nodeId, dataUrl) => this.handleOutput(nodeId, dataUrl),
      onNodeError: (nodeId, error) => this.handleError(nodeId, error),
      onOutputSize: (nodeId, width, height) => this.handleOutputSize(nodeId, width, height),
      onOutputData: (nodeId, data) => useGraphStore.getState().setOutputData(nodeId, data),
      onBackendDetected: (nodeId, backend) => this.handleBackend(nodeId, backend),
      onNativeBackendDetected: (nodeId, backend) => {
        const store = useGraphStore.getState();
        store.updateNodeData(nodeId, { onnxBackend: 'native', onnxNativeBackend: backend });
      },
    };
  }

  private rendererMirrors(nodeId: string): NodeListOf<HTMLCanvasElement> {
    return document.querySelectorAll<HTMLCanvasElement>(
      `canvas[id^="renderer-mirror-"][id$="-${nodeId}"], canvas#renderer-mirror-${nodeId}`,
    );
  }


  private mountRendererStream(nodeId: string, video: HTMLVideoElement | null, preferFullscreen = true): void {
    if (!video) return;
    const standardTargets = [
      document.getElementById(`renderer-stream-slot-sidepanel-${nodeId}`),
      document.getElementById(`renderer-stream-slot-node-${nodeId}`),
    ];
    const target = (preferFullscreen
      ? [document.getElementById(`renderer-stream-slot-fullscreen-${nodeId}`), ...standardTargets]
      : standardTargets
    ).find((slot) => slot !== null);
    if (!target || video.parentElement === target) return;
    video.style.cssText = 'width:100%;height:100%;object-fit:contain;display:block';
    target.replaceChildren(video);
    void video.play().catch((error: unknown) => {
      runtimeLog('native', 'warn', 'renderer-stream-resume-failed', {
        nodeId,
        error: error instanceof Error ? error.message : String(error),
      });
    });
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

  private handleRendererRemount = (event: Event): void => {
    const detail = (event as CustomEvent<RendererRemountDetail>).detail;
    if (this.rendererStream && (!detail?.nodeId || detail.nodeId === this.rendererStream.nodeId)) {
      this.mountRendererStream(
        this.rendererStream.nodeId,
        this.rendererStream.video,
        detail?.fullscreen !== false,
      );
    }
    if (this.lastNativeRendererFrame) {
      this.drawRendererFrame(this.lastNativeRendererFrame.nodeId, this.lastNativeRendererFrame.frame);
    }
    this.player?.refreshPreview();
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

  private handleOutput(nodeId: string, dataUrl: string): void {
    if (useGraphStore.getState().loopState === 'stopped') return;
    useGraphStore.getState().setOutputPreview(nodeId, dataUrl);
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

export async function capturePlayerOutput(nodeId: string): Promise<string | null> {
  return await captureOutput?.(nodeId) ?? null;
}

export async function listAvailableVideoDevices(): Promise<Array<{ id: string; label: string }>> {
  return await new OpenQuartzClient().listVideoDevices();
}
