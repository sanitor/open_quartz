import { useRef, useEffect } from 'react';
import { ReactFlowProvider } from '@xyflow/react';
import { Header } from './components/Header';
import { NodeGraph } from './components/NodeGraph';
import { SidePanel } from './components/SidePanel';
import { PipelineService } from './services/PipelineService';

export default function App() {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const service = new PipelineService();
    service.attach(canvas);
    return () => service.detach();
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
