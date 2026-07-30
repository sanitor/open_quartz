import type { Edge, Node } from '@xyflow/react';
import type { ShaderNodeData } from '../types';
import type { EngineEvent } from './contract';

export interface NativeRuntimeInfo {
  adapterName: string;
  backend: string;
  deviceType: string;
  surfaceFormat: string;
  nativeOnnxCpu: boolean;
  nativeOnnxDirectMl: boolean;
  sharedOnnxWgpuDevice: boolean;
  nativeVideo: boolean;
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

export interface NativeRuntimeCallbacks {
  onFrame?: (frame: NativeFrameRendered) => void;
  onError?: (error: string) => void;
  onOutput?: (nodeId: string, dataUrl: string) => void;
  onOutputSize?: (nodeId: string, width: number, height: number) => void;
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
  private unlisten: Array<() => void> = [];
  private initialized = false;
  private closed = false;
  private previewNodeId: string | null = null;
  private previewPending = false;

  constructor(
    callbacks: NativeRuntimeCallbacks = {},
    bridge?: NativeTauriBridge,
    bridgeLoader: () => Promise<NativeTauriBridge> = loadDefaultBridge,
  ) {
    this.callbacks = callbacks;
    this.bridge = bridge ?? null;
    this.bridgeLoader = bridgeLoader;
  }

  async initialize(): Promise<NativeRuntimeInfo> {
    if (this.closed) throw new Error('Native pipeline runtime is closed');
    if (this.initialized) throw new Error('Native pipeline runtime is already initialized');
    const bridge = await this.getBridge();
    this.unlisten = await Promise.all([
      bridge.listen<NativeFrameRendered>('native-runtime-frame', ({ payload }) => {
        this.callbacks.onFrame?.(payload);
        this.callbacks.onOutputSize?.(
          payload.outputNodeId,
          payload.width,
          payload.height,
        );
        this.schedulePreviewReadback();
      }),
      bridge.listen<string>('native-runtime-error', ({ payload }) => {
        this.callbacks.onError?.(payload);
      }),
    ]);
    try {
      const info = await bridge.invoke<NativeRuntimeInfo>('native_gpu_initialize');
      this.initialized = true;
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
    return revision;
  }

  async play(nodes: Node<ShaderNodeData>[], edges: Edge[]): Promise<void> {
    await this.setGraph(nodes, edges);
    await this.invoke<void>('native_gpu_play');
  }

  async updateGraph(nodes: Node<ShaderNodeData>[], edges: Edge[]): Promise<number> {
    return await this.setGraph(nodes, edges);
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
    return this.invoke<NativeVideoDevice[]>('native_video_devices');
  }

  async readOutput(nodeId: string): Promise<NativeOutputImage> {
    const response = await this.invoke<ArrayBuffer | Uint8Array>('native_gpu_read_output', {
      nodeId,
    });
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

  async captureScreenshot(nodeId: string): Promise<string> {
    return outputImageToDataUrl(await this.readOutput(nodeId));
  }

  setPreviewNode(nodeId: string | null): void {
    this.previewNodeId = nodeId;
    this.schedulePreviewReadback();
  }

  async onnxCapabilities(): Promise<NativeOnnxCapabilities> {
    return await this.invoke<NativeOnnxCapabilities>('native_onnx_capabilities');
  }

  async loadOnnxModel(
    nodeId: string,
    modelId: string,
    preferDirectMl = true,
  ): Promise<NativeOnnxSessionInfo> {
    return await this.invoke<NativeOnnxSessionInfo>('native_onnx_load_model', {
      nodeId,
      modelId,
      preferDirectMl,
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
      this.previewNodeId = null;
      this.imageResources.clear();
      this.videoResources.clear();
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

  private schedulePreviewReadback(): void {
    if (!this.previewNodeId || this.previewPending || !this.initialized) return;
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
    const output = await this.readOutput(nodeId);
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
