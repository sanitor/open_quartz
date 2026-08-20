import type { ProjectFile } from '../types';
import {
  prepareCatalogOnnx,
  prepareCustomOnnx,
  type PreparedOnnxModel,
  loadOnnxModel,
} from './internal/OnnxResourceRegistry';
import { getOnnxModelDescriptor, type OnnxModelDescriptor } from './catalog';
import { checkIsTauri } from '../utils/tauri';
import { BrowserHost } from './internal/BrowserHost';
import { NativeHost } from './internal/NativeHost';
import type {
  PlayerHost,
  PlayerHostEvents,
  RuntimeFrame,
  RuntimeVideoDevice,
} from './internal/hostTypes';
import { requireSdk } from './runtime';
import type { RawGraph, RawProject } from './WasmSdkClient';
import { SdkContractError, decodeSdkError } from './contract';

export interface GraphChange {
  revision: number;
  changedNodes?: readonly string[];
}

type GraphNode = Omit<ProjectFile['graph']['nodes'][number], 'type'> & { type?: string };
type GraphEdge = Omit<ProjectFile['graph']['edges'][number], 'sourceHandle' | 'targetHandle'> & {
  sourceHandle?: string | null;
  targetHandle?: string | null;
};
type GraphSnapshot = { nodes: GraphNode[]; edges: GraphEdge[] };

export type GraphCommand =
  | { kind: 'addNode'; node: GraphNode }
  | { kind: 'updateNode'; node: GraphNode }
  | { kind: 'updateNodeData'; nodeId: string; data: ProjectFile['graph']['nodes'][number]['data'] }
  | { kind: 'updateShaderCode'; nodeId: string; shaderCode: string }
  | { kind: 'updateInputType'; nodeId: string; dataType: string; inputMode?: string }
  | {
      kind: 'updateNodePorts';
      nodeId: string;
      inputs: ProjectFile['graph']['nodes'][number]['data']['inputs'];
      outputs: ProjectFile['graph']['nodes'][number]['data']['outputs'];
    }
  | { kind: 'setNodePosition'; nodeId: string; position: { x: number; y: number } }
  | { kind: 'removeNode'; nodeId: string }
  | { kind: 'connect'; source: { nodeId: string; portId: string }; target: { nodeId: string; portId: string } }
  | { kind: 'disconnect'; edgeId: string };

export type NodeFactoryRequest =
  | {
      kind: 'shader';
      position?: { x: number; y: number };
      code: string;
      label: string;
      templateName?: string;
      shaderTemplateId?: string;
    }
  | {
      kind: 'input';
      position?: { x: number; y: number };
      dataType: string;
      inputMode?: string;
    }
  | { kind: 'system'; position?: { x: number; y: number }; source: string }
  | { kind: 'constant'; position?: { x: number; y: number } }
  | { kind: 'math'; position?: { x: number; y: number }; op: string }
  | { kind: 'renderer'; position?: { x: number; y: number } }
  | {
      kind: 'onnx';
      position?: { x: number; y: number };
      label: string;
      templateName?: string;
      modelId?: string;
      catalogId?: string;
      inputs?: ProjectFile['graph']['nodes'][number]['data']['inputs'];
      outputs?: ProjectFile['graph']['nodes'][number]['data']['outputs'];
    }
  | { kind: 'customOnnx'; position?: { x: number; y: number } };

export class Port {
  readonly nodeId: string;
  readonly id: string;
  readonly label: string;
  readonly dataType: string;
  readonly direction: 'input' | 'output';

  constructor(
    nodeId: string,
    id: string,
    label: string,
    dataType: string,
    direction: 'input' | 'output',
  ) {
    this.nodeId = nodeId;
    this.id = id;
    this.label = label;
    this.dataType = dataType;
    this.direction = direction;
  }
}

export class Node {
  private readonly value: GraphNode;

  constructor(value: GraphNode) {
    this.value = value;
  }

