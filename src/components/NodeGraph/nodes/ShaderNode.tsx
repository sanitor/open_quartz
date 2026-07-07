import { Handle, Position, type NodeProps, type Node } from '@xyflow/react';
import type { ShaderNodeData } from '../../../types';
import { useGraphStore } from '../../../store/useGraphStore';

const PORT_COLOR = '#8e8e93';
const ROW_H = 26;
const HEADER_H = 28;

type ShaderNodeType = Node<ShaderNodeData>;

export function ShaderNode({ id, data, selected }: NodeProps<ShaderNodeType>) {
  const edges = useGraphStore((s) => s.edges);
  const nodeErrors = useGraphStore((s) => s.nodeErrors);
  const outputPreviews = useGraphStore((s) => s.outputPreviews);
  const nodes = useGraphStore((s) => s.nodes);
  const error = nodeErrors[id];
  const hasUnconnectedInput = data.inputs.some(
    (port) => !edges.some((e) => e.targetHandle === port.id)
  );
  const accent = error ? '#ff3b30' : hasUnconnectedInput ? '#8e8e93' : getAccent(data.type);

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
        <span className="text-xs font-semibold text-white">{data.type.toUpperCase()}</span>
        <span className="ml-auto text-[10px] text-white/60 font-medium">{data.label}</span>
      </div>

      {/* Input ports */}
      <div style={{ paddingTop: 2, paddingBottom: 2 }}>
        {data.inputs.map((port) => {
          const connected = edges.some((e) => e.targetHandle === port.id);
          const portErr = !connected && !!error;
          return (
            <div
              key={port.id}
              className="flex items-center text-[11px] text-[#1d1d1f] px-3"
              style={{ height: ROW_H, position: 'relative' }}
            >
              <Handle
                type="target"
                position={Position.Left}
                id={port.id}
                className="!w-3 !h-3 !border-2"
                style={{
                  borderColor: portErr ? '#ff3b30' : PORT_COLOR,
                  backgroundColor: portErr ? '#ff3b30' : connected ? PORT_COLOR : 'transparent',
                }}
              />
              <span className={`ml-4 ${portErr ? 'text-[#ff3b30] font-medium' : ''}`}>{port.label}</span>
              <span className="ml-auto text-[9px] text-[#aeaeb2]">{port.dataType}</span>
            </div>
          );
        })}
      </div>

      {/* Divider between input/output */}
      {data.inputs.length > 0 && data.outputs.length > 0 && (
        <div className="mx-3 border-t border-[#f0f0f0]" />
      )}

      {/* Output ports */}
      <div style={{ paddingTop: 2, paddingBottom: 6 }}>
        {data.outputs.map((port) => (
          <div
            key={port.id}
            className="flex items-center justify-end text-[11px] text-[#1d1d1f] px-3"
            style={{ height: ROW_H, position: 'relative' }}
          >
            <span className="mr-auto text-[9px] text-[#aeaeb2]">{port.dataType}</span>
            <span className="mr-4">{port.label}</span>
            <Handle
              type="source"
              position={Position.Right}
              id={port.id}
              className="!w-2.5 !h-2.5 !border-2 !border-white"
              style={{ backgroundColor: PORT_COLOR }}
            />
          </div>
        ))}
      </div>

      {/* Preview thumbnail for leaf shader nodes */}
      {(() => {
        const isLeaf = (data.type === 'shader' || data.type === 'constant') && !edges.some((e) => e.source === id && nodes.find((n) => n.id === e.target)?.data.type !== 'input');
        const preview = outputPreviews[id];
        if (!isLeaf || !preview) return null;
        return (
          <div className="px-2 pb-2">
            <img src={preview} alt="preview" className="w-full h-16 object-contain rounded border border-[#e8e8ed]" style={{ imageRendering: 'pixelated' }} />
            {data.resolvedWidth && data.resolvedHeight && (
              <div className="text-[9px] text-[#aeaeb2] text-center mt-0.5">
                {(data.outFormat ?? 'rgba8').toUpperCase()} {data.resolvedWidth}×{data.resolvedHeight}
              </div>
            )}
          </div>
        );
      })()}
    </div>
  );
}

function getAccent(type: string): string {
  switch (type) {
    case 'shader': return '#af52de';
    case 'input': return '#007aff';
    default: return '#8e8e93';
  }
}
