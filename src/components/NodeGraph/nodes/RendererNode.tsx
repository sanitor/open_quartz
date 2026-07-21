import { type NodeProps, type Node } from '@xyflow/react';
import type { ShaderNodeData } from '../../../types';
import { useGraphStore } from '../../../store/useGraphStore';
import { NodeShell, MENU_ICONS, InputPortRow, PortDivider, type NodeStatus } from './NodeShell';

const MAX_PREVIEW_W = 200;

type RendererNodeType = Node<ShaderNodeData>;

export function RendererNode({ id, data, selected }: NodeProps<RendererNodeType>) {
  const edges = useGraphStore((s) => s.edges);
  const nodeErrors = useGraphStore((s) => s.nodeErrors);
  const loopState = useGraphStore((s) => s.loopState);
  const isPlaying = loopState !== 'stopped';
  const error = nodeErrors[id];

  const inputPort = data.inputs[0];
  const connected = inputPort ? edges.some((e) => e.targetHandle === inputPort.id) : false;
  const status: NodeStatus = error ? 'error' : !connected ? 'not-ready' : 'ready';

  const expanded = data.expanded !== false;
  const rw = data.resolvedWidth ?? 512;
  const rh = data.resolvedHeight ?? 512;
  const aspect = rh / rw;
  const previewW = Math.min(rw, MAX_PREVIEW_W);
  const previewH = Math.round(previewW * aspect);

  const handleToggle = () => {
    useGraphStore.getState().updateNodeData(id, { expanded: !expanded });
  };

  const handlePreviewClick = () => {
    useGraphStore.getState().setActiveRenderer(id);
  };

  const headerExtra = (
    <button
      onClick={handleToggle}
      className="text-white/80 hover:text-white text-[10px] leading-none cursor-default"
    >
      {expanded ? '▴' : '▾'}
    </button>
  );

  return (
    <NodeShell
      icon={MENU_ICONS.renderer}
      typeName="Renderer"
      label={data.label}
      status={status}
      selected={selected}
      minWidth={200}
      headerExtra={headerExtra}
    >
      {/* Input port */}
      {inputPort && (
        <div style={{ paddingTop: 2, paddingBottom: expanded ? 2 : 6 }}>
          <InputPortRow port={inputPort} connected={connected} error={!!error} />
        </div>
      )}

      {/* Expanded: preview area + resolution */}
      {expanded && isPlaying && (
        <>
          <PortDivider />
          <div className="px-2 py-2">
            <canvas
              id={`renderer-mirror-${id}`}
              onClick={handlePreviewClick}
              className="cursor-pointer rounded border border-[#e8e8ed] bg-[#1d1d1f]"
              width={rw}
              height={rh}
              style={{ width: previewW, height: previewH, display: 'block' }}
            />
            <div className="text-[9px] text-[#aeaeb2] text-center mt-1">
              {rw}×{rh}
            </div>
          </div>
        </>
      )}
    </NodeShell>
  );
}