  get id(): string { return this.value.id; }
  get type(): string { return this.value.data.type; }
  get label(): string { return this.value.data.label; }
  get position(): Readonly<{ x: number; y: number }> { return this.value.position; }
  get inputs(): Port[] {
    return this.value.data.inputs.map(
      (port) => new Port(this.id, port.id, port.label, port.dataType, 'input'),
    );
  }
  get outputs(): Port[] {
    return this.value.data.outputs.map(
      (port) => new Port(this.id, port.id, port.label, port.dataType, 'output'),
    );
  }
  toFlowNode(): GraphNode { return structuredClone(this.value); }
}

export class Graph {
  private snapshotValue: GraphSnapshot;
  private readonly raw: RawGraph;
  private closed = false;

  constructor(raw: RawGraph) {
    this.raw = raw;
    this.snapshotValue = decodeSnapshot(raw.snapshotJSON());
  }

  get revision(): number {
    this.ensureOpen();
    return this.raw.revision;
  }
  get nodes(): Node[] {
    this.ensureOpen();
    return this.snapshotValue.nodes.map((node) => new Node(node));
  }
  get edges(): readonly GraphEdge[] {
    this.ensureOpen();
    return this.snapshotValue.edges;
  }

  snapshot(): GraphSnapshot {
    this.ensureOpen();
    return {
      nodes: structuredClone(this.snapshotValue.nodes),
      edges: structuredClone(this.snapshotValue.edges),
    };
  }

  replace(nodes: GraphNode[], edges: GraphEdge[], expectedRevision = this.revision): GraphChange {
    this.ensureOpen();
    const wireNodes = nodes.map((node) => ({
      ...node,
      type: node.type ?? node.data.type,
    }));
    const wireEdges = edges.map((edge) => ({
      ...edge,
      sourceHandle: edge.sourceHandle ?? '',
      targetHandle: edge.targetHandle ?? '',
    }));
    const change = decodeChange(call(() => this.raw.replace(
      JSON.stringify({ nodes: wireNodes, edges: wireEdges }),
      expectedRevision,
    )));
    this.snapshotValue = decodeSnapshot(this.raw.snapshotJSON());
    return change;
  }

  initialize(nodes: GraphNode[], edges: GraphEdge[]): void {
    this.ensureOpen();
    const wireNodes = nodes.map((node) => ({
      ...node,
      type: node.type ?? node.data.type,
    }));
    const wireEdges = edges.map((edge) => ({
      ...edge,
      sourceHandle: edge.sourceHandle ?? '',
      targetHandle: edge.targetHandle ?? '',
    }));
    call(() => this.raw.initialize(JSON.stringify({ nodes: wireNodes, edges: wireEdges })));
    this.snapshotValue = decodeSnapshot(this.raw.snapshotJSON());
  }

  rollback(): GraphChange {
    this.ensureOpen();
    const change = decodeChange(call(() => this.raw.rollback(this.revision)));
    this.snapshotValue = decodeSnapshot(this.raw.snapshotJSON());
    return change;
  }

  redo(): GraphChange {
    this.ensureOpen();
    const change = decodeChange(call(() => this.raw.redo(this.revision)));
    this.snapshotValue = decodeSnapshot(this.raw.snapshotJSON());
    return change;
  }

  apply(command: GraphCommand, expectedRevision = this.revision): GraphChange {
    this.ensureOpen();
    const change = decodeChange(call(() => this.raw.apply(
      JSON.stringify(command),
      expectedRevision,
    )));
    this.snapshotValue = decodeSnapshot(this.raw.snapshotJSON());
    return change;
  }

  canConnect(
    sourceNodeId: string,
    sourcePortId: string,
    targetNodeId: string,
    targetPortId: string,
  ): boolean {
    this.ensureOpen();
    try {
      call(() => this.raw.canConnect(sourceNodeId, sourcePortId, targetNodeId, targetPortId));
      return true;
    } catch {
      return false;
    }
  }

