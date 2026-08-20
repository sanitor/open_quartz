import { describe, expect, it, vi } from 'vitest';
import {
  NativeHost,
  type NativeFrameRendered,
  type NativeInvokeArgs,
  type NativeInvokeOptions,
  type NativeRuntimeInfo,
  type NativeTauriBridge,
} from '../../src/sdk/internal/NativeHost';

class FakeBridge implements NativeTauriBridge {
  readonly calls: Array<{
    command: string;
    args?: NativeInvokeArgs;
    options?: NativeInvokeOptions;
  }> = [];
  readonly handlers = new Map<string, (event: { payload: unknown }) => void>();
  readonly unlisten = vi.fn();

  async invoke<T>(
    command: string,
    args?: NativeInvokeArgs,
    options?: NativeInvokeOptions,
  ): Promise<T> {
    this.calls.push({ command, args, options });
    const value: unknown = command === 'native_host_resource_intents'
      ? JSON.stringify(planNativeHostResources(JSON.parse((args as Record<string, string>).requestJson)))
      : command === 'native_gpu_initialize'
      ? {
          adapterName: 'Test Adapter',
          backend: 'Dx12',
          deviceType: 'IntegratedGpu',
          outputMode: 'embedded-readback',
          nativeOnnxCpu: true,
          nativeOnnxDirectMl: true,
          sharedOnnxWgpuDevice: false,
          nativeVideo: true,
        } satisfies NativeRuntimeInfo
      : command === 'native_gpu_set_graph'
        ? 1
        : command === 'native_onnx_capabilities'
          ? { cpu: true, directMl: true, sharedWgpuDevice: false }
          : command === 'native_onnx_load_model'
            ? { inputNames: ['images'], outputNames: ['output0'], backend: 'directml+cpu' }
            : command === 'native_gpu_read_output' || command === 'native_gpu_read_preview'
              ? new Uint8Array([1, 0, 0, 0, 1, 0, 0, 0, 10, 20, 30, 255]).buffer
              : command === 'native_video_devices'
                ? [{ id: 'camera-0', label: 'Integrated Camera' }]
                : command === 'native_gpu_attach_video'
                  ? { width: 1920, height: 1080, fps: 30, decoder: 'ffmpeg-native' }
            : command === 'native_gpu_render_once'
              ? { frame: 1, revision: 1, outputNodeId: 'renderer', width: 64, height: 64 }
              : command === 'native_gpu_drain_events'
                ? '[{"type":"graph-ready","revision":1}]'
                : undefined;
    return value as T;
  }

  async listen<T>(event: string, handler: (event: { payload: T }) => void): Promise<() => void> {
    this.handlers.set(event, handler as (event: { payload: unknown }) => void);
    return this.unlisten;
  }

  emit<T>(event: string, payload: T): void {
    this.handlers.get(event)?.({ payload });
  }
}

function planNativeHostResources(request: {
  previousGraph?: { nodes: Array<Record<string, unknown>>; edges: unknown[] } | null;
  graph: { nodes: Array<Record<string, unknown>>; edges: unknown[] };
}): { graph: { nodes: Array<Record<string, unknown>>; edges: unknown[] }; intents: Array<Record<string, unknown>> } {
  const previous = summarizeResources(request.previousGraph?.nodes ?? []);
  const current = summarizeResources(request.graph.nodes);
  const intents: Array<Record<string, unknown>> = [];
  for (const [nodeId, resource] of previous.video) {
    if (current.video.get(nodeId)?.key !== resource.key) intents.push({ type: 'detach-video', nodeId });
  }
  for (const [nodeId, resource] of previous.image) {
    if (current.image.get(nodeId)?.key !== resource.key) intents.push({ type: 'remove-image', nodeId });
  }
  for (const [nodeId, resource] of previous.onnx) {
    if (current.onnx.get(nodeId)?.key !== resource.key) intents.push({ type: 'unload-onnx', nodeId });
  }
  for (const [nodeId, resource] of current.video) {
    if (previous.video.get(nodeId)?.key !== resource.key) intents.push({ type: 'attach-video', nodeId, ...resource });
  }
  for (const [nodeId, resource] of current.image) {
    if (previous.image.get(nodeId)?.key !== resource.key) intents.push({ type: 'upload-image', nodeId, ...resource });
  }
  for (const [nodeId, resource] of current.onnx) {
    if (previous.onnx.get(nodeId)?.key !== resource.key) intents.push({ type: 'load-onnx', nodeId, ...resource });
  }
  return {
    graph: {
      nodes: request.graph.nodes.map((node) => ({
        ...node,
        data: stripResourceData(node.data as Record<string, unknown> | undefined),
      })),
      edges: request.graph.edges,
    },
    intents,
  };
}

