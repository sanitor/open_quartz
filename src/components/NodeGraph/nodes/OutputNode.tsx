import { Handle, Position, type NodeProps, type Node } from '@xyflow/react';
import type { ShaderNodeData } from '../../../types';
import { useGraphStore } from '../../../store/useGraphStore';

const PORT_COLOR = '#8e8e93';
const HEADER_H = 28;

type OutputNodeType = Node<ShaderNodeData>;

export function OutputNode({ id, data, selected }: NodeProps<OutputNodeType>) {
  const outputPreviews = useGraphStore((s) => s.outputPreviews);
  const nodeErrors = useGraphStore((s) => s.nodeErrors);
  const edges = useGraphStore((s) => s.edges);
  const error = nodeErrors[id];
  const preview = outputPreviews[id];
  let accent = '#8e8e93';
  if (error) accent = '#ff3b30';
  else if (preview) accent = '#30d158';

  const inputPort = data.inputs[0];
  const outputPort = data.outputs[0];
  const inputConnected = inputPort && edges.some((e) => e.targetHandle === inputPort.id);

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

      <div className="flex items-stretch">
        {inputPort && (
          <div className="relative w-3 flex items-center">
            <Handle
              type="target"
              position={Position.Left}
              id={inputPort.id}
              className="!w-3 !h-3 !border-2"
              style={{
                borderColor: PORT_COLOR,
                backgroundColor: inputConnected ? PORT_COLOR : 'transparent',
              }}
            />
          </div>
        )}

        <div className="flex-1 min-w-0 p-2">
          {preview ? (
            <img src={preview} alt="preview" className="w-full h-24 object-contain rounded border border-[#e8e8ed]" style={{ imageRendering: 'pixelated' }} />
          ) : (
            <div className="flex items-center justify-center text-[11px] text-[#aeaeb2] border-2 border-dashed border-[#d2d2d7] rounded" style={{ height: 80 }}>
              Press Run to preview
            </div>
          )}
          {data.resolvedWidth && data.resolvedHeight && (
            <div className="text-[9px] text-[#aeaeb2] text-center mt-1">
              {(data.outFormat ?? 'rgba8').toUpperCase()} {data.resolvedWidth}×{data.resolvedHeight}
            </div>
          )}
        </div>

        {outputPort && (
          <div className="relative w-3 flex items-center">
            <Handle
              type="source"
              position={Position.Right}
              id={outputPort.id}
              className="!w-2.5 !h-2.5 !border-2 !border-white"
              style={{ backgroundColor: PORT_COLOR }}
            />
          </div>
        )}
      </div>
    </div>
  );
}
