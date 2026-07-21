import { type NodeProps, type Node } from '@xyflow/react';
import type { ShaderNodeData } from '../../../types';
import { useGraphStore } from '../../../store/useGraphStore';
import { NodeShell, MENU_ICONS, InputPortRow, OutputPortRow, PortDivider, type NodeStatus } from './NodeShell';

type ShaderNodeType = Node<ShaderNodeData>;

export function ShaderNode({ id, data, selected }: NodeProps<ShaderNodeType>) {
  const edges = useGraphStore((s) => s.edges);
  const nodeErrors = useGraphStore((s) => s.nodeErrors);
  const outputPreviews = useGraphStore((s) => s.outputPreviews);
  const error = nodeErrors[id];
  const hasUnconnectedInput = data.inputs.some(
    (port) => !edges.some((e) => e.targetHandle === port.id)
  );
  const hasFeedback = /\bpreviousFrame\b/.test(data.shaderCode);

  const status: NodeStatus = error ? 'error' : hasUnconnectedInput ? 'not-ready' : 'ready';

  const feedbackBadge = hasFeedback ? (
    <span className="ml-2 text-[9px] text-white/80 font-medium bg-white/20 rounded px-1.5 py-0.5">FB</span>
  ) : null;

  return (
    <NodeShell
      icon={MENU_ICONS.shader}
      typeName={data.templateName ?? data.label}
      label={data.label}
      status={status}
      selected={selected}
      minWidth={200}
      headerExtra={feedbackBadge}
    >
      {/* Input ports */}
      <div style={{ paddingTop: 2, paddingBottom: 2 }}>
        {data.inputs.map((port) => {
          const connected = edges.some((e) => e.targetHandle === port.id);
          return (
            <InputPortRow
              key={port.id}
              port={port}
              connected={connected}
              error={!connected && !!error}
            />
          );
        })}
      </div>

      {/* Divider between input/output */}
      {data.inputs.length > 0 && data.outputs.length > 0 && <PortDivider />}

      {/* Output ports */}
      <div style={{ paddingTop: 2, paddingBottom: 6 }}>
        {data.outputs.map((port) => (
          <OutputPortRow key={port.id} port={port} />
        ))}
      </div>

      {/* Preview thumbnail */}
      {outputPreviews[id] && (
        <div className="px-2 pb-2">
          <img src={outputPreviews[id]} alt="preview" className="w-full h-16 object-contain rounded border border-[#e8e8ed]" style={{ imageRendering: 'pixelated' }} />
          {data.resolvedWidth && data.resolvedHeight && (
            <div className="text-[9px] text-[#aeaeb2] text-center mt-0.5">
              {(data.outFormat ?? 'rgba8').toUpperCase()} {data.resolvedWidth}×{data.resolvedHeight}
            </div>
          )}
        </div>
      )}
    </NodeShell>
  );
}
