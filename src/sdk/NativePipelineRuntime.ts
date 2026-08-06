import type { Edge, Node } from '@xyflow/react';
import { ONNX_CATALOG, type OnnxTask } from '../catalog/onnxCatalog';
import type { ShaderNodeData } from '../types';
import type { EngineEvent } from './contract';
import { runtimeLog } from './runtimeLog';

export interface NativeRuntimeInfo {
  adapterName: string;
  backend: string;
  deviceType: string;
  outputMode: 'embedded-readback' | 'webview-texture-stream';
  nativeOnnxCpu: boolean;
  nativeOnnxDirectMl: boolean;
  sharedOnnxWgpuDevice: boolean;
  nativeVideo: boolean;
  videoDataPath: 'cpu-copy' | 'd3d12va-p010-zero-copy' | 'external-frame/no-cpu-readback' | 'shared-gpu';
  tensorDataPath: 'cpu-copy' | 'external-frame/no-cpu-readback' | 'shared-gpu';
  sharedTexture: boolean;
}

export interface NativeOnnxCapabilities {
  cpu: boolean;
  directMl: boolean;
  sharedWgpuDevice: boolean;
}

export interface NativeVideoDevice {
  id: string;
  label: string;
}

export interface NativeOnnxSessionInfo {
  inputNames: string[];
  outputNames: string[];
  backend: 'cpu' | 'directml' | 'directml+cpu';
}

export interface NativeFrameRendered {
  frame: number;
  revision: number;
  outputNodeId: string;
  width: number;
  height: number;
}

export interface NativeOutputImage {
  rgba: Uint8Array;
  width: number;
  height: number;
}

export interface NativeOutputEvent {
  nodeId: string;
  width: number;
  height: number;
  backend: 'cpu' | 'directml' | 'directml+cpu';
  data?: unknown;
}

export interface NativeRuntimeCallbacks {
  onFrame?: (frame: NativeFrameRendered) => void;
  onRendererFrame?: (nodeId: string, frame: NativeOutputImage) => void;
  onRendererStream?: (nodeId: string, video: HTMLVideoElement | null) => void;
  onRendererVideoFrame?: (nodeId: string) => void;
  onError?: (error: string) => void;
  onOutput?: (nodeId: string, dataUrl: string) => void;
  onOutputSize?: (nodeId: string, width: number, height: number) => void;
  onOutputData?: (nodeId: string, data: unknown) => void;
  onBackendDetected?: (nodeId: string, backend: 'native') => void;
  onNativeBackendDetected?: (nodeId: string, backend: NativeOutputEvent['backend']) => void;
}

export type NativeInvokeArgs = Record<string, unknown> | number[] | ArrayBuffer | Uint8Array;
export type NativeInvokeOptions = { headers: HeadersInit };
export type NativeInvoke = <T>(
  command: string,
  args?: NativeInvokeArgs,
  options?: NativeInvokeOptions,
) => Promise<T>;
export type NativeListen = <T>(
  event: string,
  handler: (event: { payload: T }) => void,
) => Promise<() => void>;

export interface NativeTauriBridge {
  invoke: NativeInvoke;
  listen: NativeListen;
}

async function loadDefaultBridge(): Promise<NativeTauriBridge> {
  const [{ invoke }, { listen }] = await Promise.all([
    import('@tauri-apps/api/core'),
    import('@tauri-apps/api/event'),
  ]);
  return { invoke, listen };
}

interface ExperimentalTextureStreamWebView {
  getTextureStream(streamId: string): Promise<MediaStream> | MediaStream;
}

function textureStreamWebView(): ExperimentalTextureStreamWebView | null {
  const chrome = (window as Window & {
    chrome?: { webview?: Partial<ExperimentalTextureStreamWebView> };
  }).chrome;
  return typeof chrome?.webview?.getTextureStream === 'function'
    ? chrome.webview as ExperimentalTextureStreamWebView
    : null;
}


