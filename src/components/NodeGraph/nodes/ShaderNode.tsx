import { Handle, Position, type NodeProps, type Node } from '@xyflow/react';
import type { ShaderNodeData } from '../../../types';
import { DATA_TYPE_COLORS } from '../../../types';

const ROW_H = 26;
const HEADER_H = 28;

type ShaderNodeType = Node<ShaderNodeData>;

export function ShaderNode({ data, selected }: NodeProps<ShaderNodeType>) {
  const accent = getAccent(data.type);

  return (
    <div
      className={`rounded-xl border bg-white shadow-sm min-w-[200px] ${
        selected ? 'border-[#007aff] shadow-md' : 'border-[#d2d2d7]'
      }`}
    >
      {/* Header */}
      <div
        className="flex items-center gap-2 px-3 text-xs text-white font-semibold rounded-t-xl"
        style={{ height: HEADER_H, backgroundColor: accent }}
      >
        <span className="text-[11px] font-medium opacity-80">{data.type}</span>
        <span>{data.label}</span>
        <span className="ml-auto text-[9px] font-medium opacity-60">
          in:{data.inputs.length} out:{data.outputs.length}
        </span>
      </div>

      {/* Input ports */}
      <div style={{ paddingTop: 2, paddingBottom: 2 }}>
        {data.inputs.map((port) => (
          <div
            key={port.id}
            className="flex items-center text-[11px] text-[#1d1d1f] px-3"
            style={{ height: ROW_H, position: 'relative' }}
          >
            <Handle
              type="target"
              position={Position.Left}
              id={port.id}
              className="!w-2.5 !h-2.5 !border-2 !border-white"
              style={{ backgroundColor: DATA_TYPE_COLORS[port.dataType] }}
            />
            <span className="ml-4">{port.label}</span>
            <span className="ml-auto text-[9px] text-[#aeaeb2]">{port.dataType}</span>
          </div>
        ))}
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
              style={{ backgroundColor: DATA_TYPE_COLORS[port.dataType] }}
            />
          </div>
        ))}
      </div>
    </div>
  );
}

function getAccent(type: string): string {
  switch (type) {
    case 'shader': return '#af52de';
    case 'input': return '#007aff';
    case 'output': return '#ff3b30';
    default: return '#8e8e93';
  }
}
