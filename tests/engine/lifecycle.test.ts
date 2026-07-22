import { describe, it, expect, vi, beforeEach, type Mock } from 'vitest';
import type { Node, Edge } from '@xyflow/react';
import type { ShaderNodeData } from '../../src/types';

// ---------------------------------------------------------------------------
// vi.hoisted – mock instances available inside vi.mock factories
// ---------------------------------------------------------------------------
const {
  mockCompositorInstance,
  mockClockInstance,
  mockMouseInstance,
  mockVideoSourceInstance,
} = vi.hoisted(() => {
  const canvas = document.createElement('canvas');
  const compositor = {
    init: vi.fn(() => Promise.resolve()),
    prepare: vi.fn(() => []),
    render: vi.fn(),
    readOutputs: vi.fn(),
    readNodeOutput: vi.fn(() => Promise.resolve()),
    renderRendererToScreen: vi.fn(),
    getCanvas: vi.fn(() => canvas),
    captureScreenshot: vi.fn(() => Promise.resolve(null)),
    dispose: vi.fn(),
  };
  const clock = {
    start: vi.fn(),
    tick: vi.fn(() => ({
      time: 0,
      delta: 0.016,
      frame: 0,
      date: new Float32Array(4),
      fps: 60,
    })),
    pause: vi.fn(),
    resume: vi.fn(),
    reset: vi.fn(),
  };
  const mouse = {
    attach: vi.fn(),
    detach: vi.fn(),
    iMouse: new Float32Array(4),
  };
  const videoSource = {
    init: vi.fn(() => Promise.resolve()),
    getTexture: vi.fn(() => null),
    getResolution: vi.fn(() => ({ width: 640, height: 480 })),
    play: vi.fn(),
    pause: vi.fn(),
    dispose: vi.fn(),
  };
  return {
    mockCompositorInstance: compositor,
    mockClockInstance: clock,
    mockMouseInstance: mouse,
    mockVideoSourceInstance: videoSource,
  };
});

// ---------------------------------------------------------------------------
// Module mocks
// ---------------------------------------------------------------------------

vi.mock('three', () => ({
  Texture: class {
    dispose = vi.fn();
  },
}));

vi.mock('../../src/engine/compositor', () => {
  const Ctor = vi.fn(function (this: Record<string, unknown>) {
    Object.assign(this, mockCompositorInstance);
  });
  return { Compositor: Ctor };
});

vi.mock('../../src/engine/clock', () => {
  const Ctor = vi.fn(function (this: Record<string, unknown>) {
    Object.assign(this, mockClockInstance);
  });
  return { Clock: Ctor };
});

vi.mock('../../src/engine/mouseState', () => {
  const Ctor = vi.fn(function (this: Record<string, unknown>) {
    Object.assign(this, mockMouseInstance);
  });
  return { MouseState: Ctor };
});

vi.mock('../../src/engine/videoSource', () => {
  const Ctor = vi.fn(function (this: Record<string, unknown>) {
    Object.assign(this, mockVideoSourceInstance);
  });
  return { VideoSource: Ctor };
});

// Stub rAF / cAF globally
let rafCallbacks: Array<{ id: number; cb: FrameRequestCallback }> = [];
let nextRafId = 1;

const mockRAF = vi.fn((cb: FrameRequestCallback): number => {
  const id = nextRafId++;
  rafCallbacks.push({ id, cb });
  return id;
});

const mockCAF = vi.fn((id: number) => {
  rafCallbacks = rafCallbacks.filter((entry) => entry.id !== id);
});

vi.stubGlobal('requestAnimationFrame', mockRAF);
vi.stubGlobal('cancelAnimationFrame', mockCAF);

// Stub window.addEventListener/removeEventListener to prevent errors on 'renderer-remount'
const originalAddEventListener = window.addEventListener.bind(window);
const originalRemoveEventListener = window.removeEventListener.bind(window);
vi.spyOn(window, 'addEventListener').mockImplementation(originalAddEventListener);
vi.spyOn(window, 'removeEventListener').mockImplementation(originalRemoveEventListener);

