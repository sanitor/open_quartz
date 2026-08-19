import { describe, expect, it, vi } from 'vitest';
import { requireSdk, SDK_API_VERSION, SdkContractError, WasmSdkClient } from '../../src/sdk';
import type { RawWasmBindings } from '../../src/sdk';

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

class FakeEngine {
  revision = 0;
  lastFrame?: bigint;
  pendingCommandCount = 0;
  graphJson = '';
  videoNodesJson = '[]';
  events: unknown[] = [];
  state = 'empty';

  setGraph(graphJson: string): number {
    this.graphJson = graphJson;
    this.revision += 1;
    this.state = 'ready';
    this.events.push({ type: 'graph-ready', revision: this.revision });
    this.events.push({ type: 'state', state: 'ready' });
    return this.revision;
  }

  markDirty(nodeId: string): void {
    if (this.state === 'disposed') {
      throw JSON.stringify({ code: 'disposed', message: 'Engine has been disposed', nodeId });
    }
  }

  runFrame(_time: number, _delta: number, frame: bigint): void {
    this.lastFrame = frame;
    this.pendingCommandCount = 1;
    this.state = 'running';
    this.events.push({
      type: 'frame-planned',
      frame: Number(frame),
      revision: this.revision,
      commandCount: 1,
      dirtyNodeCount: 1,
    });
  }

  setVideoNodes(nodeIdsJson: string): void {
    this.videoNodesJson = nodeIdsJson;
  }

  nodeGeneration(_nodeId: string): number {
    return 1;
  }

  pause(): void {
    this.state = 'paused';
  }

  resume(): void {
    this.state = 'running';
  }

  stop(): void {
    this.state = 'stopped';
  }

  engineState(): string {
    return this.state;
  }

  drainEvents(): string {
    const result = JSON.stringify(this.events);
    this.events = [];
    return result;
  }

  dispose(): void {
    this.state = 'disposed';
  }
}

class FakeRuntime {
  static last: FakeRuntime;
  lifecycleTimestamps: bigint[] = [];

  constructor() {
    FakeRuntime.last = this;
  }
  graphJson = '';
  videoNodesJson = '[]';
  subscriptions: string[] = [];

  setGraph(graphJson: string): number {
    this.graphJson = graphJson;
    return 1;
  }
  setVideoNodes(nodeIdsJson: string): void { this.videoNodesJson = nodeIdsJson; }

  play(nowNs: unknown): void {
    if (typeof nowNs !== 'bigint') throw new TypeError('WASM u64 timestamp must be a bigint');
    this.lifecycleTimestamps.push(nowNs);
  }
  advance(_inputJson: string): void {}
  subscribeOutput(subscriptionJson: string): void { this.subscriptions.push(subscriptionJson); }
  updateOutputSubscription(subscriptionJson: string): void { this.subscriptions = [subscriptionJson]; }
  unsubscribeOutput(subscriptionId: string): void {
    this.subscriptions = this.subscriptions.filter((item) => JSON.parse(item).subscriptionId !== subscriptionId);
  }
  publishOutput(_stateJson: string): void {}
  executionPlan(): string {
    return JSON.stringify({ revision: 1, sortedIds: [], nodes: [], outputNodes: [], cycle: false });
  }
  drainDeliveries(): string { return JSON.stringify({ deliveries: [], invalidations: [] }); }
  pause(nowNs: unknown): void {
    if (typeof nowNs !== 'bigint') throw new TypeError('WASM u64 timestamp must be a bigint');
    this.lifecycleTimestamps.push(nowNs);
  }
  resume(nowNs: unknown): void {
    if (typeof nowNs !== 'bigint') throw new TypeError('WASM u64 timestamp must be a bigint');
    this.lifecycleTimestamps.push(nowNs);
  }
  stop(): void {}
  dispose(): void {}
}

