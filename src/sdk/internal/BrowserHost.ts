import type { Edge, Node } from '@xyflow/react';
import type { ShaderNodeData } from '../../types';
import type {
  PlayerHost,
  PlayerHostEvents,
  RuntimeVideoDevice,
} from './hostTypes';
import type {
  BrowserWorkerRequest,
  BrowserWorkerRequestPayload,
  BrowserWorkerResponse,
  BrowserWorkerVideoFrame,
} from '../browserWorkerProtocol';
import { requireSdk } from '../runtime';

type PendingRequest = {
  resolve: (value: unknown) => void;
  reject: (error: Error) => void;
};

type BrowserVideoSource = {
  nodeId: string;
  key: string;
  video: HTMLVideoElement;
  stream: MediaStream | null;
  frameCallback: number | null;
  fallbackTimer: number | null;
  capturePending: boolean;
  inFlightFrameId: number | null;
  active: boolean;
};

type HostGraphSnapshot = {
  nodes: Node<ShaderNodeData>[];
  edges: Edge[];
};

type BrowserResourceIntent =
  | { type: 'attach-video'; nodeId: string; key: string; kind: 'file' | 'camera'; source: string; looping: boolean; playbackRate: number }
  | { type: 'update-video'; nodeId: string; key: string; looping: boolean; playbackRate: number }
  | { type: 'detach-video'; nodeId: string };

/** Main-thread projection of the Rust/WASM runtime hosted in a dedicated worker. */
export class BrowserHost implements PlayerHost {
  private worker: Worker | null = null;
  private nextId = 1;
  private nextVideoFrameId = 1;
  private readonly pending = new Map<number, PendingRequest>();
  private readonly callbacks: PlayerHostEvents;
  private readonly videoSources = new Map<string, BrowserVideoSource>();
  private lastResourceGraph: HostGraphSnapshot | null = null;

  constructor(callbacks: PlayerHostEvents = {}) {
    this.callbacks = callbacks;
  }

  async initialize(canvas: HTMLCanvasElement): Promise<void> {
    if (this.worker) return;
    if (typeof Worker === 'undefined' || typeof canvas.transferControlToOffscreen !== 'function') {
      throw new Error('Browser runtime requires Worker and OffscreenCanvas support');
    }
    const worker = new Worker(new URL('../BrowserRuntimeWorker.ts', import.meta.url), {
      type: 'module',
      name: 'open-quartz-runtime',
    });
    worker.onmessage = this.handleMessage;
    worker.onerror = (event) => {
      this.rejectAll(new Error(event.message || 'Browser runtime worker failed'));
    };
    this.worker = worker;
    const offscreen = canvas.transferControlToOffscreen();
    await this.request({ type: 'initialize', canvas: offscreen }, [offscreen]);
  }

  async registerOnnxModel(modelId: string, buffer: ArrayBuffer): Promise<void> {
    await this.request(
      { type: 'register-onnx-model', modelId, buffer },
      [buffer],
    );
  }

  async play(nodes: Node<ShaderNodeData>[], edges: Edge[]): Promise<void> {
    await this.applyVideoIntents(nodes, edges);
    this.lastResourceGraph = cloneHostGraph(nodes, edges);
    await this.request({ type: 'play', nodes, edges });
  }

  async updateGraph(nodes: Node<ShaderNodeData>[], edges: Edge[]): Promise<void> {
    await this.applyVideoIntents(nodes, edges);
    this.lastResourceGraph = cloneHostGraph(nodes, edges);
    await this.request({ type: 'update-graph', nodes, edges });
  }

  async pause(): Promise<void> {
    for (const source of this.videoSources.values()) source.video.pause();
    await this.request({ type: 'pause' });
  }

  async resume(): Promise<void> {
    const sources = [...this.videoSources.values()];
    const playback = Promise.allSettled(sources.map((source) => source.video.play()));
    await this.request({ type: 'resume' });
    const results = await playback;
    results.forEach((result, index) => {
      if (result.status !== 'rejected') return;
      const error = result.reason;
      this.callbacks.onNodeError?.(
        sources[index].nodeId,
        error instanceof Error ? error.message : String(error),
      );
    });
  }

  async stop(): Promise<void> {
    this.disposeVideoSources();
    this.lastResourceGraph = null;
    if (this.worker) await this.request({ type: 'stop' });
  }
  setPreviewNode(nodeId: string | null): void {
    console.info('[oq:browser-host] set-preview-node', { nodeId });
    if (this.worker) void this.request({ type: 'set-preview', nodeId });
  }

