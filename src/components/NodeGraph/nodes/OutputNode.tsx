import { Handle, Position, type NodeProps, type Node } from '@xyflow/react';
import type { ShaderNodeData } from '../../../types';

const ROW_H = 26;
const HEADER_H = 28;

type OutputNodeType = Node<ShaderNodeData>;

export function OutputNode({ data, selected }: NodeProps<OutputNodeType>) {
  const accent = '#ff3b30';

  return (
    <div
      className={`rounded-xl border bg-white shadow-sm ${
        selected ? 'border-[#007aff] shadow-md' : 'border-[#d2d2d7]'
      }`}
    >
      <div
        className="flex items-center px-3 text-[12px] font-semibold text-white rounded-t-xl"
        style={{ height: HEADER_H, backgroundColor: accent }}
      >
        {data.label}
      </div>

      {/* Input handles */}
      <div style={{ paddingTop: 2, paddingBottom: 6 }}>
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
              style={{ backgroundColor: '#ff3b30' }}
            />
            <span className="ml-4">{port.label}</span>
            <span className="ml-auto text-[9px] text-[#aeaeb2]">{port.dataType}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
