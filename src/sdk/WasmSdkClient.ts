import type { ShaderNodeData } from '../types';
import type { FrameInput } from './internal/hostTypes';
import { SDK_API_VERSION, SdkContractError, decodeCapabilities, decodeRuntimePublicSurface, decodeSdkError } from './contract';
import type {
  OutputDeliveryBatch,
  OutputSubscription,
  RuntimePublicSurface,
  SdkCapabilities,
} from './contract';
import type { CatalogSnapshot } from './catalog';

export interface RawGraph {
  readonly revision: number;
  free?: () => void;
  snapshotJSON(): string;
  initialize(graphJson: string): void;
  replace(graphJson: string, expectedRevision: number): string;
  rollback(expectedRevision: number): string;
  redo(expectedRevision: number): string;
  apply(commandJson: string, expectedRevision: number): string;
  canConnect(
    sourceNodeId: string,
    sourcePortId: string,
    targetNodeId: string,
    targetPortId: string,
  ): void;
  createNode(factoryJson: string, expectedRevision: number): string;
  nodeJSON(nodeId: string): string | undefined;
}

export interface RawProject {
  name: string;
  free?: () => void;
  set_name?: (name: string) => void;
  toJSON(): string;
  graph(): RawGraph;
  screenSaverGraph?: (rendererNodeId: string, width: number, height: number) => string;
  createPlayer(): unknown;
}

export interface RawOpenQuartz {
  new(): RawOpenQuartzInstance;
}

export interface RawOpenQuartzInstance {
  createProject(name: string): RawProject;
  openProject(projectJson: string): RawProject;
  normalizeProject?: (projectJson: string) => string;
  screenSaverExportProject?: (projectJson: string, rendererNodeId: string) => string;
}

interface RawBrowserPlayer {
  setGraph(graphJson: string): number;
  play(nowNs: bigint): void;
  pause(nowNs: bigint): void;
  resume(nowNs: bigint): void;
  stop(): void;
  uploadFrame(nodeId: string, bitmap: ImageBitmap, timestampNs: bigint): void;
  uploadRgba(nodeId: string, rgba: Uint8Array, width: number, height: number): void;
  outputInfo(nodeId: string): string;
  readOutputRgba(nodeId: string): Promise<Uint8Array>;
  frame(inputJson: string): string;
  subscribeOutput(subscriptionJson: string): void;
  unsubscribeOutput(subscriptionId: string): void;
  submitCompletion(completionJson: string): void;
  drainDeliveries(): string;
  close(): void;
}

type BrowserGraphNode = {
  id: string;
  type?: string;
  position: { x: number; y: number };
  data: ShaderNodeData;
};

type BrowserGraphEdge = {
  id: string;
  source: string;
  sourceHandle?: string | null;
  target: string;
  targetHandle?: string | null;
};

interface RawBrowserPlayerConstructor {
  create(canvas: OffscreenCanvas): Promise<RawBrowserPlayer>;
}

