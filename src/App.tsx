import { useRef, useEffect } from 'react';
import { ReactFlowProvider } from '@xyflow/react';
import { Header } from './components/Header';
import { NodeGraph } from './components/NodeGraph';
import { SidePanel } from './components/SidePanel';
import { useGraphStore } from './store/useGraphStore';
import { RealtimeHost } from './engine/realtimeHost';

export default function App() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const hostRef = useRef<RealtimeHost | null>(null);



  // Real-time loop
  useEffect(() => {
    const unsub = useGraphStore.subscribe((state, prev) => {
      const canvas = canvasRef.current;
      if (!canvas) return;

      // Play
      if (state.loopState === 'playing' && prev.loopState === 'stopped') {
        const store = useGraphStore.getState();
        store.clearOutputPreviews();
        store.clearNodeErrors();
        const host = new RealtimeHost(canvas, {
          onFrame: (ts) => {
            const s = useGraphStore.getState();
            s.setFps(ts.fps);
            s.setCurrentTime(ts.time);
            s.setCurrentFrame(ts.frame);
          },
          onOutput: (nodeId, dataUrl) => useGraphStore.getState().setOutputPreview(nodeId, dataUrl),
          onNodeError: (nodeId, error) => {
            const s = useGraphStore.getState();
            s.setNodeError(nodeId, error);
          },
          onOutputSize: (nodeId, w, h) => {
            const s = useGraphStore.getState();
            const node = s.nodes.find((n) => n.id === nodeId);
            if (node?.data.type === 'input' && node.data.inputMode === 'video') {
              s.updateNodeData(nodeId, { imageWidth: w, imageHeight: h, resolvedWidth: w, resolvedHeight: h });
            } else {
              s.updateNodeData(nodeId, { resolvedWidth: w, resolvedHeight: h });
            }
          },
        });
        hostRef.current = host;
        host.play(state.nodes, state.edges);
      }

      // Pause
      if (state.loopState === 'paused' && prev.loopState === 'playing') {
        hostRef.current?.pause();
      }
      // Resume
      if (state.loopState === 'playing' && prev.loopState === 'paused') {
        hostRef.current?.resume();
      }

      // Stop
      if (state.loopState === 'stopped' && prev.loopState !== 'stopped') {
        hostRef.current?.stop();
        hostRef.current = null;
      }

      // Hot-update graph while playing
      if (state.loopState === 'playing' &&
          (state.nodes !== prev.nodes || state.edges !== prev.edges)) {
        hostRef.current?.updateGraph(state.nodes, state.edges);
      }
    });
    return () => unsub();
  }, []);

  return (
    <ReactFlowProvider>
      <div className="flex flex-col w-full h-full">
        <Header />
        <main className="flex flex-1 overflow-hidden">
          <div className="flex-1 relative">
            <NodeGraph />
          </div>
          <SidePanel />
        </main>
        <canvas ref={canvasRef} className="hidden" />
      </div>
    </ReactFlowProvider>
  );
}
