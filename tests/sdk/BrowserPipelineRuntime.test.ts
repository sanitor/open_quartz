import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { BrowserPipelineRuntime } from '../../src/sdk/BrowserPipelineRuntime';
import type { BrowserWorkerResponse } from '../../src/sdk/browserWorkerProtocol';

class FakeWorker {
  static instances: FakeWorker[] = [];
  onmessage: ((event: MessageEvent<BrowserWorkerResponse>) => void) | null = null;
  onerror: ((event: ErrorEvent) => void) | null = null;
  readonly messages: Array<{ message: Record<string, unknown>; transfer: Transferable[] }> = [];
  terminated = false;

  constructor() { FakeWorker.instances.push(this); }

  postMessage(message: Record<string, unknown>, transfer: Transferable[] = []): void {
    this.messages.push({ message, transfer });
    if (!('id' in message)) return;
    queueMicrotask(() => this.onmessage?.({
      data: { id: message.id as number, ok: true, value: message.type === 'capture' ? 'capture' : undefined },
    } as MessageEvent<BrowserWorkerResponse>));
  }

  terminate(): void { this.terminated = true; }

  emit(message: BrowserWorkerResponse): void {
    this.onmessage?.({ data: message } as MessageEvent<BrowserWorkerResponse>);
  }
}