/**
 * Low-frequency Tauri control adapter. The Rust render thread owns frame timing
 * and GPU submission; this class never sends a per-frame command or pixel payload.
 */
export class NativePipelineRuntime {
  readonly frameScheduling = 'runtime' as const;
  private bridge: NativeTauriBridge | null;
  private readonly bridgeLoader: () => Promise<NativeTauriBridge>;
  private readonly callbacks: NativeRuntimeCallbacks;
  private readonly imageResources = new Map<string, string>();
  private readonly videoResources = new Map<string, string>();
  private readonly onnxResources = new Map<string, string>();
  private unlisten: Array<() => void> = [];
  private initialized = false;
  private closed = false;
  private previewNodeId: string | null = null;
  private lastRendererNodeId: string | null = null;
  private previewPending = false;
  private nextPreviewAt = 0;
  private rendererReadbackPending = false;
  private queuedRendererReadbackNodeId: string | null = null;
  private textureStreamVideo: HTMLVideoElement | null = null;
  private textureStream: MediaStream | null = null;
  private textureStreamMode = false;
  private textureStreamFrameMetrics = {
    windowAt: performance.now(),
    frames: 0,
    firstMediaTime: 0,
    lastMediaTime: 0,
    totalProcessingMs: 0,
    maxIntervalMs: 0,
    lastCallbackAt: 0,
  };
  private textureStreamSupported = false;
  private textureStreamStartPending = false;
  private textureStreamFrameCallback: number | null = null;
  private textureStreamRetryAt = 0;
  private rendererReadbackMetrics = {
    windowAt: performance.now(),
    completed: 0,
    queued: 0,
    totalMs: 0,
    maxMs: 0,
    bytes: 0,
  };


  constructor(
    callbacks: NativeRuntimeCallbacks = {},
    bridge?: NativeTauriBridge,
    bridgeLoader: () => Promise<NativeTauriBridge> = loadDefaultBridge,
  ) {
    this.callbacks = callbacks;
    this.bridge = bridge ?? null;
    this.bridgeLoader = bridgeLoader;
  }

  async initialize(_canvas?: HTMLCanvasElement): Promise<NativeRuntimeInfo> {
    if (this.closed) throw new Error('Native pipeline runtime is closed');
    if (this.initialized) throw new Error('Native pipeline runtime is already initialized');
    const bridge = await this.getBridge();
    const webview = textureStreamWebView();
    this.unlisten = await Promise.all([
      bridge.listen<NativeFrameRendered>('native-runtime-frame', ({ payload }) => {
        this.callbacks.onFrame?.(payload);
        this.callbacks.onOutputSize?.(
          payload.outputNodeId,
          payload.width,
          payload.height,
        );
        this.lastRendererNodeId = payload.outputNodeId;
        if (!this.presentTextureStreamFrame(payload.outputNodeId)) {
          if (this.textureStreamSupported && webview) {
            void this.startTextureStream(webview, bridge);
          }
          this.scheduleRendererReadback(payload.outputNodeId);
        }
        if (this.previewNodeId !== payload.outputNodeId) this.schedulePreviewReadback();
      }),
      bridge.listen<string>('native-runtime-error', ({ payload }) => {
        this.callbacks.onError?.(payload);
      }),
      bridge.listen<string>('native-runtime-presentation-fallback', ({ payload }) => {
        this.releaseTextureStream();
        this.callbacks.onError?.(payload);
        if (this.lastRendererNodeId) this.scheduleRendererReadback(this.lastRendererNodeId);
      }),
      bridge.listen<NativeOutputEvent>('native-runtime-output', ({ payload }) => {
        this.callbacks.onOutputSize?.(payload.nodeId, payload.width, payload.height);
        if (payload.data !== undefined) this.callbacks.onOutputData?.(payload.nodeId, payload.data);
        this.callbacks.onBackendDetected?.(payload.nodeId, 'native');
        this.callbacks.onNativeBackendDetected?.(payload.nodeId, payload.backend);
        if (payload.nodeId === this.previewNodeId) this.schedulePreviewReadback();
      }),
    ]);
    try {
      const info = await bridge.invoke<NativeRuntimeInfo>('native_gpu_initialize');
      this.initialized = true;
      if (info.outputMode === 'webview-texture-stream' && webview) {
        this.textureStreamSupported = true;
        runtimeLog('native', 'info', 'texture-stream-supported', {
          phase: 'deferred-until-native-frame',
        });
      }
      return info;
    } catch (error) {
      this.releaseListeners();
      throw error;
    }
  }