export interface RawWasmBindings {
  default(input?: unknown): Promise<unknown>;
  apiVersion(): number;
  capabilities(): string;
  sdkVersion(): string;
  runtimeContract(): string;
  catalog(): string;
  planHostResourceIntents(requestJson: string): string;
  planBrowserOnnxTask(requestJson: string): string;
  encodeBrowserOnnxInput(rgba: Uint8Array, requestJson: string): string;
  decodeBrowserOnnxOutput(sourceRgba: Uint8Array, raw: Float32Array, requestJson: string): string;
  buildBrowserOnnxCompletion(requestJson: string): string;
  parseShader(code: string): string;
  planGraph(graphJson: string): string;
  BrowserPlayer: RawBrowserPlayerConstructor;
  OpenQuartz?: RawOpenQuartz;
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

function toWasmU64(value: number): bigint {
  const rounded = Math.round(value);
  if (!Number.isSafeInteger(rounded) || rounded < 0) {
    throw new RangeError(`WASM u64 value must be a non-negative safe integer, received ${value}`);
  }
  return BigInt(rounded);
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


  async createBrowserPlayer(canvas: OffscreenCanvas): Promise<WasmBrowserPlayerContract> {
    const raw = await invoke(() => this.bindings.BrowserPlayer.create(canvas));
    return new WasmBrowserPlayerContract(raw);
  }

  createProject(name: string): RawProject {
    if (!this.bindings.OpenQuartz) {
      throw new SdkContractError({
        code: 'invalid-response',
        message: 'Rust SDK aggregate bindings are unavailable',
      });
    }
    return invoke(() => new this.bindings.OpenQuartz!().createProject(name));
  }

  openProject(projectJson: string): RawProject {
    if (!this.bindings.OpenQuartz) {
      throw new SdkContractError({
        code: 'invalid-response',
        message: 'Rust SDK aggregate bindings are unavailable',
      });
    }
    return invoke(() => new this.bindings.OpenQuartz!().openProject(projectJson));
  }

  normalizeProject(projectJson: string): string {
    if (!this.bindings.OpenQuartz) {
      throw new SdkContractError({
        code: 'invalid-response',
        message: 'Rust SDK aggregate bindings are unavailable',
      });
    }
    const sdk = new this.bindings.OpenQuartz!();
    if (!sdk.normalizeProject) {
      throw new SdkContractError({
        code: 'invalid-response',
        message: 'Rust SDK project normalization binding is unavailable',
      });
    }
    return invoke(() => sdk.normalizeProject!(projectJson));
  }

  screenSaverExportProject(projectJson: string, rendererNodeId: string): string {
    if (!this.bindings.OpenQuartz) {
      throw new SdkContractError({
        code: 'invalid-response',
        message: 'Rust SDK aggregate bindings are unavailable',
      });
    }
    const sdk = new this.bindings.OpenQuartz!();
    if (!sdk.screenSaverExportProject) {
      throw new SdkContractError({
        code: 'invalid-response',
        message: 'Rust SDK screen saver export binding is unavailable',
      });
    }
    return invoke(() => sdk.screenSaverExportProject!(projectJson, rendererNodeId));
  }

  parseShader<T = unknown>(code: string): T {
    return JSON.parse(this.bindings.parseShader(code)) as T;
  }

  planGraph<T = unknown>(graph: unknown): T {
    return JSON.parse(invoke(() => this.bindings.planGraph(JSON.stringify(graph)))) as T;
  }

  planHostResourceIntents<T = unknown>(request: unknown): T {
    return JSON.parse(invoke(() => this.bindings.planHostResourceIntents(JSON.stringify(request)))) as T;
  }

  catalog(): CatalogSnapshot {
    return JSON.parse(invoke(() => this.bindings.catalog())) as CatalogSnapshot;
  }

  planBrowserOnnxTask<T = unknown>(request: unknown): T {
    return JSON.parse(invoke(() => this.bindings.planBrowserOnnxTask(JSON.stringify(request)))) as T;
  }

  encodeBrowserOnnxInput<T = unknown>(rgba: Uint8Array, request: unknown): T {
    return JSON.parse(invoke(() => this.bindings.encodeBrowserOnnxInput(rgba, JSON.stringify(request)))) as T;
  }

  decodeBrowserOnnxOutput<T = unknown>(
    sourceRgba: Uint8Array,
    raw: Float32Array,
    request: unknown,
  ): T {
    return JSON.parse(invoke(() => this.bindings.decodeBrowserOnnxOutput(sourceRgba, raw, JSON.stringify(request)))) as T;
  }

  buildBrowserOnnxCompletion<T = unknown>(request: unknown): T {
    return JSON.parse(invoke(() => this.bindings.buildBrowserOnnxCompletion(JSON.stringify(request)))) as T;
  }
}

interface RawClockState {
  epoch: number;
  timelineNs: number;
  previousTimelineNs: number;
  frame: number;
  nextDeadlineNs: number;
}

export interface BrowserFrameResult<TTask = unknown> {
  clock: RawClockState;
  inferenceTasks: TTask[];
}

export class WasmBrowserPlayerContract {
  private readonly raw: RawBrowserPlayer;

  constructor(raw: RawBrowserPlayer) {
    this.raw = raw;
  }

  setGraph(nodes: BrowserGraphNode[], edges: BrowserGraphEdge[]): number {
    return invoke(() => this.raw.setGraph(JSON.stringify({ nodes, edges })));
  }

  play(nowNs: number): void { invoke(() => this.raw.play(toWasmU64(nowNs))); }
  pause(nowNs: number): void { invoke(() => this.raw.pause(toWasmU64(nowNs))); }
  resume(nowNs: number): void { invoke(() => this.raw.resume(toWasmU64(nowNs))); }
  stop(): void { invoke(() => this.raw.stop()); }

  uploadFrame(nodeId: string, bitmap: ImageBitmap, timestampNs: number): void {
    invoke(() => this.raw.uploadFrame(nodeId, bitmap, toWasmU64(timestampNs)));
  }

  uploadRgba(nodeId: string, rgba: Uint8Array, width: number, height: number): void {
    invoke(() => this.raw.uploadRgba(nodeId, rgba, width, height));
  }

  outputInfo(nodeId: string): { width: number; height: number } {
    return JSON.parse(invoke(() => this.raw.outputInfo(nodeId))) as { width: number; height: number };
  }

  readOutputRgba(nodeId: string): Promise<Uint8Array> {
    return this.raw.readOutputRgba(nodeId);
  }

  frame<TTask = unknown>(input: FrameInput): BrowserFrameResult<TTask> {
    return JSON.parse(invoke(() => this.raw.frame(JSON.stringify({
      nowNs: Math.round(input.time * 1_000_000_000),
      date: Array.from(input.date),
      mouse: Array.from(input.mouse),
      resolution: Array.from(input.resolution),
    }))));
  }

  subscribeOutput(subscription: OutputSubscription): void {
    invoke(() => this.raw.subscribeOutput(JSON.stringify(subscription)));
  }

  unsubscribeOutput(subscriptionId: string): void {
    invoke(() => this.raw.unsubscribeOutput(subscriptionId));
  }

  submitCompletion(completion: unknown): void {
    invoke(() => this.raw.submitCompletion(JSON.stringify(completion)));
  }
  drainDeliveries(): OutputDeliveryBatch {
    return JSON.parse(invoke(() => this.raw.drainDeliveries())) as OutputDeliveryBatch;
  }

  close(): void { invoke(() => this.raw.close()); }
}