  createNode(request: NodeFactoryRequest, expectedRevision = this.revision): Node {
    this.ensureOpen();
    const response = JSON.parse(call(() => this.raw.createNode(
      JSON.stringify(request),
      expectedRevision,
    ))) as { node?: GraphNode };
    this.snapshotValue = decodeSnapshot(this.raw.snapshotJSON());
    const nodeId = response.node?.id;
    const node = nodeId ? this.snapshotValue.nodes.find((candidate) => candidate.id === nodeId) : undefined;
    if (!node) {
      throw new SdkContractError({
        code: 'invalid-response',
        message: 'Rust SDK did not return the created graph node',
      });
    }
    return new Node(node);
  }

  node(nodeId: string): Node | undefined {
    this.ensureOpen();
    const value = this.snapshotValue.nodes.find((node) => node.id === nodeId);
    return value ? new Node(value) : undefined;
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    this.raw.free?.();
  }

  dispose(): void { this.close(); }

  private ensureOpen(): void {
    if (this.closed) {
      throw new SdkContractError({
        code: 'disposed',
        message: 'Graph has been closed',
      });
    }
  }
}

export class Resource {
  readonly id: string;
  readonly kind: string;
  readonly source: Readonly<Record<string, unknown>>;

  constructor(id: string, kind: string, source: Readonly<Record<string, unknown>>) {
    this.id = id;
    this.kind = kind;
    this.source = source;
  }

  static prepareCatalogOnnx(
    entry: OnnxModelDescriptor,
    onProgress?: (progress: number) => void,
  ): Promise<PreparedOnnxModel> {
    return prepareCatalogOnnx(entry, onProgress);
  }

  static prepareCustomOnnx(modelId: string, buffer: ArrayBuffer): Promise<PreparedOnnxModel> {
    return prepareCustomOnnx(modelId, buffer);
  }
}

export class Project {
  private readonly raw: RawProject;
  readonly graph: Graph;
  readonly resources: Resource[];
  private closed = false;

  constructor(
    name: string,
    nodes: GraphNode[] = [],
    edges: GraphEdge[] = [],
    resources: Resource[] = [],
  ) {
    this.raw = requireSdk().createProject(name);
    this.graph = new Graph(this.raw.graph());
    this.resources = resources;
    if (nodes.length > 0 || edges.length > 0) {
      this.graph.initialize(nodes, edges);
    }
  }

  static fromRaw(raw: RawProject): Project {
    const project = Object.create(Project.prototype) as Project;
    Object.assign(project, {
      raw,
      graph: new Graph(raw.graph()),
      resources: [],
      closed: false,
    });
    return project;
  }

  get name(): string {
    this.ensureOpen();
    return this.raw.name;
  }
  set name(value: string) {
    this.ensureOpen();
    if (this.raw.set_name) this.raw.set_name(value);
    else this.raw.name = value;
  }

  toFile(): ProjectFile {
    this.ensureOpen();
    return JSON.parse(this.raw.toJSON()) as ProjectFile;
  }

  toJSON(): string {
    this.ensureOpen();
    return this.raw.toJSON();
  }

  screenSaverGraph(rendererNodeId: string, width: number, height: number): GraphSnapshot {
    this.ensureOpen();
    if (!this.raw.screenSaverGraph) {
      throw new SdkContractError({
        code: 'invalid-response',
        message: 'Rust SDK screen saver graph transform is unavailable',
      });
    }
    return decodeSnapshot(call(() => this.raw.screenSaverGraph!(rendererNodeId, width, height)));
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    this.graph.close();
    this.raw.free?.();
  }

  dispose(): void { this.close(); }

  private ensureOpen(): void {
    if (this.closed) {
      throw new SdkContractError({
        code: 'disposed',
        message: 'Project has been closed',
      });
    }
  }
}