  async setGraph(nodes: Node<ShaderNodeData>[], edges: Edge[]): Promise<number> {
    const revision = await this.invoke<number>('native_gpu_set_graph', {
      graphJson: JSON.stringify({ nodes: stripGraphResourcePayloads(nodes), edges }),
    });
    await this.syncVideoResources(nodes);
    await this.syncImageResources(nodes);
    await this.syncOnnxResources(nodes);
    return revision;
  }

  async play(nodes: Node<ShaderNodeData>[], edges: Edge[]): Promise<void> {
    await this.setGraph(nodes, edges);
    await this.invoke<void>('native_gpu_play');
  }

  async updateGraph(nodes: Node<ShaderNodeData>[], edges: Edge[]): Promise<number> {
    const revision = await this.setGraph(nodes, edges);
    await this.invoke<void>('native_gpu_resume');
    return revision;
  }

  async pause(): Promise<void> {
    await this.invoke<void>('native_gpu_pause');
  }

  async resume(): Promise<void> {
    await this.invoke<void>('native_gpu_resume');
  }

  async stop(): Promise<void> {
    await this.invoke<void>('native_gpu_stop');
  }

  async setMouse(mouse: Float32Array): Promise<void> {
    if (mouse.length !== 4) throw new Error('Native mouse state must contain exactly 4 values');
    await this.invoke<void>('native_gpu_set_mouse', { mouse: Array.from(mouse) });
  }

  async uploadImage(
    nodeId: string,
    rgba: Uint8Array,
    width: number,
    height: number,
  ): Promise<void> {
    if (rgba.byteLength !== width * height * 4) {
      throw new Error(`RGBA byte length ${rgba.byteLength} does not match ${width}x${height}`);
    }
    await this.invoke<void>('native_gpu_upload_image', rgba, {
      headers: {
        'x-open-quartz-node-id': nodeId,
        'x-open-quartz-width': String(width),
        'x-open-quartz-height': String(height),
      },
    });
  }

  async removeTexture(nodeId: string): Promise<void> {
    await this.invoke<void>('native_gpu_remove_texture', { nodeId });
  }

  async attachVideo(
    nodeId: string,
    kind: 'file' | 'camera',
    source: string,
    looping = true,
    playbackRate = 1,
  ): Promise<{ width: number; height: number; fps: number; decoder: string }> {
    return await this.invoke('native_gpu_attach_video', {
      nodeId,
      kind,
      source,
      looping,
      playbackRate,
    });
  }

  async detachVideo(nodeId: string): Promise<void> {
    await this.invoke<void>('native_gpu_detach_video', { nodeId });
  }

  async listVideoDevices(): Promise<NativeVideoDevice[]> {
    const bridge = await this.getBridge();
    return await bridge.invoke<NativeVideoDevice[]>('native_video_devices');
  }

  async readOutput(nodeId: string): Promise<NativeOutputImage> {
    const response = await this.invoke<ArrayBuffer | Uint8Array>('native_gpu_read_output', {
      nodeId,
    });
    return decodeOutputImage(response);
  }

  async readPreview(nodeId: string, maxDimension = 960): Promise<NativeOutputImage> {
    const response = await this.invoke<ArrayBuffer | Uint8Array>('native_gpu_read_preview', {
      nodeId,
      maxDimension,
    });
    return decodeOutputImage(response);
  }