  async captureScreenshot(nodeId: string): Promise<string | null> {
    return await this.request<string | null>({ type: 'capture', nodeId });
  }

  async listVideoDevices(): Promise<RuntimeVideoDevice[]> {
    const devices = await navigator.mediaDevices?.enumerateDevices?.() ?? [];
    return devices
      .filter((device) => device.kind === 'videoinput')
      .map((device, index) => ({
        id: device.deviceId,
        label: device.label || `Camera ${index + 1}`,
      }));
  }

  async close(): Promise<void> {
    const worker = this.worker;
    if (!worker) return;
    this.disposeVideoSources();
    this.lastResourceGraph = null;
    try {
      await this.request({ type: 'close' });
    } finally {
      worker.terminate();
      this.worker = null;
      this.rejectAll(new Error('Browser runtime worker closed'));
    }
  }

  private async applyVideoIntents(nodes: Node<ShaderNodeData>[], edges: Edge[]): Promise<void> {
    const graph = cloneHostGraph(nodes, edges);
    const plan = requireSdk().planHostResourceIntents<{
      intents: BrowserResourceIntent[];
    }>({
      host: 'browser',
      previousGraph: this.lastResourceGraph,
      graph,
    });
    for (const intent of plan.intents) {
      if (intent.type === 'update-video') {
        const source = this.videoSources.get(intent.nodeId);
        if (!source) continue;
        source.video.loop = intent.looping;
        source.video.playbackRate = intent.playbackRate;
        continue;
      }
      if (intent.type === 'detach-video') {
        const source = this.videoSources.get(intent.nodeId);
        if (!source) continue;
        this.disposeVideoSource(source);
        this.videoSources.delete(intent.nodeId);
        continue;
      }
      const node = nodes.find((candidate) => candidate.id === intent.nodeId);
      if (!node) continue;
      const previous = this.videoSources.get(intent.nodeId);
      const next = await this.createVideoSource(intent.nodeId, intent);
      next.video.loop = intent.looping;
      next.video.playbackRate = intent.playbackRate;
      this.videoSources.set(intent.nodeId, next);
      this.scheduleVideoCapture(intent.nodeId, next);
      if (previous) this.disposeVideoSource(previous);
    }
  }

  private async createVideoSource(
    nodeId: string,
    intent: Extract<BrowserResourceIntent, { type: 'attach-video' }>,
  ): Promise<BrowserVideoSource> {
    const video = document.createElement('video');
    video.muted = true;
    video.playsInline = true;
    video.autoplay = true;
    video.loop = intent.looping;
    video.playbackRate = intent.playbackRate;
    video.style.cssText = 'position:fixed;width:1px;height:1px;opacity:0;pointer-events:none';
    let stream: MediaStream | null = null;
    if (intent.kind === 'camera') {
      const videoConstraint: MediaTrackConstraints | boolean = intent.source !== 'default'
        ? { deviceId: { exact: intent.source } }
        : true;
      stream = await navigator.mediaDevices.getUserMedia({ video: videoConstraint, audio: false });
      video.srcObject = stream;
    } else {
      const url = intent.source;
      if (!url) throw new Error(`Video input ${nodeId} has no browser-loadable URL`);
      video.src = url;
    }
    document.body.append(video);
    const source: BrowserVideoSource = {
      nodeId,
      key: intent.key,
      video,
      stream,
      frameCallback: null,
      fallbackTimer: null,
      capturePending: false,
      inFlightFrameId: null,
      active: true,
    };
    video.addEventListener('loadedmetadata', () => {
      if (source.active && video.videoWidth > 0 && video.videoHeight > 0) {
        this.callbacks.onOutputSize?.(nodeId, video.videoWidth, video.videoHeight);
      }
    }, { once: true });
    try {
      await video.play();
      return source;
    } catch (error) {
      this.disposeVideoSource(source);
      throw error;
    }
  }

