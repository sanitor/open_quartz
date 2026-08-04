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
});