function fakeBindings(apiVersion = SDK_API_VERSION): RawWasmBindings {
  return {
    default: vi.fn(async () => undefined),
    apiVersion: () => apiVersion,
    capabilities: () => JSON.stringify(capabilities),
    sdkVersion: () => '0.16.0',
    runtimeContract: () => JSON.stringify(runtimeContract),
    parseShader: (code) => JSON.stringify({ code }),
    planGraph: (graphJson) => graphJson,
    Runtime: FakeRuntime,
    Engine: FakeEngine,
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
    expect(client.parseShader<{ code: string }>('shader').code).toBe('shader');
  });

  it('serializes graph snapshots and decodes engine events', async () => {
    const client = await WasmSdkClient.load(async () => fakeBindings());
    const engine = client.createEngine();

    expect(engine.setGraph([], [])).toBe(1);
    expect(engine.state).toBe('ready');
    expect(engine.drainEvents()).toEqual([
      { type: 'graph-ready', revision: 1 },
      { type: 'state', state: 'ready' },
    ]);
    expect(engine.drainEvents()).toEqual([]);
  });

  it('projects the canonical Runtime subscription API without host-specific methods', async () => {
    const client = await WasmSdkClient.load(async () => fakeBindings());
    const runtime = client.createRuntime();
    expect(runtime.setGraph([], [])).toBe(1);
    runtime.setVideoNodes(['video']);
    expect(FakeRuntime.last.videoNodesJson).toBe('["video"]');
    expect(runtime.executionPlan()).toEqual({
      revision: 1,
      sortedIds: [],
      nodes: [],
      outputNodes: [],
      cycle: false,
    });
    runtime.subscribeOutput({
      subscriptionId: 'math',
      output: { nodeId: 'math-1', portId: 'result' },
      delivery: 'on-change',
      transport: 'value',
    });
    expect(runtime.drainDeliveries()).toEqual({ deliveries: [], invalidations: [] });
  });

  it('converts lifecycle timestamps to WASM u64 bigints', async () => {
    const client = await WasmSdkClient.load(async () => fakeBindings());
    const runtime = client.createRuntime();

    runtime.play(123);
    runtime.pause(456);
    runtime.resume(789);

    expect(FakeRuntime.last.lifecycleTimestamps).toEqual([123n, 456n, 789n]);
  });

  it('advances a connected SYSTEM TIME value through the real WASM Runtime', () => {
    const runtime = requireSdk().createRuntime();
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
          type: 'shader', label: 'Hue Rotate',
          shaderCode: '@group(0) @binding(0) var<uniform> angle: f32;\n@fragment fn main() -> @location(0) vec4f { return vec4f(angle); }',
          inputs: [{ id: 'angle', label: 'angle', dataType: 'float', direction: 'input' }],
          outputs: [], uniforms: {},
        },
      },
    ];
    const edges = [{
      id: 'time_to_hue', source: 'time', sourceHandle: 'time_out',
      target: 'hue', targetHandle: 'angle',
    }];

    runtime.setGraph(nodes as never, edges as never);
    runtime.play(1_000_000_000);
    runtime.advance({
      time: 1.5,
      delta: 0,
      frame: 0,
      date: new Float32Array(4),
      mouse: new Float32Array(4),
      resolution: new Float32Array([512, 512, 1]),
    });
    const first = runtime.drainWork<Array<{
      nodeId: string;
      uniforms: Record<string, number[]>;
    }>>().find((command) => command.nodeId === 'hue');

    runtime.advance({
      time: 2.25,
      delta: 0,
      frame: 0,
      date: new Float32Array(4),
      mouse: new Float32Array(4),
      resolution: new Float32Array([512, 512, 1]),
    });
    const second = runtime.drainWork<Array<{
      nodeId: string;
      uniforms: Record<string, number[]>;
    }>>().find((command) => command.nodeId === 'hue');
    runtime.stop();
    runtime.dispose();

    expect(first?.uniforms.angle[0]).toBeCloseTo(0.5);
    expect(second?.uniforms.angle[0]).toBeCloseTo(1.25);
  });

  it('forwards typed frame inputs without frame JSON serialization', async () => {
    const client = await WasmSdkClient.load(async () => fakeBindings());
    const engine = client.createEngine();
    engine.setGraph([], []);
    engine.drainEvents();

    engine.setVideoNodes(['video']);
    engine.runFrame({
      time: 1,
      delta: 1 / 60,
      frame: 42,
      date: new Float32Array([2026, 7, 29, 0]),
      mouse: new Float32Array(4),
      resolution: new Float32Array([640, 360, 1]),
    });

    expect(engine.state).toBe('running');
    expect(engine.lastFrame).toBe(42);
    expect(engine.pendingCommandCount).toBe(1);
    expect(engine.nodeGeneration('video')).toBe(1);
    expect(engine.drainEvents()).toEqual([
      { type: 'frame-planned', frame: 42, revision: 1, commandCount: 1, dirtyNodeCount: 1 },
    ]);
    engine.pause();
    expect(engine.state).toBe('paused');
    engine.resume();
    engine.stop();
    expect(engine.state).toBe('stopped');
  });

  it('rejects unsafe frame numbers before crossing the FFI boundary', async () => {
    const engine = (await WasmSdkClient.load(async () => fakeBindings())).createEngine();
    expect(() => engine.runFrame({
      time: 0,
      delta: 0,
      frame: -1,
      date: new Float32Array(4),
      mouse: new Float32Array(4),
      resolution: new Float32Array(3),
    })).toThrowError(expect.objectContaining({ code: 'invalid-frame' }));
  });

  it('turns structured Rust failures into SdkContractError', async () => {
    const client = await WasmSdkClient.load(async () => fakeBindings());
    const engine = client.createEngine();
    engine.dispose();

    expect(() => engine.markDirty('shader')).toThrowError(
      expect.objectContaining<SdkContractError>({
        code: 'disposed',
        message: 'Engine has been disposed',
        nodeId: 'shader',
      }),
    );
  });

  it('rejects incompatible API versions before creating an engine', async () => {
    await expect(WasmSdkClient.load(async () => fakeBindings(99))).rejects.toMatchObject({
      code: 'protocol-mismatch',
    });
  });
});
