import type { Edge, Node } from '@xyflow/react';
import type { ShaderNodeData } from '../types';
import type { FrameInput, StatefulEngineCore } from './PipelineRuntime';
import {
  SDK_API_VERSION,
  SdkContractError,
  decodeCapabilities,
  decodeEngineEvents,
  decodeRuntimePublicSurface,
  decodeSdkError,
} from './contract';
import type { EngineEvent, EngineState, RuntimePublicSurface, SdkCapabilities } from './contract';

interface RawEngine {
  readonly revision: number;
  readonly lastFrame?: bigint;
  readonly pendingCommandCount: number;
  setGraph(graphJson: string): number;
  markDirty(nodeId: string): void;
  runFrame(
    time: number,
    delta: number,
    frame: bigint,
    date: Float32Array,
    mouse: Float32Array,
    resolution: Float32Array,
  ): void;
  setVideoNodes(nodeIdsJson: string): void;
  nodeGeneration(nodeId: string): number;
  pause(): void;
  resume(): void;
  stop(): void;
  engineState(): string;
  drainEvents(): string;
  dispose(): void;
}

interface RawEngineConstructor {
  new(): RawEngine;
}

export interface RawWasmBindings {
  default(input?: unknown): Promise<unknown>;
  apiVersion(): number;
  capabilities(): string;
  sdkVersion(): string;
  runtimeContract(): string;
  parseShader(code: string): string;
  planGraph(graphJson: string): string;
  Engine: RawEngineConstructor;
}

export type WasmModuleLoader = () => Promise<RawWasmBindings>;

const SDK_MODULE_URL = '/open_quartz-sdk/open_quartz.js';

// The generated browser-only module exists only after `npm run build:sdk`;
// loader injection keeps Node contract tests independent of generated artifacts.
async function defaultLoader(): Promise<RawWasmBindings> {
  // A fully-qualified runtime URL keeps Vite from treating the generated
  // public WASM package as a source-module dependency during development.
  const moduleUrl = new URL(SDK_MODULE_URL, globalThis.location.href).href;
  return await import(/* @vite-ignore */ moduleUrl) as RawWasmBindings;
}

function invoke<T>(operation: () => T): T {
  try {
    return operation();
  } catch (error) {
    throw decodeSdkError(error);
  }
}

/** Stage A typed wrapper over the generated wasm-bindgen module. */
export class WasmSdkClient {
  readonly capabilities: SdkCapabilities;
  readonly sdkVersion: string;
  readonly runtimeContract: RuntimePublicSurface;
  private readonly bindings: RawWasmBindings;

  private constructor(
    bindings: RawWasmBindings,
    capabilities: SdkCapabilities,
    runtimeContract: RuntimePublicSurface,
  ) {
    this.bindings = bindings;
    this.capabilities = capabilities;
    this.runtimeContract = runtimeContract;
    this.sdkVersion = bindings.sdkVersion();
  }

  static async load(loader: WasmModuleLoader = defaultLoader): Promise<WasmSdkClient> {
    const bindings = await loader();
    await bindings.default();
    const actualVersion = bindings.apiVersion();
    if (actualVersion !== SDK_API_VERSION) {
      throw new SdkContractError({
        code: 'protocol-mismatch',
        message: `Rust SDK API version ${actualVersion} does not match UI version ${SDK_API_VERSION}`,
      });
    }
    return new WasmSdkClient(
      bindings,
      decodeCapabilities(bindings.capabilities()),
      decodeRuntimePublicSurface(bindings.runtimeContract()),
    );
  }

  createEngine(): WasmEngineContract {
    return new WasmEngineContract(new this.bindings.Engine());
  }

  parseShader<T = unknown>(code: string): T {
    return JSON.parse(this.bindings.parseShader(code)) as T;
  }

  planGraph<T = unknown>(graph: unknown): T {
    return JSON.parse(invoke(() => this.bindings.planGraph(JSON.stringify(graph)))) as T;
  }
}

/** Stateful graph/frame contract. GPU commands remain internal until Stage D. */
export class WasmEngineContract implements StatefulEngineCore {
  private readonly raw: RawEngine;

  constructor(raw: RawEngine) {
    this.raw = raw;
  }

  get revision(): number {
    return this.raw.revision;
  }

  get state(): EngineState {
    return this.raw.engineState() as EngineState;
  }

  get lastFrame(): number | null {
    return this.raw.lastFrame === undefined ? null : Number(this.raw.lastFrame);
  }

  get pendingCommandCount(): number {
    return this.raw.pendingCommandCount;
  }

  setGraph(nodes: Node<ShaderNodeData>[], edges: Edge[]): number {
    return invoke(() => this.raw.setGraph(JSON.stringify({ nodes, edges })));
  }

  markDirty(nodeId: string): void {
    invoke(() => this.raw.markDirty(nodeId));
  }

  runFrame(input: FrameInput): void {
    if (!Number.isSafeInteger(input.frame) || input.frame < 0) {
      throw new SdkContractError({
        code: 'invalid-frame',
        message: 'Frame number must be a non-negative safe integer',
      });
    }
    invoke(() => this.raw.runFrame(
      input.time,
      input.delta,
      BigInt(input.frame),
      input.date,
      input.mouse,
      input.resolution,
    ));
  }

  setVideoNodes(nodeIds: readonly string[]): void {
    invoke(() => this.raw.setVideoNodes(JSON.stringify(nodeIds)));
  }

  nodeGeneration(nodeId: string): number {
    return invoke(() => this.raw.nodeGeneration(nodeId));
  }

  pause(): void {
    invoke(() => this.raw.pause());
  }

  resume(): void {
    invoke(() => this.raw.resume());
  }

  stop(): void {
    invoke(() => this.raw.stop());
  }

  drainEvents(): EngineEvent[] {
    return decodeEngineEvents(this.raw.drainEvents());
  }

  dispose(): void {
    this.raw.dispose();
  }
}
