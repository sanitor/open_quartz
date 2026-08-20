import { describe, expect, it, vi } from 'vitest';
import { SDK_API_VERSION, SdkContractError } from '../../src/sdk/contract';
import { WasmSdkClient, type RawWasmBindings } from '../../src/sdk/WasmSdkClient';

const capabilities = {
  structuredEngine: true,
  typedFramePlanning: true,
  resourceGenerations: true,
  graphPlanning: true,
  wgslParsing: true,
  wgslCompilation: true,
  gpuResourcePrimitives: true,
  gpuExecution: false,
  onnxPrePostprocessing: true,
  nativeOnnxSession: false,
  browserOnnxSession: false,
};

const runtimeContract = {
  apiVersion: SDK_API_VERSION,
  methods: ['set_graph', 'subscribe_output', 'update_presentation', 'drain_deliveries'],
};


class FakeBrowserPlayer {
  static last: FakeBrowserPlayer;
  graphJson = '';
  timestamps: bigint[] = [];
  subscriptions: string[] = [];

  static async create(_canvas: OffscreenCanvas): Promise<FakeBrowserPlayer> {
    const player = new FakeBrowserPlayer();
    FakeBrowserPlayer.last = player;
    return player;
  }

  setGraph(graphJson: string): number { this.graphJson = graphJson; return 2; }
  play(nowNs: bigint): void { this.timestamps.push(nowNs); }
  pause(nowNs: bigint): void { this.timestamps.push(nowNs); }
  resume(nowNs: bigint): void { this.timestamps.push(nowNs); }
  stop(): void {}
  uploadFrame(_nodeId: string, _bitmap: ImageBitmap, timestampNs: bigint): void {
    this.timestamps.push(timestampNs);
  }
  uploadRgba(): void {}
  async readOutputRgba(): Promise<Uint8Array> { return new Uint8Array([64, 128, 191, 255]); }
  outputInfo(): string { return JSON.stringify({ width: 1, height: 1 }); }
  frame(): string {
    return JSON.stringify({
      clock: { epoch: 1, frame: 2, timelineNs: 3, previousTimelineNs: 2, nextDeadlineNs: 4 },
      inferenceTasks: [{ nodeId: 'onnx', graphRevision: 2, nodeGeneration: 1 }],
    });
  }
  subscribeOutput(subscriptionJson: string): void { this.subscriptions.push(subscriptionJson); }
  unsubscribeOutput(subscriptionId: string): void {
    this.subscriptions = this.subscriptions.filter(
      (item) => JSON.parse(item).subscriptionId !== subscriptionId,
    );
  }
  submitCompletion(): void {}
  drainDeliveries(): string { return JSON.stringify({ deliveries: [], invalidations: [] }); }
  close(): void {}
}

function fakeBindings(apiVersion = SDK_API_VERSION): RawWasmBindings {
  return {
    default: vi.fn(async () => undefined),
    apiVersion: () => apiVersion,
    capabilities: () => JSON.stringify(capabilities),
    sdkVersion: () => '0.16.0',
    runtimeContract: () => JSON.stringify(runtimeContract),
    catalog: () => JSON.stringify({ mathCategories: [], mathOps: [], onnxCategories: [], onnxModels: [], shaderGroups: [] }),
    planHostResourceIntents: (requestJson) => requestJson,
    planBrowserOnnxTask: (requestJson) => requestJson,
    encodeBrowserOnnxInput: (_rgba, requestJson) => JSON.stringify({ request: JSON.parse(requestJson), tensor: [1] }),
    decodeBrowserOnnxOutput: (_sourceRgba, _raw, requestJson) => JSON.stringify({ request: JSON.parse(requestJson), width: 1, height: 1 }),
    buildBrowserOnnxCompletion: (requestJson) => JSON.stringify({ nodeId: JSON.parse(requestJson).nodeId }),
    parseShader: (code) => JSON.stringify({ code }),
    planGraph: (graphJson) => graphJson,
    BrowserPlayer: FakeBrowserPlayer,
  };
}

describe('WasmSdkClient', () => {
  it('initializes bindings and validates the API contract', async () => {
    const bindings = fakeBindings();
    const client = await WasmSdkClient.load(async () => bindings);

    expect(bindings.default).toHaveBeenCalledOnce();
    expect(client.sdkVersion).toBe('0.16.0');
    expect(client.capabilities).toEqual(capabilities);
    expect(client.runtimeContract).toEqual(runtimeContract);
    expect(client.catalog()).toEqual({ mathCategories: [], mathOps: [], onnxCategories: [], onnxModels: [], shaderGroups: [] });
    expect(client.parseShader<{ code: string }>('shader').code).toBe('shader');
    expect(client.planHostResourceIntents<{ host: string }>({ host: 'browser' }).host).toBe('browser');
    expect(client.planBrowserOnnxTask<{ modelId: string }>({ modelId: 'yolov8n' }).modelId).toBe('yolov8n');
    expect(client.encodeBrowserOnnxInput<{ tensor: number[] }>(new Uint8Array(4), {}).tensor).toEqual([1]);
    expect(client.decodeBrowserOnnxOutput<{ width: number }>(new Uint8Array(4), new Float32Array(1), {}).width).toBe(1);
    expect(client.buildBrowserOnnxCompletion<{ nodeId: string }>({ nodeId: 'onnx' }).nodeId).toBe('onnx');
  });



  it('projects BrowserPlayer lifecycle, frame, GPU readback, and subscriptions', async () => {
    const client = await WasmSdkClient.load(async () => fakeBindings());
    const player = await client.createBrowserPlayer({} as OffscreenCanvas);

    expect(player.setGraph([], [])).toBe(2);
    player.play(10);
    player.pause(20);
    player.resume(30);
    player.uploadFrame('video', {} as ImageBitmap, 40);
    expect(FakeBrowserPlayer.last.timestamps).toEqual([10n, 20n, 30n, 40n]);
    expect(player.frame({
      time: 0.016,
      delta: 0,
      frame: 0,
      date: new Float32Array(4),
      mouse: new Float32Array(4),
      resolution: new Float32Array([4, 4, 1]),
    })).toMatchObject({
      clock: { frame: 2 },
      inferenceTasks: [{ nodeId: 'onnx', graphRevision: 2, nodeGeneration: 1 }],
    });
    await expect(player.readOutputRgba('renderer')).resolves.toEqual(
      new Uint8Array([64, 128, 191, 255]),
    );
    player.subscribeOutput({
      subscriptionId: 'preview',
      output: { nodeId: 'color', portId: 'out' },
      delivery: 'latest',
      transport: 'preview',
    });
    expect(FakeBrowserPlayer.last.subscriptions).toHaveLength(1);
    player.unsubscribeOutput('preview');
    expect(FakeBrowserPlayer.last.subscriptions).toEqual([]);
  });

  it('rejects incompatible API versions before creating an engine', async () => {
    await expect(WasmSdkClient.load(async () => fakeBindings(99))).rejects.toMatchObject({
      code: 'protocol-mismatch',
    });
  });
});