  async captureScreenshot(nodeId: string): Promise<string> {
    return outputImageToDataUrl(await this.readOutput(nodeId));
  }

  setPreviewNode(nodeId: string | null): void {
    runtimeLog('native', 'info', 'set-preview-node', {
      requested: nodeId,
      lastRendererNodeId: this.lastRendererNodeId,
      previewPending: this.previewPending,
      rendererReadbackPending: this.rendererReadbackPending,
    });
    this.previewNodeId = nodeId;
    this.nextPreviewAt = 0;
  }

  requestPreviewRefresh(): void {
    runtimeLog('native', 'debug', 'request-preview-refresh', {
      previewNodeId: this.previewNodeId,
      lastRendererNodeId: this.lastRendererNodeId,
    });
    if (this.lastRendererNodeId) this.scheduleRendererReadback(this.lastRendererNodeId);
    if (this.previewNodeId && this.previewNodeId !== this.lastRendererNodeId) {
      this.schedulePreviewReadback();
    }
  }

  async onnxCapabilities(): Promise<NativeOnnxCapabilities> {
    return await this.invoke<NativeOnnxCapabilities>('native_onnx_capabilities');
  }

  async loadOnnxModel(
    nodeId: string,
    modelId: string,
    task: OnnxTask,
    options: {
      modelPath?: string;
      targetSize?: number;
      scoreThreshold?: number;
      iouThreshold?: number;
      preferDirectMl?: boolean;
    } = {},
  ): Promise<NativeOnnxSessionInfo> {
    return await this.invoke<NativeOnnxSessionInfo>('native_onnx_load_model', {
      nodeId,
      modelId,
      options: {
        modelPath: options.modelPath,
        task,
        targetSize: options.targetSize ?? 640,
        scoreThreshold: options.scoreThreshold ?? 0.25,
        iouThreshold: options.iouThreshold ?? 0.45,
        preferDirectMl: options.preferDirectMl ?? true,
      },
    });
  }

  async unloadOnnxModel(nodeId: string): Promise<void> {
    await this.invoke<void>('native_onnx_unload_model', { nodeId });
  }

  async renderOnce(): Promise<NativeFrameRendered> {
    return await this.invoke<NativeFrameRendered>('native_gpu_render_once');
  }

  async drainEvents(): Promise<EngineEvent[]> {
    const json = await this.invoke<string>('native_gpu_drain_events');
    return JSON.parse(json) as EngineEvent[];
  }

  async close(): Promise<void> {
    if (this.closed) return;
    this.closed = true;
    try {
      if (this.initialized) {
        await this.invoke<void>('native_gpu_close', undefined, undefined, true);
      }
    } finally {
      this.initialized = false;
      this.lastRendererNodeId = null;
      this.releaseTextureStream();
      this.imageResources.clear();
      this.videoResources.clear();
      this.onnxResources.clear();
      this.releaseListeners();
    }
  }

  private async syncImageResources(nodes: Node<ShaderNodeData>[]): Promise<void> {
    const wanted = new Set<string>();
    for (const node of nodes) {
      if (node.data.type !== 'input' || node.data.inputDataType !== 'sampler2D') continue;
      const source = node.data.imageDataUrl ?? node.data.rawDataUrl;
      if (!source || node.data.inputMode === 'video') continue;
      wanted.add(node.id);
      const resourceKey = [
        source,
        node.data.fbFormat,
        node.data.fbWidth,
        node.data.fbHeight,
      ].join('|');
      if (this.imageResources.get(node.id) === resourceKey) continue;
      const image = node.data.imageDataUrl
        ? await decodeImageSource(node.data.imageDataUrl)
        : decodeRawRgba(
            await fetchResourceBytes(source),
            node.data.fbWidth,
            node.data.fbHeight,
            node.data.fbFormat,
          );
      await this.uploadImage(node.id, image.rgba, image.width, image.height);
      this.imageResources.set(node.id, resourceKey);
    }
    for (const nodeId of Array.from(this.imageResources.keys())) {
      if (wanted.has(nodeId)) continue;
      await this.removeTexture(nodeId);
      this.imageResources.delete(nodeId);
    }
  }

