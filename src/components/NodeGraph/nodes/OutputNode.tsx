import { Handle, Position, type NodeProps, type Node } from '@xyflow/react';
import type { ShaderNodeData } from '../../../types';
import { useGraphStore } from '../../../store/useGraphStore';

const PORT_COLOR = '#8e8e93';
const ROW_H = 26;
const HEADER_H = 28;

type OutputNodeType = Node<ShaderNodeData>;

export function OutputNode({ id, data, selected }: NodeProps<OutputNodeType>) {
  const outputPreviews = useGraphStore((s) => s.outputPreviews);
  const edges = useGraphStore((s) => s.edges);
  const hasOutput = !!outputPreviews[id];
  const accent = hasOutput ? '#30d158' : '#8e8e93';

  return (
    <div
      className={`rounded-xl border bg-white shadow-sm ${
        selected ? 'border-[#007aff] shadow-md' : 'border-[#d2d2d7]'
      }`}
    >
      <div
        className="flex items-center px-3 rounded-t-xl"
        style={{ height: HEADER_H, backgroundColor: accent }}
      >
        <span className="text-xs font-semibold text-white">OUTPUT</span>
        <span className="ml-auto text-[10px] text-white/60 font-medium">{data.label}</span>
      </div>

      {/* Input handles */}
      <div style={{ paddingTop: 2, paddingBottom: 6 }}>
        {data.inputs.map((port) => {
          const connected = edges.some((e) => e.targetHandle === port.id);
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
                  borderColor: PORT_COLOR,
                  backgroundColor: connected ? PORT_COLOR : 'transparent',
                }}
              />
              <span className="ml-4">{port.label}</span>
              <span className="ml-auto text-[9px] text-[#aeaeb2]">{port.dataType}</span>
            </div>
          );
        })}
      </div>
    </div>
  );
}
