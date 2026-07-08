import { useRef, useEffect } from 'react';
import { ReactFlowProvider } from '@xyflow/react';
import { Header } from './components/Header';
import { NodeGraph } from './components/NodeGraph';
import { SidePanel } from './components/SidePanel';
import { useGraphStore } from './store/useGraphStore';
import { RealtimeHost } from './engine/realtimeHost';

export default function App() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const canvasHolderRef = useRef<HTMLDivElement>(null);
  const hostRef = useRef<RealtimeHost | null>(null);

  const mountRendererCanvas = () => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const state = useGraphStore.getState();
    const rendererId = state.activeRendererId
      ?? state.nodes.find((node) => node.data.type === 'renderer' && node.data.expanded !== false)?.id;
    if (!rendererId) return;
    const mount = document.getElementById(`renderer-canvas-mount-${rendererId}`);
    if (!mount || canvas.parentElement === mount) return;
    canvas.className = '';
    canvas.style.width = '100%';
    canvas.style.height = '100%';
    canvas.style.display = 'block';
    mount.replaceChildren(canvas);
    hostRef.current?.setActiveRenderer(rendererId);
  };



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
        if (!hostRef.current) {
          hostRef.current = new RealtimeHost(canvas, {
            onFrame: (ts) => {
              const s = useGraphStore.getState();
              s.setFps(ts.fps);
              s.setCurrentTime(ts.time);
              s.setCurrentFrame(ts.frame);
            },
            onOutput: (nodeId, dataUrl) => useGraphStore.getState().setOutputPreview(nodeId, dataUrl),
            onNodeError: (nodeId, error) => {
              useGraphStore.getState().setNodeError(nodeId, error);
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
        }
        // Use setTimeout to ensure DOM has updated after state change
        setTimeout(() => {
          mountRendererCanvas();
          const s = useGraphStore.getState();
          hostRef.current?.play(s.nodes, s.edges);
        }, 0);
      }

      // Pause
      if (state.loopState === 'paused' && prev.loopState === 'playing') {
        hostRef.current?.pause();
      }
      // Resume
      if (state.loopState === 'playing' && prev.loopState === 'paused') {
        hostRef.current?.resume();
        requestAnimationFrame(mountRendererCanvas);
      }

      // Stop
      if (state.loopState === 'stopped' && prev.loopState !== 'stopped') {
        hostRef.current?.stop();
        const c = canvasRef.current;
        const holder = canvasHolderRef.current;
        if (c && holder) {
          c.style.width = '';
          c.style.height = '';
          c.style.display = '';
          holder.appendChild(c);
        }
      }

      // Hot-update graph while playing
      if (state.loopState === 'playing' &&
          (state.nodes !== prev.nodes || state.edges !== prev.edges)) {
        hostRef.current?.updateGraph(state.nodes, state.edges);
        requestAnimationFrame(mountRendererCanvas);
      }

      if (state.activeRendererId !== prev.activeRendererId) {
        hostRef.current?.setActiveRenderer(state.activeRendererId);
        requestAnimationFrame(mountRendererCanvas);
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
        <div ref={canvasHolderRef} className="hidden"><canvas ref={canvasRef} /></div>
      </div>
    </ReactFlowProvider>
  );
}
