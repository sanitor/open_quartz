import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => {
  class FakeRuntime {
    readonly initialize = vi.fn(async () => undefined);
    readonly play = vi.fn(async () => undefined);
    readonly updateGraph = vi.fn(async () => undefined);
    readonly pause = vi.fn(async () => undefined);
    readonly resume = vi.fn(async () => undefined);
    readonly stop = vi.fn(async () => undefined);
    readonly setPreviewNode = vi.fn();
    readonly requestPreviewRefresh = vi.fn();
    readonly captureScreenshot = vi.fn(async () => 'data:image/png;base64,capture');
    readonly close = vi.fn(async () => undefined);

    constructor(readonly callbacks: Record<string, (...args: never[]) => void>) {}
  }
  return {
    FakeRuntime,
    isTauri: vi.fn<() => Promise<boolean>>(),
    browser: [] as FakeRuntime[],
    native: [] as FakeRuntime[],
  };
});

vi.mock('../../src/utils/tauri', () => ({
  checkIsTauri: mocks.isTauri,
}));

vi.mock('../../src/sdk/BrowserPipelineRuntime', () => ({
  BrowserPipelineRuntime: class extends mocks.FakeRuntime {
    constructor(callbacks: Record<string, (...args: never[]) => void>) {
      super(callbacks);
      mocks.browser.push(this);
    }
  },
}));

vi.mock('../../src/sdk/NativePipelineRuntime', () => ({
  NativePipelineRuntime: class extends mocks.FakeRuntime {
    constructor(callbacks: Record<string, (...args: never[]) => void>) {
      super(callbacks);
      mocks.native.push(this);
    }
  },
}));

import { PipelineService } from '../../src/services/PipelineService';
import { useGraphStore } from '../../src/store/useGraphStore';

function resetStore(): void {
  useGraphStore.setState({
    nodes: [],
    edges: [],
    selectedNodeId: null,
    loopState: 'stopped',
    outputPreviews: {},
    outputData: {},
    nodeErrors: {},
    fps: 0,
    rendererFps: {},
    currentTime: 0,
    currentFrame: 0,
  });
}