function summarizeResources(nodes: Array<Record<string, unknown>>): {
  video: Map<string, Record<string, unknown> & { key: string }>;
  image: Map<string, Record<string, unknown> & { key: string }>;
  onnx: Map<string, Record<string, unknown> & { key: string }>;
} {
  const video = new Map<string, Record<string, unknown> & { key: string }>();
  const image = new Map<string, Record<string, unknown> & { key: string }>();
  const onnx = new Map<string, Record<string, unknown> & { key: string }>();
  for (const node of nodes) {
    const nodeId = String(node.id);
    const data = node.data as Record<string, unknown> | undefined;
    if (!data) continue;
    if (data.type === 'input' && data.inputMode === 'video') {
      const kind = data.videoSourceType === 'camera' ? 'camera' : 'file';
      const source = kind === 'camera' ? data.videoDeviceId : data.videoFilePath;
      if (typeof source !== 'string') continue;
      const looping = data.videoLoop ?? true;
      const playbackRate = data.videoPlaybackRate ?? 1;
      video.set(nodeId, {
        key: [kind, source, looping, playbackRate].join('|'),
        kind,
        source,
        looping,
        playbackRate,
      });
    } else if (data.type === 'input' && data.inputDataType === 'sampler2D') {
      const raw = typeof data.rawDataUrl === 'string' ? data.rawDataUrl : null;
      const encoded = typeof data.imageDataUrl === 'string' ? data.imageDataUrl : null;
      if (!raw && !encoded) continue;
      image.set(nodeId, {
        key: [encoded ?? raw, data.fbFormat, data.fbWidth, data.fbHeight].join('|'),
        source: encoded
          ? { kind: 'encoded', source: encoded }
          : { kind: 'raw', source: raw, format: data.fbFormat, width: data.fbWidth, height: data.fbHeight },
      });
    } else if (data.type === 'onnx') {
      const modelId = data.onnxModelId ?? data.onnxCatalogId;
      if (typeof modelId !== 'string') continue;
      const params = data.onnxParams as Record<string, unknown> | undefined;
      const targetSize = Number(params?.targetSize ?? data.onnxTargetSize ?? 640);
      const scoreThreshold = Number(params?.scoreThreshold ?? data.onnxScoreThreshold ?? 0.25);
      const iouThreshold = Number(params?.iouThreshold ?? data.onnxIouThreshold ?? 0.45);
      const task = data.onnxCatalogId === 'yolov8n' ? 'detection' : 'generic';
      const modelPath = typeof data.onnxCustomPath === 'string' ? data.onnxCustomPath : undefined;
      onnx.set(nodeId, {
        key: [modelId, modelPath, task, targetSize, scoreThreshold, iouThreshold].join('|'),
        modelId,
        task,
        modelPath,
        targetSize,
        scoreThreshold,
        iouThreshold,
        download: modelPath ? undefined : {
          modelId,
          url: 'https://raw.githubusercontent.com/caozisheng/rimeflow-yolov8n/main/models/yolov8n.onnx',
          expectedSize: 12_851_098,
          sha256: '',
        },
      });
    }
  }
  return { video, image, onnx };
}

