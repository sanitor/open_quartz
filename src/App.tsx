import { useRef, useEffect } from 'react';
import { ReactFlowProvider } from '@xyflow/react';
import { Header } from './components/Header';
import { NodeGraph } from './components/NodeGraph';
import { SidePanel } from './components/SidePanel';
import { useGraphStore } from './store/useGraphStore';
import { ExecutionEngine } from './engine/executionEngine';

export default function App() {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const unsub = useGraphStore.subscribe((state, prev) => {
      if (state.isRunning && !prev.isRunning) {
        const canvas = canvasRef.current;
        if (!canvas) return;

        const engine = new ExecutionEngine(canvas);
        const nodes = state.nodes;
        const edges = state.edges;

        const setOutputPreview = useGraphStore.getState().setOutputPreview;
        const clearOutputPreviews = useGraphStore.getState().clearOutputPreviews;
        clearOutputPreviews();

        engine.run(nodes, edges, setOutputPreview)
          .catch((err) => console.error('Execution error:', err))
          .finally(() => {
            engine.stop();
            useGraphStore.getState().setRunning(false);
          });
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