export interface PlayerEvents extends PlayerHostEvents {
  onRendererFrame?: (nodeId: string, frame: { rgba: Uint8Array; width: number; height: number }) => void;
  onRendererStream?: (nodeId: string, video: HTMLVideoElement | null) => void;
  onRendererVideoFrame?: (nodeId: string) => void;
  onRendererCadence?: (nodeId: string, cadence: {
    callbackFps: number;
    presentedFps: number;
    displayedFps: number;
    droppedFps: number;
    dropRatio: number;
    mediaRate: number;
    callbackP50Ms: number;
    callbackP95Ms: number;
    callbackMaxMs: number;
    presentedBurstP95: number;
    presentedBurstMax: number;
  }) => void;
  onNativeBackendDetected?: (nodeId: string, backend: 'cpu' | 'directml' | 'directml+cpu') => void;
}

export class Subscription {
  private active = true;
  private readonly cancel: () => void;

  constructor(cancel: () => void) {
    this.cancel = cancel;
  }

  close(): void {
    if (!this.active) return;
    this.active = false;
    this.cancel();
  }
}

export class Output {
  readonly nodeId: string;
  private readonly player: Player;

  constructor(nodeId: string, player: Player) {
    this.nodeId = nodeId;
    this.player = player;
  }

  capture(): Promise<string | null> { return this.player.capture(this.nodeId); }

  subscribe(listener: (dataUrl: string) => void): Subscription {
    return this.player.subscribeOutput(this.nodeId, listener);
  }
}

export interface PlayerOptions {
  canvas: HTMLCanvasElement;
  events?: PlayerEvents;
}

export class Player {
  private readonly project: Project;
  private readonly host: PlayerHost;
  private readonly outputListeners: Map<string, Set<(dataUrl: string) => void>>;

  private constructor(
    project: Project,
    host: PlayerHost,
    outputListeners: Map<string, Set<(dataUrl: string) => void>>,
  ) {
    this.project = project;
    this.host = host;
    this.outputListeners = outputListeners;
  }

  static async create(project: Project, options: PlayerOptions): Promise<Player> {
    const listeners = new Map<string, Set<(dataUrl: string) => void>>();
    const events = options.events ?? {};
    const onOutput = (nodeId: string, dataUrl: string): void => {
      events.onOutput?.(nodeId, dataUrl);
      for (const listener of listeners.get(nodeId) ?? []) listener(dataUrl);
    };
    const callbacks: PlayerHostEvents = { ...events, onOutput };
    const host: PlayerHost = await checkIsTauri()
      ? new NativeHost({
        onFrame: (frame) => events.onFrame?.({
          frame: frame.frame,
          time: Number.NaN,
          fps: Number.NaN,
        }),
        onRendererFrame: events.onRendererFrame,
        onRendererStream: events.onRendererStream,
        onRendererVideoFrame: events.onRendererVideoFrame,
        onRendererCadence: events.onRendererCadence,
        onError: (error) => events.onNodeError?.(null, error),
        onOutput,
        onOutputSize: events.onOutputSize,
        onOutputData: events.onOutputData,
        onBackendDetected: events.onBackendDetected
          ? (nodeId) => events.onBackendDetected?.(nodeId, 'native')
          : undefined,
        onNativeBackendDetected: events.onNativeBackendDetected,
      }, undefined, undefined, true)
      : new BrowserHost(callbacks);
    await host.initialize(options.canvas);
    if (host.registerOnnxModel) {
      const { nodes } = project.graph.snapshot();
      for (const node of nodes) {
        if (node.data.type !== 'onnx') continue;
        const modelId = node.data.onnxCatalogId ?? node.data.onnxModelId;
        if (!modelId) continue;
        try {
          const entry = node.data.onnxCatalogId ? getOnnxModelDescriptor(node.data.onnxCatalogId) : undefined;
          const buffer = await loadOnnxModel(modelId, entry, node.data.onnxCustomPath);
          await host.registerOnnxModel(modelId, buffer.slice(0));
        } catch (error) {
          events.onNodeError?.(node.id, error instanceof Error ? error.message : String(error));
        }
      }
    }
    return new Player(project, host, listeners);
  }