describe('BrowserPipelineRuntime worker host', () => {
  beforeEach(() => {
    FakeWorker.instances = [];
    vi.stubGlobal('Worker', FakeWorker);
  });

  afterEach(() => {
    delete (HTMLVideoElement.prototype as Partial<HTMLVideoElement>).requestVideoFrameCallback;
    delete (HTMLVideoElement.prototype as Partial<HTMLVideoElement>).cancelVideoFrameCallback;
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it('transfers the canvas once and projects graph/lifecycle intents to the worker', async () => {
    const offscreen = {} as OffscreenCanvas;
    const canvas = {
      transferControlToOffscreen: vi.fn(() => offscreen),
    } as unknown as HTMLCanvasElement;
    const runtime = new BrowserPipelineRuntime();

    await runtime.initialize(canvas);
    await runtime.play([], []);
    await runtime.updateGraph([], []);
    await runtime.pause();
    await runtime.resume();
    await runtime.stop();

    const worker = FakeWorker.instances[0];
    expect(canvas.transferControlToOffscreen).toHaveBeenCalledOnce();
    expect(worker.messages[0].message.type).toBe('initialize');
    expect(worker.messages[0].transfer).toEqual([offscreen]);
    expect(worker.messages.map(({ message }) => message.type)).toEqual([
      'initialize', 'play', 'update-graph', 'pause', 'resume', 'stop',
    ]);
  });

  it('projects worker deliveries and closes without accepting more requests', async () => {
    const onFrame = vi.fn();
    const runtime = new BrowserPipelineRuntime({ onFrame });
    const canvas = {
      transferControlToOffscreen: () => ({} as OffscreenCanvas),
    } as HTMLCanvasElement;
    await runtime.initialize(canvas);
    const worker = FakeWorker.instances[0];
    worker.emit({ type: 'frame', frame: 2, time: 1, fps: 60 });
    expect(onFrame).toHaveBeenCalledWith({ type: 'frame', frame: 2, time: 1, fps: 60 });
    await expect(runtime.captureScreenshot('renderer')).resolves.toBe('capture');
    await runtime.close();
    expect(worker.terminated).toBe(true);
    await expect(runtime.captureScreenshot('renderer')).rejects.toThrow('not initialized');
  });

  it('reports Renderer presentation only after drawing a mirror canvas', async () => {
    const onRendererPresented = vi.fn();
    const context = { clearRect: vi.fn(), drawImage: vi.fn() } as unknown as CanvasRenderingContext2D;
    const mirror = document.createElement('canvas');
    mirror.id = 'renderer-mirror-renderer';
    vi.spyOn(mirror, 'getContext').mockReturnValue(context);
    document.body.appendChild(mirror);
    vi.stubGlobal('Image', class {
      onload: (() => void) | null = null;
      set src(_value: string) { this.onload?.(); }
    });
    const runtime = new BrowserPipelineRuntime({ onRendererPresented });
    await runtime.initialize({
      transferControlToOffscreen: () => ({} as OffscreenCanvas),
    } as HTMLCanvasElement);

    FakeWorker.instances[0].emit({ type: 'output', nodeId: 'renderer', dataUrl: 'data:image/png;base64,frame' });

    expect(context.drawImage).toHaveBeenCalledOnce();
    expect(onRendererPresented).toHaveBeenCalledWith('renderer');
    mirror.remove();
  });
  it('decodes web video on the main thread and transfers bounded frames to the worker', async () => {
    let frameCallback: VideoFrameRequestCallback | null = null;
    const requestVideoFrameCallback = vi.fn((callback: VideoFrameRequestCallback) => {
      frameCallback = callback;
      return 7;
    });
    const cancelVideoFrameCallback = vi.fn();
    Object.defineProperty(HTMLVideoElement.prototype, 'requestVideoFrameCallback', {
      configurable: true, value: requestVideoFrameCallback,
    });
    Object.defineProperty(HTMLVideoElement.prototype, 'cancelVideoFrameCallback', {
      configurable: true, value: cancelVideoFrameCallback,
    });
    const play = vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue();
    const pause = vi.spyOn(HTMLMediaElement.prototype, 'pause').mockImplementation(() => undefined);
    vi.spyOn(HTMLMediaElement.prototype, 'readyState', 'get')
      .mockReturnValue(HTMLMediaElement.HAVE_CURRENT_DATA);
    vi.spyOn(HTMLMediaElement.prototype, 'paused', 'get').mockReturnValue(false);
    const bitmap = { close: vi.fn() } as unknown as ImageBitmap;
    const createBitmap = vi.fn().mockResolvedValue(bitmap);
    vi.stubGlobal('createImageBitmap', createBitmap);
    const runtime = new BrowserPipelineRuntime();
    await runtime.initialize({
      transferControlToOffscreen: () => ({} as OffscreenCanvas),
    } as HTMLCanvasElement);
    const videoNode = {
      id: 'video', type: 'input', position: { x: 0, y: 0 },
      data: {
        type: 'input', label: 'Video', shaderCode: '', inputs: [], outputs: [], uniforms: {},
        inputMode: 'video', inputDataType: 'sampler2D', videoSourceType: 'file',
        videoUrl: 'blob:web-video', videoLoop: true, videoPlaybackRate: 1,
      },
    };

    await runtime.play([videoNode] as never, []);
    expect(play).toHaveBeenCalled();
    expect(requestVideoFrameCallback).toHaveBeenCalled();
    frameCallback!(0, {} as VideoFrameCallbackMetadata);
    await vi.waitFor(() => {
      expect(FakeWorker.instances[0].messages.some(({ message }) => message.type === 'video-frame')).toBe(true);
    });
    const worker = FakeWorker.instances[0];
    const delivery = worker.messages.find(({ message }) => message.type === 'video-frame');
    expect(delivery?.message).toMatchObject({
      type: 'video-frame', nodeId: 'video', frameId: 1, frame: bitmap,
    });
    expect(delivery?.transfer).toEqual([bitmap]);

    frameCallback!(0, {} as VideoFrameCallbackMetadata);
    await Promise.resolve();
    expect(worker.messages.filter(({ message }) => message.type === 'video-frame')).toHaveLength(1);
    expect(createBitmap).toHaveBeenCalledOnce();

    worker.emit({ type: 'video-frame-consumed', nodeId: 'video', frameId: 1 });
    frameCallback!(0, {} as VideoFrameCallbackMetadata);
    await vi.waitFor(() => {
      expect(worker.messages.filter(({ message }) => message.type === 'video-frame')).toHaveLength(2);
    });
    expect(createBitmap).toHaveBeenCalledTimes(2);

    await runtime.stop();
    expect(pause).toHaveBeenCalled();
    expect(cancelVideoFrameCallback).toHaveBeenCalledWith(7);
  });

  it('keeps the current decoder alive when a replacement video cannot play', async () => {
    const play = vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue();
    vi.spyOn(HTMLMediaElement.prototype, 'pause').mockImplementation(() => undefined);
    const runtime = new BrowserPipelineRuntime();
    await runtime.initialize({
      transferControlToOffscreen: () => ({} as OffscreenCanvas),
    } as HTMLCanvasElement);
    const videoNode = (videoUrl: string) => ({
      id: 'video', type: 'input', position: { x: 0, y: 0 },
      data: {
        type: 'input', label: 'Video', shaderCode: '', inputs: [], outputs: [], uniforms: {},
        inputMode: 'video', inputDataType: 'sampler2D', videoSourceType: 'file',
        videoUrl, videoLoop: true, videoPlaybackRate: 1,
      },
    });

    await runtime.play([videoNode('blob:current')] as never, []);
    expect(document.body.querySelectorAll('video')).toHaveLength(1);
    play.mockRejectedValueOnce(new Error('unsupported replacement'));
    await expect(runtime.updateGraph([videoNode('blob:replacement')] as never, []))
      .rejects.toThrow('unsupported replacement');

    const remaining = document.body.querySelectorAll('video');
    expect(remaining).toHaveLength(1);
    expect(remaining[0].src).toBe('blob:current');
    expect(FakeWorker.instances[0].messages.filter(({ message }) => message.type === 'update-graph'))
      .toHaveLength(0);
    await runtime.stop();
  });

});
