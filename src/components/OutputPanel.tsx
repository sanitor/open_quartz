import { useEffect, useRef } from 'react';
import { useGraphStore } from '../store/useGraphStore';
import { ExecutionEngine } from '../engine/executionEngine';

export function OutputPanel() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const engineRef = useRef<ExecutionEngine | null>(null);
  const isRunning = useGraphStore((s) => s.isRunning);
  const nodes = useGraphStore((s) => s.nodes);
  const edges = useGraphStore((s) => s.edges);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    if (isRunning) {
      const engine = new ExecutionEngine(canvas);
      engineRef.current = engine;
      engine.run(nodes, edges).catch((err) => {
        console.error('Execution error:', err);
        useGraphStore.getState().setRunning(false);
      });
    } else {
      engineRef.current?.stop();
      engineRef.current = null;
    }

    return () => {
      engineRef.current?.stop();
      engineRef.current = null;
    };
  }, [isRunning]);

  return (
    <div className="h-48 border-t border-[#d2d2d7] bg-white flex-shrink-0 relative">
      <div className="flex items-center justify-between px-4 py-1 text-[11px] text-[#86868b] border-b border-[#e8e8ed]">
        <span className="font-medium">Output Preview</span>
        {isRunning && (
          <span className="text-[#ff3b30] font-medium">● Running</span>
        )}
      </div>
      <canvas
        ref={canvasRef}
        width={640}
        height={180}
        className="w-full h-[calc(100%-24px)] block"
      />
      {!isRunning && (
        <div className="absolute inset-0 flex items-center justify-center pointer-events-none top-6">
          <span className="text-[12px] text-[#aeaeb2]">Press Run to preview output</span>
        </div>
      )}
    </div>
  );
}