function stripResourceData(data: Record<string, unknown> | undefined): Record<string, unknown> | undefined {
  if (!data) return data;
  const stripped = { ...data };
  delete stripped.imageDataUrl;
  delete stripped.rawDataUrl;
  delete stripped.videoUrl;
  delete stripped.videoFilePath;
  delete stripped.videoDeviceId;
  delete stripped.onnxCustomPath;
  return stripped;
}

class TextureStreamBridge extends FakeBridge {
  override async invoke<T>(
    command: string,
    args?: NativeInvokeArgs,
    options?: NativeInvokeOptions,
  ): Promise<T> {
    if (command !== 'native_gpu_initialize') return await super.invoke<T>(command, args, options);
    this.calls.push({ command, args, options });
    return {
      adapterName: 'Test Adapter',
      backend: 'Dx12',
      deviceType: 'IntegratedGpu',
      outputMode: 'webview-texture-stream',
      nativeOnnxCpu: true,
      nativeOnnxDirectMl: true,
      sharedOnnxWgpuDevice: false,
      nativeVideo: true,
      videoDataPath: 'cpu-copy',
      tensorDataPath: 'cpu-copy',
      sharedTexture: true,
    } as T;
  }
}

class DeferredOutputBridge extends FakeBridge {
  private resolveFirstOutput: ((value: ArrayBuffer) => void) | null = null;
  private outputCalls = 0;

  override async invoke<T>(command: string, args?: NativeInvokeArgs, options?: NativeInvokeOptions): Promise<T> {
    if (command !== 'native_gpu_read_preview') return super.invoke<T>(command, args, options);
    this.calls.push({ command, args, options });
    this.outputCalls += 1;
    if (this.outputCalls > 1) {
      return new Uint8Array([1, 0, 0, 0, 1, 0, 0, 0, 10, 20, 30, 255]).buffer as T;
    }
    return await new Promise<T>((resolve) => {
      this.resolveFirstOutput = (value) => resolve(value as T);
    });
  }

  completeFirstOutput(): void {
    this.resolveFirstOutput?.(
      new Uint8Array([1, 0, 0, 0, 1, 0, 0, 0, 10, 20, 30, 255]).buffer,
    );
    this.resolveFirstOutput = null;
  }
}