  private async syncVideoResources(nodes: Node<ShaderNodeData>[]): Promise<void> {
    const wanted = new Set<string>();
    for (const node of nodes) {
      if (node.data.type !== 'input' || node.data.inputMode !== 'video') continue;
      const kind = node.data.videoSourceType ?? 'file';
      const source = kind === 'camera'
        ? node.data.videoDeviceId
        : node.data.videoFilePath;
      if (!source) continue;
      wanted.add(node.id);
      const looping = node.data.videoLoop ?? true;
      const playbackRate = node.data.videoPlaybackRate ?? 1;
      const resourceKey = [kind, source, looping, playbackRate].join('|');
      if (this.videoResources.get(node.id) === resourceKey) continue;
      const info = await this.attachVideo(node.id, kind, source, looping, playbackRate);
      this.videoResources.set(node.id, resourceKey);
      this.callbacks.onOutputSize?.(node.id, info.width, info.height);
    }
    for (const nodeId of Array.from(this.videoResources.keys())) {
      if (wanted.has(nodeId)) continue;
      await this.detachVideo(nodeId);
      this.videoResources.delete(nodeId);
    }
  }

  private async syncOnnxResources(nodes: Node<ShaderNodeData>[]): Promise<void> {
    const wanted = new Set<string>();
    for (const node of nodes) {
      if (node.data.type !== 'onnx') continue;
      const catalogId = node.data.onnxCatalogId ?? node.data.onnxModelId;
      const modelId = node.data.onnxModelId ?? catalogId;
      if (!modelId) continue;
      const catalog = catalogId ? ONNX_CATALOG[catalogId] : undefined;
      const task = catalog?.task ?? 'generic';
      const params = node.data.onnxParams ?? {};
      const targetSize = Number(params.targetSize ?? node.data.onnxTargetSize ?? 640);
      const scoreThreshold = Number(params.scoreThreshold ?? node.data.onnxScoreThreshold ?? 0.25);
      const iouThreshold = Number(params.iouThreshold ?? node.data.onnxIouThreshold ?? 0.45);
      const modelPath = node.data.onnxCustomPath;
      const resourceKey = [modelId, modelPath, task, targetSize, scoreThreshold, iouThreshold].join('|');
      wanted.add(node.id);
      if (this.onnxResources.get(node.id) === resourceKey) continue;
      if (catalog && !modelPath) {
        await this.invoke<string>('download_model', {
          modelId,
          url: catalog.downloadUrl,
          expectedSize: catalog.fileSize,
        });
      }
      const info = await this.loadOnnxModel(node.id, modelId, task, {
        modelPath,
        targetSize,
        scoreThreshold,
        iouThreshold,
      });
      this.onnxResources.set(node.id, resourceKey);
      this.callbacks.onBackendDetected?.(node.id, 'native');
      this.callbacks.onNativeBackendDetected?.(node.id, info.backend);
    }
    for (const nodeId of Array.from(this.onnxResources.keys())) {
      if (wanted.has(nodeId)) continue;
      await this.unloadOnnxModel(nodeId);
      this.onnxResources.delete(nodeId);
    }
  }

