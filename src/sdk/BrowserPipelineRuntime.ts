import type { Edge, Node } from '@xyflow/react';
import type { ShaderNodeData } from '../types';
import type {
  PipelineHostRuntime,
  PipelineRuntimeCallbacks,
  RuntimeVideoDevice,
} from './PipelineRuntime';
import type {
  BrowserWorkerRequest,
  BrowserWorkerRequestPayload,
  BrowserWorkerResponse,
} from './browserWorkerProtocol';

type PendingRequest = {
  resolve: (value: unknown) => void;
  reject: (error: Error) => void;
};

/** Main-thread projection of the Rust/WASM runtime hosted in a dedicated worker. */
export class BrowserPipelineRuntime implements PipelineHostRuntime {
  readonly frameScheduling = 'runtime' as const;
  private worker: Worker | null = null;
  private nextId = 1;
  private readonly pending = new Map<number, PendingRequest>();
  private readonly callbacks: PipelineRuntimeCallbacks;

  constructor(callbacks: PipelineRuntimeCallbacks = {}) {
    this.callbacks = callbacks;
  }

  async initialize(canvas: HTMLCanvasElement): Promise<void> {
    if (this.worker) return;
    if (typeof Worker === 'undefined' || typeof canvas.transferControlToOffscreen !== 'function') {
      throw new Error('Browser runtime requires Worker and OffscreenCanvas support');
    }
    const worker = new Worker(new URL('./BrowserRuntimeWorker.ts', import.meta.url), {
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

  async play(nodes: Node<ShaderNodeData>[], edges: Edge[]): Promise<void> {
    await this.request({ type: 'play', nodes, edges });
  }

  async updateGraph(nodes: Node<ShaderNodeData>[], edges: Edge[]): Promise<void> {
    await this.request({ type: 'update-graph', nodes, edges });
  }

  async pause(): Promise<void> { await this.request({ type: 'pause' }); }
  async resume(): Promise<void> { await this.request({ type: 'resume' }); }
  async stop(): Promise<void> {
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
    try {
      await this.request({ type: 'close' });
    } finally {
      worker.terminate();
      this.worker = null;
      this.rejectAll(new Error('Browser runtime worker closed'));
    }
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
