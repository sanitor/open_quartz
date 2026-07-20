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
  onOutputData?: (nodeId: string, data: unknown) => void;
  onStateChange?: (state: HostState) => void;
}

/** Time-varying builtins — if a shader references any of these it needs continuous frames. */
const DYNAMIC_BUILTINS = /\b(iTime|iTimeDelta|iFrame|iMouse)\b/;

/** Dynamic system sources that change every frame. Resolution is static. */
const DYNAMIC_SYSTEM_SOURCES: Record<string, true> = { time: true, timeDelta: true, frame: true, mouse: true };

/**
 * A pipeline is static when no node depends on time-varying inputs.
 * Static pipelines only need a single render pass.
 */
export function isStaticPipeline(nodes: Node<ShaderNodeData>[]): boolean {
  for (const node of nodes) {
    if (node.data.type === 'input' && node.data.inputMode === 'video') return false;
    if (node.data.type === 'input' && node.data.inputMode === 'system'
        && node.data.systemSource && DYNAMIC_SYSTEM_SOURCES[node.data.systemSource]) return false;
    if ((node.data.type === 'shader' || node.data.type === 'constant')
        && (DYNAMIC_BUILTINS.test(node.data.shaderCode) || /\bpreviousFrame\b/.test(node.data.shaderCode))) return false;
  }
  return true;
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
  private isStatic = false;
  private lastInputs: FrameInputs | null = null;

  /** Repaint all renderer mirrors on demand (e.g. fullscreen canvas mounted). */
  private onRemount = (): void => {
    if (this.state === 'playing') {
      requestAnimationFrame(() => this.renderToScreen());
    }
  };

  /**
   * Re-render with the last frame's frozen inputs (no clock advance).
   * Called when async ONNX completes in static mode — triggers the downstream
   * renderer to pick up new textures.  Cascaded ONNX (A→B→Renderer) naturally
   * converges: A completes → rerender fires B → B completes → rerender shows result.
   */
  private scheduleRerender = (): void => {
    if (this.state !== 'playing' || !this.isStatic || !this.lastInputs) return;
    requestAnimationFrame(() => {
      if (this.state !== 'playing' || !this.lastInputs) return;
      this.compositor.render(this.lastInputs);
      this.renderToScreen();
    });
  };

  constructor(canvas: HTMLCanvasElement, callbacks: HostCallbacks) {
    this.compositor = new Compositor(canvas);
    this.callbacks = callbacks;
  }

  play(nodes: Node<ShaderNodeData>[], edges: Edge[]): void {
    this.nodes = nodes;
    this.edges = edges;
    this.isStatic = isStaticPipeline(nodes);
    this.compositor.prepare(
      nodes,
      edges,
      this.callbacks.onNodeError,
      this.callbacks.onOutputSize,
      this.callbacks.onOutputData,
      this.callbacks.onOutput,
      this.scheduleRerender,
    );
    void this.reconcileVideoSources(nodes);
    window.addEventListener('renderer-remount', this.onRemount);
    this.clock.start();
    this.mouse.attach(document.body);
    this.state = 'playing';
    this.callbacks.onStateChange?.('playing');

    if (this.isStatic) {
      // Static pipeline: render one frame, then stop the loop.
      this.rafId = requestAnimationFrame((now) => {
        this.tick(now);
        this.rafId = null;
      });
    } else {
      const frame = (now: DOMHighResTimeStamp): void => {
        if (this.state === 'stopped') return;
        this.tick(now);
        this.rafId = requestAnimationFrame(frame);
      };
      this.rafId = requestAnimationFrame(frame);
    }
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

    if (this.isStatic) {
      this.rafId = requestAnimationFrame((now) => {
        this.tick(now);
        this.rafId = null;
      });
    } else {
      const frame = (now: DOMHighResTimeStamp): void => {
        if (this.state === 'stopped') return;
        this.tick(now);
        this.rafId = requestAnimationFrame(frame);
      };
      this.rafId = requestAnimationFrame(frame);
    }
  }

  stop(): void {
    this.state = 'stopped';
    if (this.rafId !== null) {
      cancelAnimationFrame(this.rafId);
      this.rafId = null;
    }
    this.clock.reset();
    this.mouse.detach();
    window.removeEventListener('renderer-remount', this.onRemount);
    for (const source of this.videoSources.values()) source.dispose();
    this.videoSources.clear();
    this.videoTextures.clear();
    this.compositor.dispose();
    this.callbacks.onStateChange?.('stopped');
  }

  updateGraph(nodes: Node<ShaderNodeData>[], edges: Edge[]): void {
    const prevNodes = this.nodes;
    const prevEdges = this.edges;
    this.nodes = nodes;
    this.edges = edges;

    // Only recompile when topology or node data actually changed.
    // Position-only moves (dragging) must not rebuild the plan — that
    // would reset feedback ping-pong buffers and clear the simulation.
    const graphChanged = edges !== prevEdges
      || nodes.length !== prevNodes.length
      || nodes.some((n, i) => n.id !== prevNodes[i].id || n.data !== prevNodes[i].data);

    if (!graphChanged) return;

    this.needsRecompile = true;
    void this.reconcileVideoSources(nodes);

    const wasStatic = this.isStatic;
    this.isStatic = isStaticPipeline(nodes);

    if (this.isStatic && this.state === 'playing') {
      // Re-render one frame with updated graph.
      if (this.rafId !== null) {
        cancelAnimationFrame(this.rafId);
      }
      this.rafId = requestAnimationFrame((now) => {
        this.tick(now);
        this.rafId = null;
      });
    } else if (!this.isStatic && wasStatic && this.state === 'playing') {
      // Pipeline became dynamic — start the continuous loop.
      const frame = (now: DOMHighResTimeStamp): void => {
        if (this.state === 'stopped') return;
        this.tick(now);
        this.rafId = requestAnimationFrame(frame);
      };
      this.rafId = requestAnimationFrame(frame);
    }
  }


  captureScreenshot(rendererId: string): string | null {
    return this.compositor.captureScreenshot(rendererId);
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
        this.callbacks.onOutputData,
        this.callbacks.onOutput,
        this.scheduleRerender,
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

    this.lastInputs = inputs;
    this.compositor.render(inputs);
    this.renderToScreen();
    this.callbacks.onFrame?.(ts);
  }

  private renderToScreen(): void {
    const glCanvas = this.compositor.getCanvas();
    const rendererNodes = this.nodes.filter((n) => n.data.type === 'renderer');
    for (const rNode of rendererNodes) {
      this.compositor.renderRendererToScreen(rNode.id);
      if (!glCanvas) continue;
      const mirrors = document.querySelectorAll<HTMLCanvasElement>(
        `canvas[id^="renderer-mirror-"][id$="-${rNode.id}"], canvas#renderer-mirror-${rNode.id}`
      );
      for (const mirror of mirrors) {
        const ctx = mirror.getContext('2d');
        if (!ctx) continue;
        ctx.clearRect(0, 0, mirror.width, mirror.height);
        ctx.drawImage(glCanvas, 0, 0, mirror.width, mirror.height);
      }
    }
  }
}