  private recordRendererReadback(startedAt: number, frame: NativeOutputImage): void {
    const now = performance.now();
    const durationMs = now - startedAt;
    const metrics = this.rendererReadbackMetrics;
    if (metrics.completed === 0) metrics.windowAt = now;
    metrics.completed += 1;
    metrics.totalMs += durationMs;
    metrics.maxMs = Math.max(metrics.maxMs, durationMs);
    metrics.bytes += frame.rgba.byteLength;
    const windowMs = now - metrics.windowAt;
    if (windowMs < 1000) return;
    runtimeLog('native', 'info', 'renderer-readback-perf', {
      rate: Number((metrics.completed * 1000 / windowMs).toFixed(1)),
      avgMs: Number((metrics.totalMs / Math.max(metrics.completed, 1)).toFixed(3)),
      maxMs: Number(metrics.maxMs.toFixed(3)),
      queued: metrics.queued,
      throughputMiBs: Number((metrics.bytes * 1000 / windowMs / (1024 * 1024)).toFixed(1)),
      width: frame.width,
      height: frame.height,
    });
    this.rendererReadbackMetrics = {
      windowAt: now,
      completed: 0,
      queued: 0,
      totalMs: 0,
      maxMs: 0,
      bytes: 0,
    };
  }

  private scheduleRendererReadback(nodeId: string): void {
    if (this.textureStreamMode) return;
    if (!this.initialized) return;
    if (this.rendererReadbackPending) {
      this.queuedRendererReadbackNodeId = nodeId;
      this.rendererReadbackMetrics.queued += 1;
      runtimeLog('native', 'debug', 'renderer-readback-queued', { nodeId });
      return;
    }
    this.rendererReadbackPending = true;
    const startedAt = performance.now();
    void this.readPreview(nodeId, 960)
      .then((frame) => {
        if (this.textureStreamMode) return;
        this.recordRendererReadback(startedAt, frame);
        this.callbacks.onRendererFrame?.(nodeId, frame);
      })
      .catch((error: unknown) => {
        this.callbacks.onError?.(error instanceof Error ? error.message : String(error));
      })
      .finally(() => {
        this.rendererReadbackPending = false;
        const queuedNodeId = this.queuedRendererReadbackNodeId;
        this.queuedRendererReadbackNodeId = null;
        if (queuedNodeId) this.scheduleRendererReadback(queuedNodeId);
      });
  }

  private async startTextureStream(
    webview: ExperimentalTextureStreamWebView,
    bridge: NativeTauriBridge,
  ): Promise<void> {
    const now = performance.now();
    if (
      this.textureStreamStartPending
      || this.textureStreamMode
      || now < this.textureStreamRetryAt
    ) {
      return;
    }
    this.textureStreamStartPending = true;
    this.textureStreamMode = true;
    try {
      const streamPromise = Promise.resolve(
        webview.getTextureStream('open-quartz-renderer'),
      );
      await bridge.invoke('native_gpu_set_shared_texture_enabled', { enabled: true });
      await this.attachTextureStream(streamPromise);
      runtimeLog('native', 'info', 'texture-stream-started', {
        width: this.textureStreamVideo?.videoWidth,
        height: this.textureStreamVideo?.videoHeight,
      });
    } catch (error) {
      this.textureStreamMode = false;
      this.textureStreamRetryAt = performance.now() + 1000;
      await bridge.invoke('native_gpu_set_shared_texture_enabled', { enabled: false });
      runtimeLog('native', 'warn', 'texture-stream-start-failed', {
        error: error instanceof Error ? error.message : String(error),
        retryInMs: 1000,
      });
    } finally {
      this.textureStreamStartPending = false;
    }
  }

  private async attachTextureStream(streamPromise: Promise<MediaStream>): Promise<void> {
    try {
      const stream = await streamPromise;
      if (this.closed || !this.textureStreamMode) {
        for (const track of stream.getTracks()) track.stop();
        return;
      }
      const video = document.createElement('video');
      video.muted = true;
      video.autoplay = true;
      video.playsInline = true;
      video.style.cssText = 'position:fixed;width:1px;height:1px;opacity:0;pointer-events:none';
      video.srcObject = stream;
      document.body.append(video);
      this.textureStream = stream;
      this.textureStreamVideo = video;
      await video.play();
      if (this.lastRendererNodeId) this.presentTextureStreamFrame(this.lastRendererNodeId);
    } catch (error) {
      this.releaseTextureStream();
      this.textureStreamMode = false;
      throw error;
    }
  }