  async play(): Promise<void> {
    const { nodes, edges } = this.project.graph.snapshot();
    await this.host.play(
      nodes as Parameters<PlayerHost['play']>[0],
      edges as Parameters<PlayerHost['play']>[1],
    );
  }

  async apply(graph = this.project.graph): Promise<void> {
    const { nodes, edges } = graph.snapshot();
    await this.host.updateGraph(
      nodes as Parameters<PlayerHost['updateGraph']>[0],
      edges as Parameters<PlayerHost['updateGraph']>[1],
    );
  }

  async pause(): Promise<void> { await this.host.pause(); }
  async resume(): Promise<void> { await this.host.resume(); }
  async stop(): Promise<void> { await this.host.stop(); }
  async close(): Promise<void> { await this.host.close(); }
  setPreview(output: Output | null): void { this.host.setPreviewNode(output?.nodeId ?? null); }
  refreshPreview(): void { this.host.requestPreviewRefresh?.(); }
  capture(nodeId: string): Promise<string | null> { return this.host.captureScreenshot(nodeId); }

  async registerOnnxModel(modelId: string, buffer: ArrayBuffer): Promise<void> {
    await this.host.registerOnnxModel?.(modelId, buffer);
  }

  output(nodeId: string): Output { return new Output(nodeId, this); }

  listVideoDevices(): Promise<RuntimeVideoDevice[]> {
    return this.host.listVideoDevices?.() ?? Promise.resolve([]);
  }

  subscribeOutput(nodeId: string, listener: (dataUrl: string) => void): Subscription {
    const listeners = this.outputListeners.get(nodeId) ?? new Set<(dataUrl: string) => void>();
    listeners.add(listener);
    this.outputListeners.set(nodeId, listeners);
    return new Subscription(() => {
      listeners.delete(listener);
      if (listeners.size === 0) this.outputListeners.delete(nodeId);
    });
  }
}

export class OpenQuartzClient {
  createProject(name = 'Untitled'): Project { return new Project(name); }

  async openProject(projectJson: string): Promise<Project> {
    return Project.fromRaw(requireSdk().openProject(projectJson));
  }

  normalizeProject(projectJson: string): string {
    return requireSdk().normalizeProject(projectJson);
  }

  screenSaverExportProject(projectJson: string, rendererNodeId: string): string {
    return requireSdk().screenSaverExportProject(projectJson, rendererNodeId);
  }

  async listVideoDevices(): Promise<RuntimeVideoDevice[]> {
    if (await checkIsTauri()) return await new NativeHost().listVideoDevices();
    const devices = await navigator.mediaDevices?.enumerateDevices?.() ?? [];
    return devices
      .filter((device) => device.kind === 'videoinput')
      .map((device, index) => ({
        id: device.deviceId,
        label: device.label || `Camera ${index + 1}`,
      }));
  }

  player(project: Project, options: PlayerOptions): Promise<Player> {
    return Player.create(project, options);
  }
}

function decodeSnapshot(json: string): GraphSnapshot {
  const value = JSON.parse(json) as GraphSnapshot;
  if (!value || !Array.isArray(value.nodes) || !Array.isArray(value.edges)) {
    throw new Error('Rust SDK returned an invalid graph snapshot');
  }
  return {
    nodes: structuredClone(value.nodes),
    edges: structuredClone(value.edges),
  };
}

function decodeChange(json: string): GraphChange {
  const value = JSON.parse(json) as GraphChange;
  if (!value || typeof value.revision !== 'number') {
    throw new Error('Rust SDK returned an invalid graph change');
  }
  return {
    revision: value.revision,
    changedNodes: Array.isArray(value.changedNodes) ? value.changedNodes : [],
  };
}

function call<T>(operation: () => T): T {
  try {
    return operation();
  } catch (error) {
    throw decodeSdkError(error);
  }
}

export type { RuntimeFrame };
