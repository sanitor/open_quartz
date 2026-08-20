import { useEffect, useRef, useState } from 'react';
import { PipelineService } from '../services/PipelineService';
import { useGraphStore } from '../store/useGraphStore';
import { OpenQuartzClient } from '../sdk';
import type { ScreenSaverBootstrap } from '../screensaver';

export function ScreenSaverApp({ bootstrap }: { bootstrap: ScreenSaverBootstrap }) {
  return <ScreenSaverPlayer bootstrap={bootstrap} />;
}

function ScreenSaverPlayer({ bootstrap }: { bootstrap: ScreenSaverBootstrap }) {
  const runtimeCanvasRef = useRef<HTMLCanvasElement>(null);
  const [error, setError] = useState<string | null>(null);
  const [outputSize] = useState(() => {
    const scale = window.devicePixelRatio || 1;
    return {
      width: Math.max(2, Math.round(window.innerWidth * scale / 2) * 2),
      height: Math.max(2, Math.round(window.innerHeight * scale / 2) * 2),
    };
  });

  useEffect(() => {
    const canvas = runtimeCanvasRef.current;
    if (!canvas) return;
    const service = new PipelineService();
    const objectUrls: string[] = [];
    let cancelled = false;
    service.attach(canvas);

    const start = async () => {
      const { invoke, convertFileSrc } = await import('@tauri-apps/api/core');
      const project = await new OpenQuartzClient().openProject(bootstrap.projectJson);
      const snapshot = project.graph.snapshot();
      const nodes = snapshot.nodes;
      for (const input of bootstrap.exposedInputs) {
        const selected = bootstrap.settings[input.nodeId];
        if (!selected) continue;
        const node = nodes.find((candidate) => candidate.id === input.nodeId);
        if (!node) continue;
        if (input.kind === 'video') {
          Object.assign(node.data, {
            videoSourceType: 'file',
            videoFilePath: selected,
            videoFileName: fileName(selected),
          });
        } else {
          let source = convertFileSrc(selected);
          try {
            const bytes = Uint8Array.from(await invoke<number[]>('screen_saver_read_file', { path: selected }));
            source = URL.createObjectURL(new Blob([bytes], { type: imageMimeType(selected) }));
            objectUrls.push(source);
          } catch {
            // The scoped asset URL remains valid for standard Pictures/Documents locations.
          }
          Object.assign(node.data, {
            imageDataUrl: source,
            imageFileName: fileName(selected),
          });
        }
      }
      project.graph.replace(nodes, snapshot.edges);
      const graph = project.screenSaverGraph(
        bootstrap.rendererNodeId,
        outputSize.width,
        outputSize.height,
      );
      if (cancelled) return;
      useGraphStore.getState().loadGraph(graph.nodes, graph.edges);
      useGraphStore.getState().play();
    };
    void start().catch((cause) => setError(cause instanceof Error ? cause.message : String(cause)));

    return () => {
      cancelled = true;
      service.detach();
      for (const url of objectUrls) URL.revokeObjectURL(url);
    };
  }, [bootstrap, outputSize]);

  useEffect(() => {
    if (bootstrap.mode !== 'run') return;
    let origin: { x: number; y: number } | null = null;
    const exit = () => {
      void import('@tauri-apps/api/core').then(({ invoke }) => invoke('screen_saver_exit'));
    };
    const onMouseMove = (event: MouseEvent) => {
      if (!origin) {
        origin = { x: event.screenX, y: event.screenY };
        return;
      }
      if (Math.abs(event.screenX - origin.x) + Math.abs(event.screenY - origin.y) > 6) exit();
    };
    window.addEventListener('mousemove', onMouseMove);
    window.addEventListener('mousedown', exit);
    window.addEventListener('keydown', exit);
    return () => {
      window.removeEventListener('mousemove', onMouseMove);
      window.removeEventListener('mousedown', exit);
      window.removeEventListener('keydown', exit);
    };
  }, [bootstrap.mode]);

  return (
    <div className="fixed inset-0 overflow-hidden bg-black">
      <div id={`renderer-stream-slot-fullscreen-${bootstrap.rendererNodeId}`} className="absolute inset-0" />
      <canvas
        id={`renderer-mirror-${bootstrap.rendererNodeId}`}
        width={outputSize.width}
        height={outputSize.height}
        className="absolute inset-0 w-full h-full object-contain"
      />
      <canvas ref={runtimeCanvasRef} className="hidden" />
      {error && (
        <div className="absolute inset-0 flex items-center justify-center p-8 text-center text-sm text-white bg-black">
          {error}
        </div>
      )}
    </div>
  );
}


function fileName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

function imageMimeType(path: string): string {
  const extension = path.split('.').pop()?.toLowerCase();
  if (extension === 'jpg' || extension === 'jpeg') return 'image/jpeg';
  if (extension === 'webp') return 'image/webp';
  if (extension === 'gif') return 'image/gif';
  if (extension === 'bmp') return 'image/bmp';
  if (extension === 'tif' || extension === 'tiff') return 'image/tiff';
  return 'image/png';
}