// Stub document.querySelectorAll for renderToScreen() — returns empty list
vi.spyOn(document, 'querySelectorAll').mockReturnValue(
  document.querySelectorAll('.nonexistent-selector'),
);

// ---------------------------------------------------------------------------
// Imports (after mocks are set up)
// ---------------------------------------------------------------------------
import { RealtimeHost, isStaticPipeline } from '../../src/engine/realtimeHost';
import type { HostCallbacks, HostState } from '../../src/engine/realtimeHost';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function makeNode(id: string, data: Partial<ShaderNodeData>): Node<ShaderNodeData> {
  return {
    id,
    type: 'custom',
    position: { x: 0, y: 0 },
    data: {
      type: 'shader',
      label: id,
      shaderCode: 'void main() {}',
      inputs: [],
      outputs: [],
      uniforms: {},
      ...data,
    },
  };
}

/** A simple dynamic shader node that references iTime. */
function makeDynamicNode(id: string): Node<ShaderNodeData> {
  return makeNode(id, { shaderCode: 'void main() { float t = iTime; }' });
}

/** A static image-input node. */
function makeImageInputNode(id: string): Node<ShaderNodeData> {
  return makeNode(id, { type: 'input', inputMode: 'image', shaderCode: '' });
}

/** A video-input node (always dynamic). */
function makeVideoInputNode(id: string): Node<ShaderNodeData> {
  return makeNode(id, { type: 'input', inputMode: 'video', shaderCode: '' });
}

/** A renderer node. */
function makeRendererNode(id: string): Node<ShaderNodeData> {
  return makeNode(id, { type: 'renderer', shaderCode: '' });
}

/** Flush all queued rAF callbacks once. */
function flushRAF(now = 16.67): void {
  const pending = [...rafCallbacks];
  rafCallbacks = [];
  for (const entry of pending) {
    entry.cb(now);
  }
}

/** Flush rAF callbacks a specified number of times. */
function flushRAFTimes(n: number, startTime = 16.67, dt = 16.67): void {
  for (let i = 0; i < n; i++) {
    flushRAF(startTime + i * dt);
  }
}

// ---------------------------------------------------------------------------
// Test setup
// ---------------------------------------------------------------------------

let canvas: HTMLCanvasElement;
let callbacks: HostCallbacks;
let host: RealtimeHost;

beforeEach(() => {
  vi.clearAllMocks();
  rafCallbacks = [];
  nextRafId = 1;

  canvas = document.createElement('canvas');
  callbacks = {
    onFrame: vi.fn(),
    onOutput: vi.fn(),
    onNodeError: vi.fn(),
    onOutputSize: vi.fn(),
    onOutputData: vi.fn(),
    onStateChange: vi.fn(),
  };
  host = new RealtimeHost(canvas, callbacks);
});

// ===========================================================================
// RealtimeHost lifecycle
// ===========================================================================