  private presentTextureStreamFrame(nodeId: string): boolean {
    const video = this.textureStreamVideo;
    if (!this.textureStreamMode || !video || video.readyState < HTMLMediaElement.HAVE_CURRENT_DATA) {
      return false;
    }
    this.callbacks.onRendererStream?.(nodeId, video);
    if (this.textureStreamFrameCallback === null) {
      const onFrame: VideoFrameRequestCallback = (_now, metadata) => {
        const callbackAt = performance.now();
        const metrics = this.textureStreamFrameMetrics;
        const interval = metrics.lastCallbackAt > 0 ? callbackAt - metrics.lastCallbackAt : 0;
        metrics.lastCallbackAt = callbackAt;
        metrics.frames += 1;
        metrics.lastMediaTime = metadata.mediaTime;
        if (metrics.frames === 1) metrics.firstMediaTime = metadata.mediaTime;
        metrics.totalProcessingMs += metadata.processingDuration ?? 0;
        metrics.maxIntervalMs = Math.max(metrics.maxIntervalMs, interval);
        const elapsed = callbackAt - metrics.windowAt;
        if (elapsed >= 1000) {
          runtimeLog('native', 'info', 'texture-stream-consumer-perf', {
            rate: Number((metrics.frames * 1000 / elapsed).toFixed(1)),
            mediaDelta: Number((metrics.lastMediaTime - metrics.firstMediaTime).toFixed(3)),
            avgProcessingMs: Number((metrics.totalProcessingMs / metrics.frames).toFixed(3)),
            maxIntervalMs: Number(metrics.maxIntervalMs.toFixed(3)),
          });
          this.textureStreamFrameMetrics = {
            windowAt: callbackAt,
            frames: 0,
            firstMediaTime: 0,
            lastMediaTime: 0,
            totalProcessingMs: 0,
            maxIntervalMs: 0,
            lastCallbackAt: callbackAt,
          };
        }
        this.callbacks.onRendererVideoFrame?.(nodeId);
        if (!this.textureStreamMode || !this.textureStreamVideo) {
          this.textureStreamFrameCallback = null;
          return;
        }
        this.textureStreamFrameCallback = this.textureStreamVideo.requestVideoFrameCallback(onFrame);
      };
      this.textureStreamFrameCallback = video.requestVideoFrameCallback(onFrame);
    }
    return true;

  }

  private releaseTextureStream(): void {
    this.textureStreamMode = false;
    if (this.textureStreamVideo && this.textureStreamFrameCallback !== null) {
      this.textureStreamVideo.cancelVideoFrameCallback(this.textureStreamFrameCallback);
    }
    this.textureStreamFrameCallback = null;
    if (this.lastRendererNodeId) {
      this.callbacks.onRendererStream?.(this.lastRendererNodeId, null);
    }
    if (this.textureStreamVideo) {
      this.textureStreamVideo.srcObject = null;
      this.textureStreamVideo.remove();
    }
    this.textureStreamVideo = null;
    if (this.textureStream) {
      for (const track of this.textureStream.getTracks()) track.stop();
    }
    this.textureStream = null;
  }

  private schedulePreviewReadback(): void {
    const now = performance.now();
    if (now < this.nextPreviewAt || this.previewPending || !this.previewNodeId || !this.initialized) return;
    this.nextPreviewAt = now + 200;
    this.previewPending = true;
    void this.refreshPreview()
      .catch((error: unknown) => {
        this.callbacks.onError?.(error instanceof Error ? error.message : String(error));
      })
      .finally(() => {
        this.previewPending = false;
      });
  }

