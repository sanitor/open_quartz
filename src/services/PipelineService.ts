/**
 * PipelineService — bridges the Zustand store with the RealtimeHost.
 *
 * Subscribes to store state changes and translates them into host
 * lifecycle calls (play/pause/resume/stop, graph hot-update, preview sync).
 * Host callbacks are forwarded back into the store.
 *
 * This is the ONLY module allowed to import both store and runtime.
 */

import { useGraphStore } from '../store/useGraphStore';
import { RealtimeHost } from '../engine/realtimeHost';

export class PipelineService {
  private host: RealtimeHost | null = null;
  private unsub: (() => void) | null = null;

  /**
   * Attach to a canvas and start listening to store changes.
   * Call `detach()` on unmount to clean up.
   */
  attach(canvas: HTMLCanvasElement): void {
    this.unsub = useGraphStore.subscribe((state, prev) => {
      // Play
      if (state.loopState === 'playing' && prev.loopState === 'stopped') {
        const store = useGraphStore.getState();
        store.clearOutputPreviews();
        store.clearNodeErrors();
        if (!this.host) {
          this.host = new RealtimeHost(canvas, {
            onFrame: (ts) => {
              const s = useGraphStore.getState();
              s.setFps(ts.fps);
              s.setCurrentTime(ts.time);
              s.setCurrentFrame(ts.frame);
            },
            onOutput: (nodeId, dataUrl) =>
              useGraphStore.getState().setOutputPreview(nodeId, dataUrl),
            onNodeError: (nodeId, error) => {
              useGraphStore.getState().setNodeError(nodeId, error);
            },
            onOutputSize: (nodeId, w, h) => {
              const s = useGraphStore.getState();
              const node = s.nodes.find((n) => n.id === nodeId);
              if (node?.data.type === 'input' && node.data.inputMode === 'video') {
                s.updateNodeData(nodeId, {
                  imageWidth: w,
                  imageHeight: h,
                  resolvedWidth: w,
                  resolvedHeight: h,
                });
              } else {
                s.updateNodeData(nodeId, { resolvedWidth: w, resolvedHeight: h });
              }
            },
            onOutputData: (nodeId, data) => {
              useGraphStore.getState().setOutputData(nodeId, data);
            },
            onBackendDetected: (nodeId, backend) => {
              const s = useGraphStore.getState();
              const node = s.nodes.find((n) => n.id === nodeId);
              if (node && node.data.onnxBackend !== backend) {
                s.updateNodeData(nodeId, { onnxBackend: backend });
              }
            },
          });
        }
        useGraphStore
          .getState()
          .setCaptureScreenshot(
            (id) => this.host?.captureScreenshot(id) ?? null,
          );
        setTimeout(() => {
          const s = useGraphStore.getState();
          this.host?.setPreviewNode(s.selectedNodeId);
          this.host?.play(s.nodes, s.edges);
        }, 0);
      }

      // Pause
      if (state.loopState === 'paused' && prev.loopState === 'playing') {
        this.host?.pause();
      }

      // Resume
      if (state.loopState === 'playing' && prev.loopState === 'paused') {
        this.host?.resume();
      }

      // Stop
      if (state.loopState === 'stopped' && prev.loopState !== 'stopped') {
        this.host?.stop();
      }

      // Hot-update graph while playing
      if (
        state.loopState === 'playing' &&
        (state.nodes !== prev.nodes || state.edges !== prev.edges)
      ) {
        this.host?.updateGraph(state.nodes, state.edges);
      }

      // Sync preview node with side panel selection
      if (state.selectedNodeId !== prev.selectedNodeId) {
        this.host?.setPreviewNode(state.selectedNodeId);
      }
    });
  }

  detach(): void {
    this.unsub?.();
    this.unsub = null;
  }
}