describe('RealtimeHost lifecycle', () => {
  it('play() starts the rAF loop', async () => {
    const nodes = [makeDynamicNode('s1')];
    await host.play(nodes, []);

    expect(mockRAF).toHaveBeenCalled();
    expect(mockClockInstance.start).toHaveBeenCalled();
    expect(mockMouseInstance.attach).toHaveBeenCalledWith(document.body);
  });

  it('pause() freezes the clock and fires onStateChange', async () => {
    const nodes = [makeDynamicNode('s1')];
    await host.play(nodes, []);
    vi.clearAllMocks();

    host.pause();

    expect(mockClockInstance.pause).toHaveBeenCalled();
    expect(callbacks.onStateChange).toHaveBeenCalledWith('paused');
  });

  it('resume() resumes the clock and restarts the rAF loop', async () => {
    const nodes = [makeDynamicNode('s1')];
    await host.play(nodes, []);
    host.pause();
    vi.clearAllMocks();

    host.resume();

    expect(mockClockInstance.resume).toHaveBeenCalled();
    expect(mockRAF).toHaveBeenCalled();
    expect(callbacks.onStateChange).toHaveBeenCalledWith('playing');
  });

  it('stop() cleans up compositor, clock, mouse, and cancels rAF', async () => {
    const nodes = [makeDynamicNode('s1')];
    await host.play(nodes, []);
    vi.clearAllMocks();

    host.stop();

    expect(mockCompositorInstance.dispose).toHaveBeenCalled();
    expect(mockClockInstance.reset).toHaveBeenCalled();
    expect(mockMouseInstance.detach).toHaveBeenCalled();
    expect(callbacks.onStateChange).toHaveBeenCalledWith('stopped');
  });

  it('play→pause→resume→stop full lifecycle sequence', async () => {
    const nodes = [makeDynamicNode('s1')];
    const stateChanges: HostState[] = [];
    callbacks.onStateChange = (s: HostState) => stateChanges.push(s);

    await host.play(nodes, []);
    expect(host.getState()).toBe('playing');

    host.pause();
    expect(host.getState()).toBe('paused');

    host.resume();
    expect(host.getState()).toBe('playing');

    host.stop();
    expect(host.getState()).toBe('stopped');

    expect(stateChanges).toEqual(['playing', 'paused', 'playing', 'stopped']);
  });

  it('updateGraph() sets needsRecompile — compositor.prepare() called on next tick', async () => {
    const nodes = [makeDynamicNode('s1')];
    await host.play(nodes, []);
    // Initial prepare from play()
    expect(mockCompositorInstance.prepare).toHaveBeenCalledTimes(1);

    // Update graph with a new node to trigger data change
    const newNodes = [makeDynamicNode('s1'), makeDynamicNode('s2')];
    host.updateGraph(newNodes, []);

    // Flush the rAF so tick() runs and triggers the recompile
    flushRAF();

    expect(mockCompositorInstance.prepare).toHaveBeenCalledTimes(2);
    // Second prepare should receive the updated nodes
    expect(mockCompositorInstance.prepare.mock.calls[1][0]).toBe(newNodes);
  });

  it('setPreviewNode() stores the preview node ID — readNodeOutput called during tick', async () => {
    const nodes = [makeDynamicNode('s1')];
    await host.play(nodes, []);

    host.setPreviewNode('s1');
    flushRAF();

    expect(mockCompositorInstance.readNodeOutput).toHaveBeenCalledWith(
      's1',
      callbacks.onOutput,
    );
  });

  it('play() with static pipeline runs one frame then does not chain rAF', async () => {
    const nodes = [makeImageInputNode('img1'), makeRendererNode('r1')];
    await host.play(nodes, []);

    // One rAF was queued for the single frame
    expect(rafCallbacks).toHaveLength(1);

    // After flushing the single rAF, no further callbacks are queued
    // because the static pipeline's callback nulls out rafId and does not re-queue
    flushRAF();
    expect(rafCallbacks).toHaveLength(0);
  });

  it('play() calls onFrame callback with time state', async () => {
    const nodes = [makeDynamicNode('s1')];
    await host.play(nodes, []);

    flushRAF();

    expect(callbacks.onFrame).toHaveBeenCalledWith(
      expect.objectContaining({ time: 0, delta: 0.016, frame: 0, fps: 60 }),
    );
  });

  it('captureScreenshot() returns null (WebGPU readback not yet implemented)', async () => {
    const nodes = [makeDynamicNode('s1')];
    await host.play(nodes, []);

    const result = host.captureScreenshot('r1');
    expect(result).toBeNull();
  });

  it('stop() cancels a pending rAF', async () => {
    const nodes = [makeDynamicNode('s1')];
    await host.play(nodes, []);

    const queuedBefore = rafCallbacks.length;
    expect(queuedBefore).toBeGreaterThan(0);

    host.stop();
    expect(mockCAF).toHaveBeenCalled();
  });
});

// ===========================================================================
// Static vs dynamic pipeline detection
// ===========================================================================