describe('PipelineService', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    mocks.isTauri.mockReset();
    mocks.browser.length = 0;
    mocks.native.length = 0;
    resetStore();
  });

  it('selects the browser runtime and forwards runtime callbacks into the store', async () => {
    mocks.isTauri.mockResolvedValue(false);
    const service = new PipelineService();
    service.attach(document.createElement('canvas'));

    useGraphStore.getState().play();
    await vi.waitFor(() => expect(mocks.browser).toHaveLength(1));
    const runtime = mocks.browser[0]!;
    await vi.waitFor(() => expect(runtime.play).toHaveBeenCalledOnce());

    runtime.callbacks.onFrame?.({ frame: 12, time: 0.2, fps: 60 } as never);
    runtime.callbacks.onOutput?.('renderer', 'data:image/png;base64,preview' as never);
    runtime.callbacks.onOutputSize?.('renderer', 1920 as never, 1080 as never);
    runtime.callbacks.onOutputData?.('onnx', [{ classId: 0 }] as never);
    runtime.callbacks.onBackendDetected?.('onnx', 'webgpu' as never);

    expect(useGraphStore.getState()).toMatchObject({
      currentFrame: 12,
      currentTime: 0.2,
      fps: 60,
      outputPreviews: { renderer: 'data:image/png;base64,preview' },
      outputData: { onnx: [{ classId: 0 }] },
    });
    expect(mocks.native).toHaveLength(0);
    service.detach();
    await vi.waitFor(() => expect(runtime.close).toHaveBeenCalledOnce());
  });

  it('clears runtime previews on stop and ignores late output callbacks', async () => {
    mocks.isTauri.mockResolvedValue(false);
    const service = new PipelineService();
    service.attach(document.createElement('canvas'));

    useGraphStore.getState().play();
    await vi.waitFor(() => expect(mocks.browser).toHaveLength(1));
    const runtime = mocks.browser[0]!;
    await vi.waitFor(() => expect(runtime.play).toHaveBeenCalledOnce());

    runtime.callbacks.onOutput?.('input_2', 'data:image/png;base64,runtime-frame' as never);
    expect(useGraphStore.getState().outputPreviews).toEqual({
      input_2: 'data:image/png;base64,runtime-frame',
    });

    useGraphStore.getState().stop();
    await vi.waitFor(() => expect(runtime.stop).toHaveBeenCalledOnce());
    expect(useGraphStore.getState().outputPreviews).toEqual({});

    runtime.callbacks.onOutput?.('input_2', 'data:image/png;base64,late-frame' as never);
    expect(useGraphStore.getState().outputPreviews).toEqual({});

    const setPreviewCalls = runtime.setPreviewNode.mock.calls.length;
    const refreshCalls = runtime.requestPreviewRefresh.mock.calls.length;
    useGraphStore.getState().setSelectedNode('input_2');
    expect(runtime.setPreviewNode.mock.calls.length).toBe(setPreviewCalls);
    expect(runtime.requestPreviewRefresh.mock.calls.length).toBe(refreshCalls);
    service.detach();
  });

  it('counts frames only after a Renderer is delivered for display', async () => {
    mocks.isTauri.mockResolvedValue(false);
    const now = vi.spyOn(performance, 'now').mockReturnValue(0);
    const service = new PipelineService();
    service.attach(document.createElement('canvas'));
    useGraphStore.getState().play();
    await vi.waitFor(() => expect(mocks.browser).toHaveLength(1));
    const runtime = mocks.browser[0]!;

    for (let frame = 0; frame < 30; frame += 1) {
      runtime.callbacks.onRendererPresented?.('renderer-a' as never);
    }
    expect(useGraphStore.getState().rendererFps).toEqual({});

    now.mockReturnValue(600);
    runtime.callbacks.onRendererPresented?.('renderer-a' as never);
    expect(useGraphStore.getState().rendererFps['renderer-a']).toBeCloseTo(31_000 / 600);
    expect(useGraphStore.getState().rendererFps['renderer-b']).toBeUndefined();

    useGraphStore.getState().stop();
    expect(useGraphStore.getState().rendererFps).toEqual({});
    service.detach();
  });

  it('samples native Renderer FPS over a stable window and resets it on replay', async () => {
    mocks.isTauri.mockResolvedValue(true);
    const now = vi.spyOn(performance, 'now').mockReturnValue(0);
    const service = new PipelineService();
    service.attach(document.createElement('canvas'));

    useGraphStore.getState().play();
    await vi.waitFor(() => expect(mocks.native).toHaveLength(1));
    const runtime = mocks.native[0]!;
    await vi.waitFor(() => expect(runtime.play).toHaveBeenCalledOnce());

    now.mockReturnValue(100);
    runtime.callbacks.onFrame?.({ frame: 1, outputNodeId: 'renderer', width: 1, height: 1 } as never);
    expect(useGraphStore.getState().fps).toBe(0);

    now.mockReturnValue(600);
    runtime.callbacks.onFrame?.({ frame: 31, outputNodeId: 'renderer', width: 1, height: 1 } as never);
    expect(useGraphStore.getState().fps).toBeCloseTo(31_000 / 600);

    useGraphStore.getState().stop();
    now.mockReturnValue(1_000);
    useGraphStore.getState().play();
    await vi.waitFor(() => expect(runtime.play).toHaveBeenCalledTimes(2));
    now.mockReturnValue(1_016);
    runtime.callbacks.onFrame?.({ frame: 1, outputNodeId: 'renderer', width: 1, height: 1 } as never);
    expect(useGraphStore.getState()).toMatchObject({ fps: 0, currentFrame: 1 });

    service.detach();
  });

  it('does not rebuild the native graph for the in-place UI toggle', async () => {
    mocks.isTauri.mockResolvedValue(true);
    const service = new PipelineService();
    service.attach(document.createElement('canvas'));
    useGraphStore.getState().play();
    await vi.waitFor(() => expect(mocks.native).toHaveLength(1));
    const runtime = mocks.native[0]!;
    const node = {
      id: 'renderer', type: 'renderer', position: { x: 0, y: 0 },
      data: { type: 'renderer', label: 'Renderer', shaderCode: '', inputs: [], outputs: [], uniforms: {}, expanded: true },
    };
    useGraphStore.setState({ nodes: [node] as never });
    await vi.waitFor(() => expect(runtime.updateGraph).toHaveBeenCalledOnce());
    useGraphStore.setState({ nodes: [{ ...node, data: { ...node.data, expanded: false } }] as never });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(runtime.updateGraph).toHaveBeenCalledOnce();
    service.detach();
    await vi.waitFor(() => expect(runtime.close).toHaveBeenCalledOnce());
  });

  it('selects the native runtime and reports its execution provider', async () => {
    mocks.isTauri.mockResolvedValue(true);
    const sourceContext = { putImageData: vi.fn() } as unknown as CanvasRenderingContext2D;
    const mirrorContext = {
      clearRect: vi.fn(),
      drawImage: vi.fn(),
    } as unknown as CanvasRenderingContext2D;
    const mirror = document.createElement('canvas');
    mirror.id = 'renderer-mirror-renderer';
    mirror.width = 2;
    mirror.height = 2;
    document.body.appendChild(mirror);
    vi.stubGlobal('ImageData', class {
      constructor(_pixels: Uint8ClampedArray, _width: number, _height: number) {}
    });
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(function () {
      return this === mirror ? mirrorContext : sourceContext;
    } as typeof HTMLCanvasElement.prototype.getContext);
    const service = new PipelineService();
    service.attach(document.createElement('canvas'));

    useGraphStore.getState().play();
    await vi.waitFor(() => expect(mocks.native).toHaveLength(1));
    useGraphStore.setState({
      nodes: [{
        id: 'onnx', type: 'onnx', position: { x: 0, y: 0 },
        data: { type: 'onnx', label: 'ONNX', shaderCode: '', inputs: [], outputs: [], uniforms: {} },
      }] as never,
    });
    const runtime = mocks.native[0]!;
    await vi.waitFor(() => expect(runtime.play).toHaveBeenCalledOnce());

    runtime.callbacks.onBackendDetected?.('onnx', 'native' as never);
    runtime.callbacks.onNativeBackendDetected?.('onnx', 'directml+cpu' as never);
    runtime.callbacks.onRendererFrame?.('renderer', {
      rgba: new Uint8Array(16), width: 2, height: 2,
    } as never);
    expect(sourceContext.putImageData).toHaveBeenCalledOnce();
    expect(mirrorContext.drawImage).toHaveBeenCalledWith(expect.any(HTMLCanvasElement), 0, 0, 2, 2);
    window.dispatchEvent(new CustomEvent('renderer-remount'));
    expect(runtime.requestPreviewRefresh).toHaveBeenCalledOnce();
    expect(mirrorContext.drawImage).toHaveBeenCalledTimes(2);
    expect(useGraphStore.getState().nodes[0]?.data).toMatchObject({
      onnxBackend: 'native',
      onnxNativeBackend: 'directml+cpu',
    });
    expect(mocks.browser).toHaveLength(0);

    await service.detach();
    await vi.waitFor(() => expect(runtime.close).toHaveBeenCalledOnce());
    mirror.remove();
  });
  it('keeps the canonical texture stream playing while moving into and out of fullscreen', async () => {
    mocks.isTauri.mockResolvedValue(true);
    const service = new PipelineService();
    service.attach(document.createElement('canvas'));
    useGraphStore.getState().play();
    await vi.waitFor(() => expect(mocks.native).toHaveLength(1));
    const runtime = mocks.native[0]!;
    await vi.waitFor(() => expect(runtime.play).toHaveBeenCalledOnce());

    const sidePanelSlot = document.createElement('div');
    sidePanelSlot.id = 'renderer-stream-slot-sidepanel-renderer';
    document.body.appendChild(sidePanelSlot);
    const video = document.createElement('video');
    const stream = {} as MediaStream;
    Object.defineProperty(video, 'srcObject', { configurable: true, value: stream });
    const play = vi.spyOn(video, 'play').mockResolvedValue();

    runtime.callbacks.onRendererStream?.('renderer' as never, video as never);
    expect(video.parentElement).toBe(sidePanelSlot);

    const fullscreenSlot = document.createElement('div');
    fullscreenSlot.id = 'renderer-stream-slot-fullscreen-renderer';
    document.body.appendChild(fullscreenSlot);
    window.dispatchEvent(new CustomEvent('renderer-remount'));
    expect(video.parentElement).toBe(fullscreenSlot);
    expect(play).toHaveBeenCalled();

    window.dispatchEvent(new CustomEvent('renderer-remount', {
      detail: { nodeId: 'renderer', fullscreen: false },
    }));
    expect(video.parentElement).toBe(sidePanelSlot);
    expect(video.isConnected).toBe(true);

    service.detach();
    await vi.waitFor(() => expect(runtime.close).toHaveBeenCalledOnce());
    fullscreenSlot.remove();
    sidePanelSlot.remove();
  });

});
