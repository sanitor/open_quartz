import { beforeEach, describe, expect, it, vi } from 'vitest';
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
});