describe('Static vs dynamic pipeline detection', () => {
  it('shader with iTime is classified as dynamic', async () => {
    const nodes = [makeDynamicNode('s1')];
    expect(isStaticPipeline(nodes)).toBe(false);
  });

  it('video input node forces dynamic pipeline', async () => {
    const nodes = [makeVideoInputNode('v1')];
    expect(isStaticPipeline(nodes)).toBe(false);
  });

  it('image-only pipeline is static', async () => {
    const nodes = [makeImageInputNode('img1'), makeRendererNode('r1')];
    expect(isStaticPipeline(nodes)).toBe(true);
  });

  it('system source "time" forces dynamic', async () => {
    const nodes = [makeNode('sys', {
      type: 'input',
      inputMode: 'system',
      systemSource: 'time',
      shaderCode: '',
    })];
    expect(isStaticPipeline(nodes)).toBe(false);
  });

  it('system source "resolution" is static', async () => {
    const nodes = [makeNode('sys', {
      type: 'input',
      inputMode: 'system',
      systemSource: 'resolution',
      shaderCode: '',
    })];
    expect(isStaticPipeline(nodes)).toBe(true);
  });

  it('shader with previousFrame is dynamic', async () => {
    const nodes = [makeNode('s1', {
      shaderCode: 'void main() { vec4 c = previousFrame; }',
    })];
    expect(isStaticPipeline(nodes)).toBe(false);
  });

  it('static pipeline via RealtimeHost: rAF stops after first render', async () => {
    const nodes = [makeImageInputNode('img1')];
    await host.play(nodes, []);

    // Exactly one rAF was queued
    expect(rafCallbacks).toHaveLength(1);
    flushRAF();

    // After render, no more rAF callbacks queued
    expect(rafCallbacks).toHaveLength(0);

    // Compositor still rendered exactly once
    expect(mockCompositorInstance.render).toHaveBeenCalledTimes(1);
  });

  it('dynamic pipeline via RealtimeHost: rAF chains continuously', async () => {
    const nodes = [makeDynamicNode('s1')];
    await host.play(nodes, []);

    // Flush 3 frames — each tick re-queues rAF
    flushRAF(16);
    expect(rafCallbacks).toHaveLength(1);
    flushRAF(32);
    expect(rafCallbacks).toHaveLength(1);
    flushRAF(48);

    expect(mockCompositorInstance.render).toHaveBeenCalledTimes(3);
  });
});

// ===========================================================================
// Hot-update while playing
// ===========================================================================

describe('Hot-update while playing', () => {
  it('updateGraph during play triggers recompile on next tick', async () => {
    const nodes = [makeDynamicNode('s1')];
    await host.play(nodes, []);
    expect(mockCompositorInstance.prepare).toHaveBeenCalledTimes(1);

    // Change the shader code in a new node array to force data change
    const updated = [makeNode('s1', { shaderCode: 'void main() { float x = iTime + 1.0; }' })];
    host.updateGraph(updated, []);

    // The recompile happens on the next tick, not immediately
    expect(mockCompositorInstance.prepare).toHaveBeenCalledTimes(1);

    flushRAF();
    expect(mockCompositorInstance.prepare).toHaveBeenCalledTimes(2);
  });

  it('recompile passes updated nodes and edges to compositor.prepare', async () => {
    const nodes = [makeDynamicNode('s1')];
    const edges: Edge[] = [];
    await host.play(nodes, edges);

    const newEdges: Edge[] = [{ id: 'e1', source: 's1', target: 's2' }];
    const newNodes = [makeDynamicNode('s1'), makeDynamicNode('s2')];
    host.updateGraph(newNodes, newEdges);

    flushRAF();

    const lastCall = mockCompositorInstance.prepare.mock.calls[
      mockCompositorInstance.prepare.mock.calls.length - 1
    ];
    expect(lastCall[0]).toBe(newNodes);
    expect(lastCall[1]).toBe(newEdges);
  });

  it('updateGraph with new nodes triggers prepare with updated node list', async () => {
    const nodes = [makeDynamicNode('s1')];
    await host.play(nodes, []);

    const newNodes = [makeDynamicNode('s1'), makeDynamicNode('s2'), makeDynamicNode('s3')];
    host.updateGraph(newNodes, []);

    flushRAF();

    // The recompile should use the latest node list
    const recompileNodes = mockCompositorInstance.prepare.mock.calls[1][0] as Node<ShaderNodeData>[];
    expect(recompileNodes).toHaveLength(3);
    expect(recompileNodes.map((n: Node<ShaderNodeData>) => n.id)).toEqual(['s1', 's2', 's3']);
  });

  it('position-only change still triggers recompile (no diff optimization yet)', async () => {
    const nodes = [makeDynamicNode('s1')];
    await host.play(nodes, []);
    expect(mockCompositorInstance.prepare).toHaveBeenCalledTimes(1);

    // Move node position — updateGraph currently always recompiles
    const movedNodes = [{ ...nodes[0], position: { x: 100, y: 200 } }];
    host.updateGraph(movedNodes, []);
    flushRAF();

    // Current behavior: recompile on every updateGraph call
    expect(mockCompositorInstance.prepare).toHaveBeenCalledTimes(2);
  });
});