describe('NativeHost transport', () => {
  it('initializes the native output runtime and reports explicit capabilities', async () => {
    const bridge = new FakeBridge();
    const runtime = new NativeHost({}, bridge);

    const info = await runtime.initialize();

    expect(info).toMatchObject({
      backend: 'Dx12',
      outputMode: 'embedded-readback',
      nativeOnnxCpu: true,
      nativeOnnxDirectMl: true,
      sharedOnnxWgpuDevice: false,
    });
    expect(bridge.handlers.has('native-runtime-frame')).toBe(true);
    expect(bridge.handlers.has('native-runtime-error')).toBe(true);
  });

  it('sends graph updates and play control without per-frame IPC', async () => {
    const bridge = new FakeBridge();
    const runtime = new NativeHost({}, bridge);
    await runtime.initialize();

    await runtime.play([], []);

    expect(bridge.calls.map(({ command }) => command)).toEqual([
      'native_gpu_initialize',
      'native_host_resource_intents',
      'native_gpu_set_graph',
      'native_gpu_play',
    ]);
    const graphCall = bridge.calls.find(({ command }) => command === 'native_gpu_set_graph')!;
    expect(JSON.parse((graphCall.args as Record<string, string>).graphJson)).toEqual({ nodes: [], edges: [] });
  });

  it('updates a playing graph without issuing a second play command', async () => {
    const bridge = new FakeBridge();
    const runtime = new NativeHost({}, bridge);
    await runtime.initialize();
    await runtime.play([], []);
    const before = bridge.calls.length;

    await runtime.updateGraph([], []);

    expect(bridge.calls.slice(before).map(({ command }) => command)).toEqual([
      'native_host_resource_intents',
      'native_gpu_set_graph',
    ]);
  });

  it('preserves SYSTEM TIME and its Hue angle edge in the native graph snapshot', async () => {
    const bridge = new FakeBridge();
    const runtime = new NativeHost({}, bridge);
    await runtime.initialize();
    const nodes = [
      {
        id: 'time', type: 'input', position: { x: 0, y: 0 },
        data: {
          type: 'input', label: 'Time', shaderCode: '', inputs: [],
          outputs: [{ id: 'time_out', label: 'value', dataType: 'float', direction: 'output' }],
          uniforms: {}, inputMode: 'system', inputDataType: 'float', systemSource: 'time',
        },
      },
      {
        id: 'hue', type: 'shader', position: { x: 1, y: 0 },
        data: {
          type: 'shader', label: 'Hue Rotate', shaderCode: '@fragment fn main() -> @location(0) vec4f { return vec4f(angle); }',
          inputs: [{ id: 'angle', label: 'angle', dataType: 'float', direction: 'input' }],
          outputs: [], uniforms: {},
        },
      },
    ];
    const edges = [{
      id: 'time_to_hue', source: 'time', sourceHandle: 'time_out',
      target: 'hue', targetHandle: 'angle',
    }];

    await runtime.setGraph(nodes as never, edges as never);

    const graphCall = bridge.calls.find(({ command }) => command === 'native_gpu_set_graph');
    const graph = JSON.parse((graphCall?.args as Record<string, string>).graphJson);
    expect(graph.nodes[0].data).toMatchObject({
      inputMode: 'system', inputDataType: 'float', systemSource: 'time',
    });
    expect(graph.edges).toEqual([expect.objectContaining(edges[0])]);
  });

  it('separates native video paths from graph snapshots and attaches once', async () => {
    const bridge = new FakeBridge();
    const runtime = new NativeHost({}, bridge);
    await runtime.initialize();
    const nodes = [{
      id: 'video-1',
      type: 'input',
      position: { x: 0, y: 0 },
      data: {
        type: 'input', label: 'Video', shaderCode: '', inputs: [], outputs: [], uniforms: {},
        inputMode: 'video', inputDataType: 'sampler2D', videoSourceType: 'file',
        videoFilePath: 'C:/video/input.mp4', videoUrl: 'asset://video/input.mp4',
      },
    }];

    await runtime.setGraph(nodes as never, []);
    await runtime.setGraph(nodes as never, []);

    const graph = JSON.parse(
      (bridge.calls.find(({ command }) => command === 'native_gpu_set_graph')?.args as Record<string, string>).graphJson,
    );
    expect(graph.nodes[0].data).not.toHaveProperty('videoFilePath');
    expect(graph.nodes[0].data).not.toHaveProperty('videoUrl');
    expect(bridge.calls.filter(({ command }) => command === 'native_gpu_attach_video')).toHaveLength(1);
  });

  it('replaces H.265 with H.264 before a stop and replay without reopening the decoder', async () => {
    const bridge = new FakeBridge();
    const runtime = new NativeHost({}, bridge);
    await runtime.initialize();
    const node = (videoFilePath: string) => ({
      id: 'video-1',
      type: 'input',
      position: { x: 0, y: 0 },
      data: {
        type: 'input', label: 'Video', shaderCode: '', inputs: [], outputs: [], uniforms: {},
        inputMode: 'video', inputDataType: 'sampler2D', videoSourceType: 'file', videoFilePath,
      },
    });
    await runtime.setGraph([node('C:/video/source-hevc.mp4')] as never, []);
    const before = bridge.calls.length;

    await runtime.setGraph([node('C:/video/source-h264.mp4')] as never, []);

    expect(bridge.calls.slice(before).map(({ command }) => command)).toEqual([
      'native_host_resource_intents',
      'native_gpu_set_graph',
      'native_gpu_detach_video',
      'native_gpu_attach_video',
    ]);
    expect(bridge.calls.at(-1)?.args).toMatchObject({
      nodeId: 'video-1',
      source: 'C:/video/source-h264.mp4',
    });
    const replayBefore = bridge.calls.length;
    await runtime.stop();
    await runtime.play([node('C:/video/source-h264.mp4')] as never, []);
    expect(bridge.calls.slice(replayBefore).map(({ command }) => command)).toEqual([
      'native_gpu_stop',
      'native_host_resource_intents',
      'native_gpu_set_graph',
      'native_gpu_play',
    ]);
  });

  it('detaches stale video before uploading a replacement image texture', async () => {
    const bridge = new FakeBridge();
    const runtime = new NativeHost({}, bridge);
    await runtime.initialize();
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      arrayBuffer: async () => new Uint8Array([255, 0, 0, 255]).buffer,
    }));
    const videoNode = {
      id: 'resource-1', type: 'input', position: { x: 0, y: 0 },
      data: {
        type: 'input', label: 'Resource', shaderCode: '', inputs: [], outputs: [], uniforms: {},
        inputMode: 'video', inputDataType: 'sampler2D', videoSourceType: 'file',
        videoFilePath: 'C:/video/input.mp4',
      },
    };
    const imageNode = {
      ...videoNode,
      data: {
        ...videoNode.data,
        inputMode: 'image', rawDataUrl: 'raw://image', fbFormat: 'rgba8', fbWidth: 1, fbHeight: 1,
        videoFilePath: undefined, videoSourceType: undefined,
      },
    };
    await runtime.setGraph([videoNode] as never, []);
    const before = bridge.calls.length;
    await runtime.setGraph([imageNode] as never, []);
    expect(bridge.calls.slice(before).map(({ command }) => command)).toEqual([
      'native_host_resource_intents', 'native_gpu_set_graph', 'native_gpu_detach_video', 'native_gpu_upload_image',
    ]);
  });


  it('forwards coalesced native frame, size, and error events', async () => {
    const bridge = new FakeBridge();
    const onFrame = vi.fn();
    const onError = vi.fn();
    const onOutputSize = vi.fn();
    const onRendererFrame = vi.fn();
    const runtime = new NativeHost({ onFrame, onError, onOutputSize, onRendererFrame }, bridge);
    await runtime.initialize();
    const frame: NativeFrameRendered = {
      frame: 6,
      revision: 2,
      outputNodeId: 'renderer',
      width: 960,
      height: 540,
    };

    runtime.setPreviewNode('renderer');
    bridge.emit('native-runtime-frame', frame);
    bridge.emit('native-runtime-error', 'device lost');

    expect(onFrame).toHaveBeenCalledWith(frame);
    expect(onError).toHaveBeenCalledWith('device lost');
    await vi.waitFor(() => expect(onRendererFrame).toHaveBeenCalledWith('renderer', {
      rgba: new Uint8Array([10, 20, 30, 255]), width: 1, height: 1,
    }));
    expect(onOutputSize).toHaveBeenCalledTimes(1);
    expect(onOutputSize).toHaveBeenCalledWith('renderer', 960, 540);
    expect(bridge.calls.find(({ command }) => command === 'native_gpu_read_preview')?.args)
      .toEqual({ nodeId: 'renderer', maxDimension: 960 });
    expect(bridge.calls.some(({ command }) => command === 'native_gpu_read_output')).toBe(false);
  });

  it('presents WebView2 TextureStream frames without renderer readback', async () => {
    const bridge = new TextureStreamBridge();
    const stop = vi.fn();
    const stream = { getTracks: () => [{ stop }] } as unknown as MediaStream;
    const getTextureStream = vi.fn(() => stream);
    Object.defineProperty(window, 'chrome', {
      configurable: true,
      value: { webview: { getTextureStream } },
    });
    const play = vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue();
    const readyState = vi
      .spyOn(HTMLMediaElement.prototype, 'readyState', 'get')
      .mockReturnValue(HTMLMediaElement.HAVE_CURRENT_DATA);
    const onRendererStream = vi.fn();
    const onRendererVideoFrame = vi.fn();
    const onError = vi.fn();
    const runtime = new NativeHost({ onRendererStream, onRendererVideoFrame, onError }, bridge);

    try {
      await expect(runtime.initialize()).resolves.toMatchObject({
        outputMode: 'webview-texture-stream',
      });
      const frame = {
        frame: 1,
        revision: 1,
        outputNodeId: 'renderer',
        width: 640,
        height: 360,
      } satisfies NativeFrameRendered;
      bridge.emit('native-runtime-frame', frame);
      await vi.waitFor(() => expect(play).toHaveBeenCalledOnce());

      expect(onRendererStream).toHaveBeenCalledWith('renderer', expect.any(HTMLVideoElement));
      expect(bridge.calls.some(({ command }) => command === 'native_gpu_read_output')).toBe(false);
      bridge.emit('native-runtime-presentation-fallback', 'present failed');
      expect(onError).toHaveBeenCalledWith('present failed');
      await vi.waitFor(() => {
        expect(bridge.calls.some(({ command }) => command === 'native_gpu_read_preview')).toBe(true);
      });
      await runtime.close();
      expect(stop).toHaveBeenCalledOnce();
    } finally {
      play.mockRestore();
      readyState.mockRestore();
      Reflect.deleteProperty(window, 'chrome');
    }
  });

  it('resumes the existing TextureStream consumer after stop-play and pause-resume', async () => {
    const bridge = new TextureStreamBridge();
    const stream = { getTracks: () => [{ stop: vi.fn() }] } as unknown as MediaStream;
    Object.defineProperty(window, 'chrome', {
      configurable: true,
      value: { webview: { getTextureStream: vi.fn(() => stream) } },
    });
    const play = vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue();
    const readyState = vi
      .spyOn(HTMLMediaElement.prototype, 'readyState', 'get')
      .mockReturnValue(HTMLMediaElement.HAVE_CURRENT_DATA);
    const requestVideoFrameCallback = vi.fn(() => 1);
    const cancelVideoFrameCallback = vi.fn();
    Object.defineProperty(HTMLVideoElement.prototype, 'requestVideoFrameCallback', {
      configurable: true,
      value: requestVideoFrameCallback,
    });
    Object.defineProperty(HTMLVideoElement.prototype, 'cancelVideoFrameCallback', {
      configurable: true,
      value: cancelVideoFrameCallback,
    });
    const runtime = new NativeHost({}, bridge);

    try {
      await runtime.initialize();
      bridge.emit('native-runtime-frame', {
        frame: 1,
        revision: 1,
        outputNodeId: 'renderer',
        width: 640,
        height: 360,
      } satisfies NativeFrameRendered);
      await vi.waitFor(() => expect(play).toHaveBeenCalledOnce());

      await runtime.stop();
      await runtime.play([], []);
      expect(play).toHaveBeenCalledTimes(2);

      await runtime.pause();
      await runtime.resume();
      expect(play).toHaveBeenCalledTimes(3);
    } finally {
      await runtime.close();
      play.mockRestore();
      readyState.mockRestore();
      Reflect.deleteProperty(HTMLVideoElement.prototype, 'requestVideoFrameCallback');
      Reflect.deleteProperty(HTMLVideoElement.prototype, 'cancelVideoFrameCallback');
      Reflect.deleteProperty(window, 'chrome');
    }
  });

  it('runs a queued renderer readback after selection remounts its canvas', async () => {
    const bridge = new DeferredOutputBridge();
    const onRendererFrame = vi.fn();
    const runtime = new NativeHost({ onRendererFrame }, bridge);
    await runtime.initialize();

    bridge.emit('native-runtime-frame', {
      frame: 1,
      revision: 1,
      outputNodeId: 'renderer',
      width: 640,
      height: 360,
    } satisfies NativeFrameRendered);
    runtime.setPreviewNode('renderer');
    runtime.requestPreviewRefresh();

    await vi.waitFor(() => {
      expect(bridge.calls.filter(({ command }) => command === 'native_gpu_read_preview')).toHaveLength(1);
    });
    bridge.completeFirstOutput();
    await vi.waitFor(() => {
      expect(bridge.calls.filter(({ command }) => command === 'native_gpu_read_preview')).toHaveLength(2);
      expect(onRendererFrame).toHaveBeenCalledTimes(2);
    });
  });

  it('forwards native ONNX pixels, data, and provider events', async () => {
    const bridge = new FakeBridge();
    const onOutputSize = vi.fn();
    const onOutputData = vi.fn();
    const onBackendDetected = vi.fn();
    const onNativeBackendDetected = vi.fn();
    const runtime = new NativeHost({
      onOutputSize,
      onOutputData,
      onBackendDetected,
      onNativeBackendDetected,
    }, bridge);
    await runtime.initialize();

    bridge.emit('native-runtime-output', {
      nodeId: 'onnx-1',
      width: 640,
      height: 640,
      data: [{ classId: 0, score: 0.9 }],
      backend: 'directml+cpu',
    });

    expect(onOutputSize).toHaveBeenCalledWith('onnx-1', 640, 640);
    expect(onOutputData).toHaveBeenCalledWith('onnx-1', [{ classId: 0, score: 0.9 }]);
    expect(onBackendDetected).toHaveBeenCalledWith('onnx-1', 'native');
    expect(onNativeBackendDetected).toHaveBeenCalledWith('onnx-1', 'directml+cpu');
  });

  it('uploads raw image bytes once and decodes binary output readback', async () => {
    const bridge = new FakeBridge();
    const runtime = new NativeHost({}, bridge);
    await runtime.initialize();
    const rgba = new Uint8Array([10, 20, 30, 255]);

    await runtime.uploadImage('image-1', rgba, 1, 1);
    await expect(runtime.readOutput('image-1')).resolves.toEqual({
      rgba,
      width: 1,
      height: 1,
    });

    expect(bridge.calls.at(-2)).toEqual({
      command: 'native_gpu_upload_image',
      args: rgba,
      options: {
        headers: {
          'x-open-quartz-node-id': 'image-1',
          'x-open-quartz-width': '1',
          'x-open-quartz-height': '1',
        },
      },
    });
    expect(bridge.calls.at(-1)).toEqual({
      command: 'native_gpu_read_output',
      args: { nodeId: 'image-1' },
      options: undefined,
    });
  });

  it('delivers bounded Renderer frames without encoding PNG previews', async () => {
    const bridge = new FakeBridge();
    const onRendererFrame = vi.fn();
    const onOutput = vi.fn();
    const runtime = new NativeHost({ onRendererFrame, onOutput }, bridge);
    await runtime.initialize();
    runtime.setPreviewNode('renderer');
    bridge.emit('native-runtime-frame', {
      frame: 6,
      revision: 1,
      outputNodeId: 'renderer',
      width: 1,
      height: 1,
    });

    await vi.waitFor(() => expect(onRendererFrame).toHaveBeenCalledOnce());
    expect(onOutput).not.toHaveBeenCalled();
    expect(bridge.calls.filter(({ command }) => command === 'native_gpu_read_preview')).toHaveLength(1);
  });

  it('validates mouse state and decodes engine events', async () => {
    const bridge = new FakeBridge();
    const runtime = new NativeHost({}, bridge);
    await runtime.initialize();

    await expect(runtime.setMouse(new Float32Array(3))).rejects.toThrow(
      'exactly 4 values',
    );
    await runtime.setMouse(new Float32Array([1, 2, 3, 4]));
    await expect(runtime.drainEvents()).resolves.toEqual([
      { type: 'graph-ready', revision: 1 },
    ]);
    expect(bridge.calls.at(-2)).toEqual({
      command: 'native_gpu_set_mouse',
      args: { mouse: [1, 2, 3, 4] },
    });
  });

  it('attaches native video by file path without frame pixel IPC', async () => {
    const bridge = new FakeBridge();
    const runtime = new NativeHost({}, bridge);
    await runtime.initialize();
    await expect(runtime.listVideoDevices()).resolves.toEqual([
      { id: 'camera-0', label: 'Integrated Camera' },
    ]);

    await expect(
      runtime.attachVideo('video-1', 'file', 'C:/video/input.mp4', true, 1.25),
    ).resolves.toEqual({
      width: 1920,
      height: 1080,
      fps: 30,
      decoder: 'ffmpeg-native',
    });
    await runtime.detachVideo('video-1');

    expect(bridge.calls.at(-2)).toEqual({
      command: 'native_gpu_attach_video',
      args: {
        nodeId: 'video-1',
        kind: 'file',
        source: 'C:/video/input.mp4',
        looping: true,
        playbackRate: 1.25,
      },
      options: undefined,
    });
    expect(bridge.calls.at(-1)?.command).toBe('native_gpu_detach_video');
  });

  it('loads native ONNX sessions by model ID without transferring model bytes', async () => {
    const bridge = new FakeBridge();
    const runtime = new NativeHost({}, bridge);
    await runtime.initialize();

    await expect(runtime.onnxCapabilities()).resolves.toEqual({
      cpu: true,
      directMl: true,
      sharedWgpuDevice: false,
    });
    await expect(runtime.loadOnnxModel('onnx-1', 'yolov8n', 'detection')).resolves.toEqual({
      inputNames: ['images'],
      outputNames: ['output0'],
      backend: 'directml+cpu',
    });
    await runtime.unloadOnnxModel('onnx-1');

    expect(bridge.calls.at(-2)).toEqual({
      command: 'native_onnx_load_model',
      args: {
        nodeId: 'onnx-1',
        modelId: 'yolov8n',
        options: {
          modelPath: undefined, task: 'detection', targetSize: 640,
          scoreThreshold: 0.25, iouThreshold: 0.45, preferDirectMl: true,
        },
      },
    });
    expect(bridge.calls.at(-1)).toEqual({
      command: 'native_onnx_unload_model',
      args: { nodeId: 'onnx-1' },
    });
  });

  it('downloads, loads, reuses, and unloads catalog ONNX resources with the graph', async () => {
    const bridge = new FakeBridge();
    const runtime = new NativeHost({}, bridge);
    await runtime.initialize();
    const node = {
      id: 'onnx-1',
      type: 'onnx',
      position: { x: 0, y: 0 },
      data: {
        type: 'onnx', label: 'Detector', shaderCode: '', inputs: [], outputs: [], uniforms: {},
        onnxCatalogId: 'yolov8n', onnxModelId: 'yolov8n',
        onnxParams: { targetSize: 320, scoreThreshold: 0.4, iouThreshold: 0.5 },
      },
    };

    await runtime.setGraph([node] as never, []);
    await runtime.setGraph([node] as never, []);
    await runtime.setGraph([], []);

    expect(bridge.calls.filter(({ command }) => command === 'download_model')).toHaveLength(1);
    const loads = bridge.calls.filter(({ command }) => command === 'native_onnx_load_model');
    expect(loads).toHaveLength(1);
    expect(loads[0]?.args).toMatchObject({
      nodeId: 'onnx-1', modelId: 'yolov8n',
      options: { task: 'detection', targetSize: 320, scoreThreshold: 0.4, iouThreshold: 0.5 },
    });
    expect(bridge.calls.filter(({ command }) => command === 'native_onnx_unload_model')).toHaveLength(1);
  });

  it('closes the native window and releases listeners exactly once', async () => {
    const bridge = new FakeBridge();
    const runtime = new NativeHost({}, bridge);
    await runtime.initialize();

    await runtime.close();
    await runtime.close();

    expect(bridge.calls.filter(({ command }) => command === 'native_gpu_close')).toHaveLength(1);
    expect(bridge.unlisten).toHaveBeenCalledTimes(4);
    await expect(runtime.renderOnce()).rejects.toThrow('not initialized');
  });
});
