import * as THREE from 'three';
import type { Node, Edge } from '@xyflow/react';
import type { ShaderNodeData } from '../types';
import { Clock, type TimeState } from './clock';
import { MouseState } from './mouseState';
import { Compositor, type FrameInputs } from './compositor';
import { VideoSource, type VideoSourceConfig } from './videoSource';

export type HostState = 'stopped' | 'playing' | 'paused';

export interface HostCallbacks {
  onFrame?: (time: TimeState) => void;
  onOutput?: (nodeId: string, dataUrl: string) => void;
  onNodeError?: (nodeId: string, error: string) => void;
  onOutputSize?: (nodeId: string, w: number, h: number) => void;
  onStateChange?: (state: HostState) => void;
}

export class RealtimeHost {
  private compositor: Compositor;
  private clock = new Clock();
  private mouse = new MouseState();
  private rafId: number | null = null;
  private state: HostState = 'stopped';
  private callbacks: HostCallbacks;
  private nodes: Node<ShaderNodeData>[] = [];
  private edges: Edge[] = [];
  private resolution = new Float32Array(3); // [w, h, pixelRatio]
  private videoSources = new Map<string, VideoSource>();
  private videoTextures = new Map<string, THREE.Texture>();
  private needsRecompile = false;
  private activeRendererId: string | null = null;

  constructor(canvas: HTMLCanvasElement, callbacks: HostCallbacks) {
    this.compositor = new Compositor(canvas);
    this.callbacks = callbacks;
  }

  play(nodes: Node<ShaderNodeData>[], edges: Edge[]): void {
    this.nodes = nodes;
    this.edges = edges;
    this.compositor.prepare(
      nodes,
      edges,
      this.callbacks.onNodeError,
      this.callbacks.onOutputSize,
    );
    void this.reconcileVideoSources(nodes);
    this.clock.start();
    this.mouse.attach(document.body);
    this.state = 'playing';
    this.callbacks.onStateChange?.('playing');

    const frame = (now: DOMHighResTimeStamp): void => {
      if (this.state === 'stopped') return;
      this.tick(now);
      this.rafId = requestAnimationFrame(frame);
    };
    this.rafId = requestAnimationFrame(frame);
  }

  pause(): void {
    this.clock.pause();
    for (const source of this.videoSources.values()) source.pause();
    this.state = 'paused';
    this.callbacks.onStateChange?.('paused');
  }

  resume(): void {
    this.clock.resume();
    for (const source of this.videoSources.values()) source.play();
    this.state = 'playing';
    this.callbacks.onStateChange?.('playing');
  }

  stop(): void {
    this.state = 'stopped';
    if (this.rafId !== null) {
      cancelAnimationFrame(this.rafId);
      this.rafId = null;
    }
    this.clock.reset();
    this.mouse.detach();
    for (const source of this.videoSources.values()) source.dispose();
    this.videoSources.clear();
    this.videoTextures.clear();
    this.compositor.dispose();
    this.callbacks.onStateChange?.('stopped');
  }

  updateGraph(nodes: Node<ShaderNodeData>[], edges: Edge[]): void {
    this.nodes = nodes;
    this.edges = edges;
    this.needsRecompile = true;
    void this.reconcileVideoSources(nodes);
  }

  setActiveRenderer(id: string | null): void {
    this.activeRendererId = id;
  }

  async addVideoSource(nodeId: string, config: VideoSourceConfig): Promise<void> {
    this.removeVideoSource(nodeId);
    const source = new VideoSource(config);
    await source.init();
    this.videoSources.set(nodeId, source);
    const texture = source.getTexture();
    if (texture) this.videoTextures.set(nodeId, texture);
  }

  removeVideoSource(nodeId: string): void {
    const source = this.videoSources.get(nodeId);
    if (source) source.dispose();
    this.videoSources.delete(nodeId);
    this.videoTextures.delete(nodeId);
  }

  private async reconcileVideoSources(nodes: Node<ShaderNodeData>[]): Promise<void> {
    const wanted = new Set<string>();
    for (const node of nodes) {
      if (node.data.type !== 'input' || node.data.inputMode !== 'video') continue;
      wanted.add(node.id);
      if (this.videoSources.has(node.id)) continue;
      const config: VideoSourceConfig = {
        type: node.data.videoSourceType ?? 'file',
        url: node.data.videoUrl,
        deviceId: node.data.videoDeviceId,
        loop: node.data.videoLoop ?? true,
        playbackRate: node.data.videoPlaybackRate ?? 1,
      };
      try {
        await this.addVideoSource(node.id, config);
        const { width, height } = this.videoSources.get(node.id)?.getResolution() ?? { width: 0, height: 0 };
        if (width > 0 && height > 0) this.callbacks.onOutputSize?.(node.id, width, height);
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        this.callbacks.onNodeError?.(node.id, msg);
      }
    }
    for (const nodeId of Array.from(this.videoSources.keys())) {
      if (!wanted.has(nodeId)) this.removeVideoSource(nodeId);
    }
    if (wanted.size > 0) this.needsRecompile = true;
  }

  getState(): HostState {
    return this.state;
  }

  getClock(): Clock {
    return this.clock;
  }

  getMouse(): MouseState {
    return this.mouse;
  }

  private tick(now: DOMHighResTimeStamp): void {
    if (this.needsRecompile) {
      this.needsRecompile = false;
      this.compositor.prepare(
        this.nodes,
        this.edges,
        this.callbacks.onNodeError,
        this.callbacks.onOutputSize,
      );
    }

    const ts = this.clock.tick(now);

    for (const [nodeId, source] of this.videoSources) {
      const texture = source.getTexture();
      if (texture) this.videoTextures.set(nodeId, texture);
    }

    const inputs: FrameInputs = {
      time: ts.time,
      delta: ts.delta,
      frame: ts.frame,
      date: ts.date,
      mouse: this.mouse.iMouse,
      resolution: this.resolution,
      videoTextures: this.videoTextures,
    };

    this.compositor.render(inputs);

    const rendererId = this.activeRendererId
      ?? this.nodes.find((node) => node.data.type === 'renderer' && node.data.expanded !== false)?.id
      ?? null;
    if (rendererId) {
      this.compositor.renderRendererToScreen(rendererId);
    }

    this.callbacks.onFrame?.(ts);
  }
}