  private scheduleVideoCapture(nodeId: string, source: BrowserVideoSource): void {
    const capture = (): void => {
      if (!source.active) return;
      if (!source.capturePending && source.inFlightFrameId === null && !source.video.paused
        && source.video.readyState >= HTMLMediaElement.HAVE_CURRENT_DATA) {
        source.capturePending = true;
        void createImageBitmap(source.video).then((frame) => {
          source.capturePending = false;
          const worker = this.worker;
          if (!source.active || !worker) {
            frame.close();
            return;
          }
          const frameId = this.nextVideoFrameId++;
          source.inFlightFrameId = frameId;
          const message: BrowserWorkerVideoFrame = { type: 'video-frame', nodeId, frameId, frame };
          try {
            worker.postMessage(message, [frame]);
          } catch (error) {
            source.inFlightFrameId = null;
            frame.close();
            this.callbacks.onNodeError?.(
              nodeId,
              error instanceof Error ? error.message : String(error),
            );
          }
        }).catch((error: unknown) => {
          source.capturePending = false;
          this.callbacks.onNodeError?.(
            nodeId,
            error instanceof Error ? error.message : String(error),
          );
        });
      }
      if ('requestVideoFrameCallback' in source.video) {
        source.frameCallback = source.video.requestVideoFrameCallback(capture);
      }
    };
    if ('requestVideoFrameCallback' in source.video) {
      source.frameCallback = source.video.requestVideoFrameCallback(capture);
    } else {
      source.fallbackTimer = window.setInterval(capture, 33);
    }
  }

  private disposeVideoSource(source: BrowserVideoSource): void {
    source.active = false;
    if (source.frameCallback !== null && 'cancelVideoFrameCallback' in source.video) {
      source.video.cancelVideoFrameCallback(source.frameCallback);
    }
    if (source.fallbackTimer !== null) window.clearInterval(source.fallbackTimer);
    source.video.pause();
    source.video.srcObject = null;
    source.video.removeAttribute('src');
    source.video.remove();
    for (const track of source.stream?.getTracks() ?? []) track.stop();
  }

  private disposeVideoSources(): void {
    for (const source of this.videoSources.values()) this.disposeVideoSource(source);
    this.videoSources.clear();
  }

  private readonly handleMessage = (event: MessageEvent<BrowserWorkerResponse>): void => {
    const message = event.data;
    if ('id' in message) {
      const pending = this.pending.get(message.id);
      if (!pending) return;
      this.pending.delete(message.id);
      if (message.ok) pending.resolve(message.value);
      else pending.reject(new Error(message.error));
      return;
    }
    switch (message.type) {
      case 'frame':
        this.callbacks.onFrame?.(message);
        break;
      case 'video-frame-consumed': {
        const source = this.videoSources.get(message.nodeId);
        if (source?.inFlightFrameId === message.frameId) source.inFlightFrameId = null;
        break;
      }
      case 'output':
        this.drawOutputToMirrors(message.nodeId, message.dataUrl);
        this.callbacks.onOutput?.(message.nodeId, message.dataUrl);
        break;
      case 'output-size':
        this.callbacks.onOutputSize?.(message.nodeId, message.width, message.height);
        break;
      case 'output-data':
        this.callbacks.onOutputData?.(message.nodeId, message.data);
        break;
      case 'node-error':
        this.callbacks.onNodeError?.(message.nodeId, message.error);
        break;
      case 'backend':
        this.callbacks.onBackendDetected?.(message.nodeId, message.backend);
        break;
    }
  };

  private drawOutputToMirrors(nodeId: string, dataUrl: string): void {
    if (typeof document === 'undefined') return;
    const image = new Image();
    image.onload = () => {
      const mirrors = document.querySelectorAll<HTMLCanvasElement>(
        `canvas[id^="renderer-mirror-"][id$="-${nodeId}"], canvas#renderer-mirror-${nodeId}`,
      );
      let presented = false;
      for (const mirror of mirrors) {
        const context = mirror.getContext('2d');
        if (!context) continue;
        context.clearRect(0, 0, mirror.width, mirror.height);
        context.drawImage(image, 0, 0, mirror.width, mirror.height);
        presented = true;
      }
      if (presented) this.callbacks.onRendererPresented?.(nodeId);
    };
    image.src = dataUrl;
  }

  private async request<T = void>(
    message: BrowserWorkerRequestPayload,
    transfer: Transferable[] = [],
  ): Promise<T> {
    const worker = this.worker;
    if (!worker) throw new Error('Browser pipeline runtime is not initialized');
    const id = this.nextId++;
    const response = new Promise<T>((resolve, reject) => {
      this.pending.set(id, {
        resolve: (value) => resolve(value as T),
        reject,
      });
    });
    worker.postMessage({ ...message, id } as BrowserWorkerRequest, transfer);
    return await response;
  }

  private rejectAll(error: Error): void {
    for (const pending of this.pending.values()) pending.reject(error);
    this.pending.clear();
  }
}

function cloneHostGraph(
  nodes: Node<ShaderNodeData>[],
  edges: Edge[],
): HostGraphSnapshot {
  return {
    nodes: nodes.map((node) => ({ ...node, data: { ...node.data } })),
    edges: edges.map((edge) => ({ ...edge })),
  };
}