// ===========================================================================
// App store→host bridge
// ===========================================================================

describe('App store→host bridge', () => {
  /**
   * These tests verify the state-machine contract between the Zustand store
   * and RealtimeHost, as currently implemented in App.tsx lines 14–84.
   * They replicate the subscription logic so that the upcoming refactor
   * (extracting it into a service layer) has regression coverage.
   */

  interface StoreSlice {
    loopState: 'stopped' | 'playing' | 'paused';
    nodes: Node<ShaderNodeData>[];
    edges: Edge[];
    selectedNodeId: string | null;
  }

  // The mock host tracks calls
  interface MockHost {
    play: Mock;
    pause: Mock;
    resume: Mock;
    stop: Mock;
    updateGraph: Mock;
    setPreviewNode: Mock;
    captureScreenshot: Mock;
  }

  function createMockHost(): MockHost {
    return {
      play: vi.fn(),
      pause: vi.fn(),
      resume: vi.fn(),
      stop: vi.fn(),
      updateGraph: vi.fn(),
      setPreviewNode: vi.fn(),
      captureScreenshot: vi.fn(() => null),
    };
  }

  /**
   * Replicates the subscription handler from App.tsx.
   * This is the contract under test — the bridge between store state changes
   * and host method calls.
   */
  function createBridgeHandler(getHost: () => MockHost | null, setHost: (h: MockHost) => void) {
    return (state: StoreSlice, prev: StoreSlice) => {
      // Play (stopped → playing)
      if (state.loopState === 'playing' && prev.loopState === 'stopped') {
        let mockHost = getHost();
        if (!mockHost) {
          mockHost = createMockHost();
          setHost(mockHost);
        }
        mockHost.setPreviewNode(state.selectedNodeId);
        mockHost.play(state.nodes, state.edges);
      }

      // Pause (playing → paused)
      if (state.loopState === 'paused' && prev.loopState === 'playing') {
        getHost()?.pause();
      }

      // Resume (paused → playing)
      if (state.loopState === 'playing' && prev.loopState === 'paused') {
        getHost()?.resume();
      }

      // Stop (any → stopped)
      if (state.loopState === 'stopped' && prev.loopState !== 'stopped') {
        getHost()?.stop();
      }

      // Hot-update graph while playing
      if (
        state.loopState === 'playing' &&
        (state.nodes !== prev.nodes || state.edges !== prev.edges)
      ) {
        getHost()?.updateGraph(state.nodes, state.edges);
      }

      // Sync preview node
      if (state.selectedNodeId !== prev.selectedNodeId) {
        getHost()?.setPreviewNode(state.selectedNodeId);
      }
    };
  }

  let mockHost: MockHost | null;
  let handler: (state: StoreSlice, prev: StoreSlice) => void;

  const defaultNodes = [makeDynamicNode('s1')];
  const defaultEdges: Edge[] = [];

  const stoppedState: StoreSlice = {
    loopState: 'stopped',
    nodes: defaultNodes,
    edges: defaultEdges,
    selectedNodeId: null,
  };

  beforeEach(() => {
    mockHost = null;
    handler = createBridgeHandler(
      () => mockHost,
      (h) => { mockHost = h; },
    );
  });

  it('stopped→playing creates host and calls play()', async () => {
    const playingState: StoreSlice = { ...stoppedState, loopState: 'playing' };

    handler(playingState, stoppedState);

    expect(mockHost).not.toBeNull();
    expect(mockHost!.play).toHaveBeenCalledWith(defaultNodes, defaultEdges);
  });

  it('playing→paused calls host.pause()', async () => {
    // First transition to playing
    const playingState: StoreSlice = { ...stoppedState, loopState: 'playing' };
    handler(playingState, stoppedState);

    const pausedState: StoreSlice = { ...playingState, loopState: 'paused' };
    handler(pausedState, playingState);

    expect(mockHost!.pause).toHaveBeenCalledTimes(1);
  });

  it('paused→playing calls host.resume()', async () => {
    // Setup: stopped → playing → paused
    const playingState: StoreSlice = { ...stoppedState, loopState: 'playing' };
    handler(playingState, stoppedState);
    const pausedState: StoreSlice = { ...playingState, loopState: 'paused' };
    handler(pausedState, playingState);

    // paused → playing
    handler(playingState, pausedState);

    expect(mockHost!.resume).toHaveBeenCalledTimes(1);
    // resume, not play — play is only for stopped→playing
    expect(mockHost!.play).toHaveBeenCalledTimes(1); // only from initial start
  });

  it('playing→stopped calls host.stop()', async () => {
    const playingState: StoreSlice = { ...stoppedState, loopState: 'playing' };
    handler(playingState, stoppedState);

    handler(stoppedState, playingState);

    expect(mockHost!.stop).toHaveBeenCalledTimes(1);
  });

  it('nodes/edges change while playing calls host.updateGraph()', async () => {
    const playingState: StoreSlice = { ...stoppedState, loopState: 'playing' };
    handler(playingState, stoppedState);

    const newNodes = [makeDynamicNode('s1'), makeDynamicNode('s2')];
    const updatedState: StoreSlice = { ...playingState, nodes: newNodes };
    handler(updatedState, playingState);

    expect(mockHost!.updateGraph).toHaveBeenCalledWith(newNodes, defaultEdges);
  });

  it('nodes/edges change while stopped does NOT call updateGraph', async () => {
    // Go playing first to create the host, then stop
    const playingState: StoreSlice = { ...stoppedState, loopState: 'playing' };
    handler(playingState, stoppedState);
    handler(stoppedState, playingState);

    const newNodes = [makeDynamicNode('s1'), makeDynamicNode('s2')];
    const updatedStopped: StoreSlice = { ...stoppedState, nodes: newNodes };
    handler(updatedStopped, stoppedState);

    expect(mockHost!.updateGraph).not.toHaveBeenCalled();
  });

  it('selectedNodeId change syncs preview node', async () => {
    const playingState: StoreSlice = { ...stoppedState, loopState: 'playing' };
    handler(playingState, stoppedState);

    const withSelection: StoreSlice = { ...playingState, selectedNodeId: 's1' };
    handler(withSelection, playingState);

    // Once from initial play (null), once from selection change
    expect(mockHost!.setPreviewNode).toHaveBeenCalledWith('s1');
  });

  it('full bridge lifecycle: stopped→playing→paused→playing→stopped', async () => {
    const playing: StoreSlice = { ...stoppedState, loopState: 'playing' };
    const paused: StoreSlice = { ...playing, loopState: 'paused' };

    handler(playing, stoppedState);
    expect(mockHost!.play).toHaveBeenCalledTimes(1);

    handler(paused, playing);
    expect(mockHost!.pause).toHaveBeenCalledTimes(1);

    handler(playing, paused);
    expect(mockHost!.resume).toHaveBeenCalledTimes(1);

    handler(stoppedState, playing);
    expect(mockHost!.stop).toHaveBeenCalledTimes(1);
  });
});
