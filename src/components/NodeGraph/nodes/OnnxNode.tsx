import { useRef, useCallback } from 'react';
import { type NodeProps, type Node } from '@xyflow/react';
import type { ShaderNodeData } from '../../../types';
import { DATA_TYPE_COLORS } from '../../../types';
import { useGraphStore } from '../../../store/useGraphStore';
import { NodeShell, MENU_ICONS, InputPortRow, OutputPortRow, PortDivider, type NodeStatus } from './NodeShell';
type OnnxNodeType = Node<ShaderNodeData>;

export function OnnxNode({ id, data, selected }: NodeProps<OnnxNodeType>) {
  const edges = useGraphStore((s) => s.edges);
  const nodeErrors = useGraphStore((s) => s.nodeErrors);
  const outputPreviews = useGraphStore((s) => s.outputPreviews);
  const error = nodeErrors[id];
  const hasUnconnectedInput = data.inputs.some(
    (port) => !edges.some((e) => e.targetHandle === port.id),
  );
  const portsVisible = data.onnxStatus === 'ready';
  const status: NodeStatus = error ? 'error' : (data.onnxStatus !== 'ready' || hasUnconnectedInput) ? 'not-ready' : 'ready';

  return (
    <NodeShell icon={MENU_ICONS.onnx} typeName={data.onnxModelId ?? 'ONNX'} label={data.label} status={status} selected={selected} minWidth={220}>

      {data.onnxSource === 'custom' && data.onnxCustomFileName && (
        <div className="px-3 py-1 text-[10px] text-[#86868b] truncate border-b border-[#f0f0f0]">
          📄 {data.onnxCustomFileName}
        </div>
      )}

      {data.onnxStatus === 'not-downloaded' && data.onnxSource !== 'custom' && (
        <div className="px-3 py-2 text-[10px] text-[#86868b]">Waiting to download...</div>
      )}

      {data.onnxStatus === 'downloading' && (
        <div className="mx-3 my-1">
          <div className="flex items-center justify-between text-[9px] text-[#86868b] mb-0.5">
            <span>Downloading...</span>
            <span>{Math.round((data.onnxProgress ?? 0) * 100)}%</span>
          </div>
          <div className="h-1.5 bg-[#e8e8ed] rounded-full overflow-hidden">
            <div className="h-full bg-[#007aff] rounded-full transition-all" style={{ width: `${(data.onnxProgress ?? 0) * 100}%` }} />
          </div>
        </div>
      )}

      {data.onnxStatus === 'downloaded' && (
        <div className="px-3 py-2 text-[10px] text-[#86868b]">Loading...</div>
      )}

      {data.onnxStatus === 'introspecting' && (
        <div className="px-3 py-2 text-[10px] text-[#86868b]">Analyzing model...</div>
      )}

      {data.onnxStatus === 'ready' && data.onnxBackend === 'wasm' && (
        <div className="px-3 py-1 text-[9px] font-medium text-[#ff9f0a]">CPU fallback</div>
      )}

      {data.onnxStatus === 'error' && (
        <div className="px-3 py-2 text-[10px] text-[#ff3b30]">{data.onnxError ?? 'Unknown error'}</div>
      )}

      {data.onnxSource === 'custom' && !data.onnxCustomFileName && (!data.onnxStatus || data.onnxStatus === 'not-downloaded') && (
        <OnnxFilePicker nodeId={id} />
      )}
      {portsVisible && (
        <div style={{ paddingTop: 2, paddingBottom: 2 }}>
          {data.inputs.map((port) => {
            const connected = edges.some((e) => e.targetHandle === port.id);
            const portErr = !connected && !!error;
            return (
              <InputPortRow key={port.id} port={port} connected={connected} error={portErr} />
            );
          })}
        </div>
      )}

      {portsVisible && data.inputs.length > 0 && data.outputs.length > 0 && (
        <PortDivider />
      )}

      {portsVisible && (
        <div style={{ paddingTop: 2, paddingBottom: 6 }}>
          {data.outputs.map((port) => (
            <OutputPortRow key={port.id} port={port} color={DATA_TYPE_COLORS[port.dataType]} />
          ))}
        </div>
      )}

      {outputPreviews[id] && (
        <div className="px-2 pb-2">
          <img
            src={outputPreviews[id]}
            alt="detections"
            className="w-full h-16 object-contain rounded border border-[#e8e8ed]"
            style={{ imageRendering: 'pixelated' }}
          />
          {data.resolvedWidth && data.resolvedHeight && (
            <div className="text-[9px] text-[#aeaeb2] text-center mt-0.5">
              ONNX {data.resolvedWidth}×{data.resolvedHeight}
            </div>
          )}
        </div>
      )}
    </NodeShell>
  );
}

function OnnxFilePicker({ nodeId }: { nodeId: string }) {
  const inputRef = useRef<HTMLInputElement>(null);
  const loadCustomOnnxModel = useGraphStore((s) => s.loadCustomOnnxModel);

  const handleFileChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const file = e.target.files?.[0];
      if (!file) return;
      const reader = new FileReader();
      reader.onload = () => {
        const buffer = reader.result as ArrayBuffer;
        loadCustomOnnxModel(nodeId, buffer, file.name);
      };
      reader.readAsArrayBuffer(file);
    },
    [nodeId, loadCustomOnnxModel],
  );

  return (
    <div className="px-3 py-2">
      <input
        ref={inputRef}
        type="file"
        accept=".onnx"
        onChange={handleFileChange}
        style={{ display: 'none' }}
      />
      <button
        type="button"
        onClick={() => inputRef.current?.click()}
        style={{
          fontSize: 10,
          fontFamily: 'system-ui',
          color: '#007aff',
          background: 'none',
          border: '1px solid #007aff',
          borderRadius: 4,
          padding: '2px 8px',
          cursor: 'pointer',
        }}
      >
        Select .onnx file…
      </button>
    </div>
  );
}
