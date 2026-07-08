import { Handle, Position, type NodeProps, type Node } from '@xyflow/react';
import type { ShaderNodeData } from '../../../types';
import { useGraphStore } from '../../../store/useGraphStore';

const PORT_COLOR = '#8e8e93';
const ROW_H = 26;
const HEADER_H = 28;
const ACCENT = '#34c759';
const MAX_PREVIEW_W = 200;

type RendererNodeType = Node<ShaderNodeData>;

export function RendererNode({ id, data, selected }: NodeProps<RendererNodeType>) {
  const edges = useGraphStore((s) => s.edges);
  const nodeErrors = useGraphStore((s) => s.nodeErrors);
  const outputPreviews = useGraphStore((s) => s.outputPreviews);
  const error = nodeErrors[id];

  const inputPort = data.inputs[0];
  const connected = inputPort ? edges.some((e) => e.targetHandle === inputPort.id) : false;
  const accent = error ? '#ff3b30' : !connected ? '#8e8e93' : ACCENT;

  const expanded = data.expanded !== false;
  const rw = data.rendererWidth ?? 512;
  const rh = data.rendererHeight ?? 512;
  const aspect = rh / rw;
  const previewW = Math.min(rw, MAX_PREVIEW_W);
  const previewH = Math.round(previewW * aspect);

  const handleToggle = () => {
    useGraphStore.getState().updateNodeData(id, { expanded: !expanded });
  };

  const handlePreviewClick = () => {
    useGraphStore.getState().setActiveRenderer(id);
  };

  return (
    <div
      className={`rounded-xl border bg-white shadow-sm min-w-[200px] ${
        selected ? 'border-[#007aff] shadow-md' : 'border-[#d2d2d7]'
      }`}
    >
      {/* Header */}
      <div
        className="flex items-center px-3 rounded-t-xl"
        style={{ height: HEADER_H, backgroundColor: accent }}
      >
        <span className="text-xs font-semibold text-white">RENDERER</span>
        <span className="ml-auto text-[10px] text-white/60 font-medium">{data.label}</span>
        <button
          onClick={handleToggle}
          className="ml-2 text-white/80 hover:text-white text-[10px] leading-none cursor-default"
        >
          {expanded ? '▴' : '▾'}
        </button>
      </div>

      {/* Input port */}
      {inputPort && (
        <div style={{ paddingTop: 2, paddingBottom: expanded ? 2 : 6 }}>
          <div
            className="flex items-center text-[11px] text-[#1d1d1f] px-3"
            style={{ height: ROW_H, position: 'relative' }}
          >
            <Handle
              type="target"
              position={Position.Left}
              id={inputPort.id}
              className="!w-3 !h-3 !border-2"
              style={{
                borderColor: error ? '#ff3b30' : PORT_COLOR,
                backgroundColor: error ? '#ff3b30' : connected ? PORT_COLOR : 'transparent',
              }}
            />
            <span className={`ml-4 ${error ? 'text-[#ff3b30] font-medium' : ''}`}>{inputPort.label}</span>
            <span className="ml-auto text-[9px] text-[#aeaeb2]">{inputPort.dataType}</span>
          </div>
        </div>
      )}

      {/* Expanded: preview area + resolution */}
      {expanded && (
        <>
          <div className="mx-3 border-t border-[#f0f0f0]" />
          <div className="px-2 py-2">
            {/* Preview placeholder or actual preview */}
            <div
              onClick={handlePreviewClick}
              className="cursor-pointer rounded border border-[#e8e8ed] overflow-hidden"
              style={{ width: previewW, height: previewH }}
            >
              {outputPreviews[id] ? (
                <img
                  src={outputPreviews[id]}
                  alt="preview"
                  className="w-full h-full object-contain"
                  style={{ imageRendering: 'pixelated' }}
                />
              ) : (
                <div className="w-full h-full bg-[#1d1d1f]" />
              )}
            </div>
            {/* Resolution */}
            <div className="text-[9px] text-[#aeaeb2] text-center mt-1">
              {rw}×{rh}
            </div>
          </div>
        </>
      )}
    </div>
  );
}