  private async refreshPreview(): Promise<void> {
    const nodeId = this.previewNodeId;
    if (!nodeId) return;
    const output = await this.readPreview(nodeId, 960);
    if (this.previewNodeId !== nodeId) return;
    this.callbacks.onOutputSize?.(nodeId, output.width, output.height);
    this.callbacks.onOutput?.(nodeId, outputImageToDataUrl(output));
  }

  private async getBridge(): Promise<NativeTauriBridge> {
    this.bridge ??= await this.bridgeLoader();
    return this.bridge;
  }

  private async invoke<T>(
    command: string,
    args?: NativeInvokeArgs,
    options?: NativeInvokeOptions,
    allowClosed = false,
  ): Promise<T> {
    if (!this.initialized) throw new Error('Native pipeline runtime is not initialized');
    if (this.closed && !allowClosed) throw new Error('Native pipeline runtime is closed');
    const bridge = await this.getBridge();
    return await bridge.invoke<T>(command, args, options);
  }

  private releaseListeners(): void {
    for (const unlisten of this.unlisten.splice(0)) unlisten();
  }
}

function stripGraphResourcePayloads(
  nodes: Node<ShaderNodeData>[],
): Node<ShaderNodeData>[] {
  return nodes.map((node) => ({
    ...node,
    data: {
      ...node.data,
      imageDataUrl: undefined,
      rawDataUrl: undefined,
      videoUrl: undefined,
      videoFilePath: undefined,
      videoDeviceId: undefined,
    },
  }));
}

async function fetchResourceBytes(source: string): Promise<Uint8Array> {
  const response = await fetch(source);
  if (!response.ok) throw new Error(`Cannot load native image resource: HTTP ${response.status}`);
  return new Uint8Array(await response.arrayBuffer());
}

async function decodeImageSource(source: string): Promise<NativeOutputImage> {
  const image = new Image();
  image.decoding = 'async';
  image.src = source;
  await image.decode();
  const canvas = document.createElement('canvas');
  canvas.width = image.naturalWidth;
  canvas.height = image.naturalHeight;
  const context = canvas.getContext('2d');
  if (!context) throw new Error('Cannot create 2D canvas for native image upload');
  context.drawImage(image, 0, 0);
  const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
  return {
    rgba: new Uint8Array(pixels),
    width: canvas.width,
    height: canvas.height,
  };
}

function decodeRawRgba(
  rgba: Uint8Array,
  width: number | undefined,
  height: number | undefined,
  format: string | undefined,
): NativeOutputImage {
  if (!width || !height) throw new Error('Raw RGBA resource requires width and height');
  if (format && format !== 'rgba8') {
    throw new Error(`Native raw image upload does not support ${format}`);
  }
  const expected = width * height * 4;
  if (rgba.byteLength !== expected) {
    throw new Error(`Raw RGBA byte length ${rgba.byteLength} does not match ${width}x${height}`);
  }
  return { rgba, width, height };
}

function decodeOutputImage(response: ArrayBuffer | Uint8Array): NativeOutputImage {
  const bytes = response instanceof Uint8Array ? response : new Uint8Array(response);
  if (bytes.byteLength < 8) throw new Error('Native output payload is missing its header');
  const header = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const width = header.getUint32(0, true);
  const height = header.getUint32(4, true);
  const expected = 8 + width * height * 4;
  if (bytes.byteLength !== expected) {
    throw new Error(`Native output payload has ${bytes.byteLength} bytes; expected ${expected}`);
  }
  return { rgba: bytes.slice(8), width, height };
}

function outputImageToDataUrl(output: NativeOutputImage): string {
  const canvas = document.createElement('canvas');
  canvas.width = output.width;
  canvas.height = output.height;
  const context = canvas.getContext('2d');
  if (!context) throw new Error('Cannot create 2D canvas for native output');
  const pixels = new Uint8ClampedArray(output.rgba);
  context.putImageData(new ImageData(pixels, output.width, output.height), 0, 0);
  return canvas.toDataURL('image/png');
}
