import { useMemo, useState } from 'react';
import type { Edge, Node } from '@xyflow/react';
import type { ProjectFile, ShaderNodeData } from '../types';
import {
  collectScreenSaverGraph,
  rendererCandidates,
  screenSaverInputCandidates,
  type ScreenSaverExportRequest,
} from '../screensaver';

interface ScreenSaverExportDialogProps {
  nodes: Node<ShaderNodeData>[];
  edges: Edge[];
  project: ProjectFile;
  activeRendererId: string | null;
  onClose: () => void;
}

export function ScreenSaverExportDialog({
  nodes,
  edges,
  project,
  activeRendererId,
  onClose,
}: ScreenSaverExportDialogProps) {
  const inputs = useMemo(() => screenSaverInputCandidates(nodes), [nodes]);
  const renderers = useMemo(() => rendererCandidates(nodes, edges), [nodes, edges]);
  const initialRenderer = renderers.find((item) => item.id === activeRendererId && item.connected)
    ?? renderers.find((item) => item.connected);
  const [rendererNodeId, setRendererNodeId] = useState(initialRenderer?.id ?? '');
  const [exposed, setExposed] = useState(() => new Set(inputs.map((input) => input.nodeId)));
  const [exporting, setExporting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const toggleInput = (nodeId: string) => {
    setExposed((current) => {
      const next = new Set(current);
      if (next.has(nodeId)) next.delete(nodeId);
      else next.add(nodeId);
      return next;
    });
  };

  const handleExport = async () => {
    if (!rendererNodeId) return;
    setExporting(true);
    setError(null);
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const { save } = await import('@tauri-apps/plugin-dialog');
      const selected = await save({
        defaultPath: `${project.name}.scr`,
        filters: [{ name: 'Windows Screen Saver', extensions: ['scr'] }],
      });
      if (!selected) return;
      const outputPath = selected.toLowerCase().endsWith('.scr') ? selected : `${selected}.scr`;
      const exportedProject = collectScreenSaverGraph(structuredClone(project), rendererNodeId);


      const request: ScreenSaverExportRequest = {
        outputPath,
        name: exportedProject.name,
        projectJson: JSON.stringify(exportedProject),
        rendererNodeId,
        exposedInputs: inputs.filter((input) => exposed.has(input.nodeId)),
      };
      await invoke('screen_saver_export', { request });
      onClose();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setExporting(false);
    }
  };

  return (
    <>
      <div className="fixed inset-0 bg-black/25 z-50" onClick={exporting ? undefined : onClose} />
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="screen-saver-export-title"
        className="fixed top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 z-50 w-[460px] max-h-[80vh] overflow-auto bg-white rounded-xl shadow-2xl border border-[#d2d2d7] p-5"
      >
        <div id="screen-saver-export-title" className="text-[13px] font-bold text-[#1d1d1f]">
          EXPORT AS WINDOWS SCREEN SAVER
        </div>
        <div className="text-[10px] text-[#86868b] mt-1 mb-4">
          The exported .scr stores the selected graph and references existing media and the installed OpenQuartz runtime.
        </div>

        <label className="block text-[10px] font-bold text-[#1d1d1f] mb-1">OUTPUT RENDERER</label>
        <select
          value={rendererNodeId}
          onChange={(event) => setRendererNodeId(event.target.value)}
          className="w-full text-[11px] px-2 py-1.5 border border-[#d2d2d7] rounded bg-white outline-none focus:border-[#007aff]"
        >
          {renderers.length === 0 && <option value="">No Renderer nodes</option>}
          {renderers.map((renderer) => (
            <option key={renderer.id} value={renderer.id} disabled={!renderer.connected}>
              {renderer.label}{renderer.connected ? '' : ' (not connected)'}
            </option>
          ))}
        </select>
        <div className="text-[9px] text-[#86868b] mt-1">
          Output is resampled to the active monitor or preview window resolution at runtime.
        </div>

        <div className="text-[10px] font-bold text-[#1d1d1f] mt-4 mb-1">CONTROL PANEL INPUTS</div>
        <div className="border border-[#e8e8ed] rounded-lg overflow-hidden">
          {inputs.length === 0 ? (
            <div className="px-3 py-2 text-[10px] text-[#86868b]">No image or video inputs are available.</div>
          ) : inputs.map((input) => (
            <label key={input.nodeId} className="flex items-center gap-2 px-3 py-2 border-b last:border-b-0 border-[#e8e8ed] hover:bg-[#f5f5f7] cursor-pointer">
              <input
                type="checkbox"
                checked={exposed.has(input.nodeId)}
                onChange={() => toggleInput(input.nodeId)}
                className="accent-[#007aff]"
              />
              <span className="text-[11px] text-[#1d1d1f] flex-1">{input.label}</span>
              <span className="text-[9px] font-bold text-[#86868b] uppercase">{input.kind}</span>
            </label>
          ))}
        </div>
        <div className="text-[9px] text-[#86868b] mt-1">
          Checked inputs can be replaced from the Windows Screen Saver Settings dialog.
        </div>

        {error && <div role="alert" className="mt-3 text-[10px] text-[#ff3b30] bg-[#fff2f1] rounded px-2 py-1.5">{error}</div>}

        <div className="flex justify-end gap-2 mt-5">
          <button disabled={exporting} onClick={onClose} className="text-[10px] font-bold text-[#86868b] hover:text-[#1d1d1f] disabled:opacity-50 px-3 py-1.5">
            CANCEL
          </button>
          <button
            disabled={exporting || !rendererNodeId}
            onClick={() => void handleExport()}
            className="text-[10px] font-bold text-white bg-[#007aff] hover:bg-[#0066d6] disabled:bg-[#aeaeb2] px-3 py-1.5 rounded"
          >
            {exporting ? 'EXPORTING…' : 'EXPORT .SCR'}
          </button>
        </div>
      </div>
    </>
  );
}
