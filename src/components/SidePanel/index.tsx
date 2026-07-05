import { useGraphStore } from '../../store/useGraphStore';
import { ShaderEditor } from './ShaderEditor';
import { PortInspector } from './PortInspector';
import { useCallback } from 'react';


export function SidePanel() {
  const { nodes, selectedNodeId, updateNodeData, removeNode, outputPreviews, nodeErrors } = useGraphStore();
  const selectedNode = nodes.find((n) => n.id === selectedNodeId);
  const data = selectedNode?.data;
  const nodeError = selectedNodeId ? nodeErrors[selectedNodeId] : undefined;

  if (!selectedNode || !data) return null;

  const handleLabelChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      if (selectedNodeId) {
        updateNodeData(selectedNodeId, { label: e.target.value });
      }
    },
    [selectedNodeId, updateNodeData],
  );

  const handleShaderChange = useCallback(
    (code: string) => {
      if (selectedNodeId) {
        updateNodeData(selectedNodeId, { shaderCode: code });
      }
    },
    [selectedNodeId, updateNodeData],
  );

  const handleUniformChange = useCallback(
    (label: string, value: unknown) => {
      if (!selectedNodeId || !data) return;
      updateNodeData(selectedNodeId, {
        uniforms: { ...data.uniforms, [label]: value },
      });
    },
    [selectedNodeId, data, updateNodeData],
  );

  if (!selectedNode || !data) {
    return (
      <aside className="w-72 bg-white border-l border-[#d2d2d7] flex-shrink-0 flex items-center justify-center">
        <p className="text-[12px] text-[#aeaeb2] select-none">Select a node to edit</p>
      </aside>
    );
  }

  return (
    <aside className="w-80 bg-white border-l border-[#d2d2d7] flex-shrink-0 flex flex-col overflow-hidden">
      {/* Node header */}
      <div className="flex items-center justify-between px-4 py-2 border-b border-[#e8e8ed]">
        <div>
          <label className="text-[10px] text-[#86868b] font-medium">{data.type.toUpperCase()}</label>
          <input
            type="text"
            value={data.label}
            onChange={handleLabelChange}
            className="block w-full text-[13px] font-semibold text-[#1d1d1f] bg-transparent outline-none border-b border-transparent focus:border-[#007aff]"
          />
        </div>
        <button
          onClick={() => { if (selectedNodeId) removeNode(selectedNodeId); }}
          className="text-[11px] text-[#ff3b30] hover:text-[#d70015]"
        >
          Delete
        </button>
      </div>

      {/* Error display */}
      {nodeError && (
        <div className="px-4 py-2 bg-[#fff0f0] border-b border-[#ffd0d0]">
          <div className="text-[10px] text-[#ff3b30] font-medium mb-0.5">Error</div>
          <div className="text-[11px] text-[#1d1d1f] font-mono whitespace-pre-wrap break-words leading-snug max-h-24 overflow-y-auto">{nodeError}</div>
        </div>
      )}

      {/* Shader editor (only for shader type) */}
      {data.type === 'shader' && (
        <div className="flex-1 flex flex-col min-h-0">
          <div className="px-4 py-1.5 text-[11px] text-[#86868b] font-medium border-b border-[#e8e8ed]">
            Shader Editor
          </div>
          <div className="flex-1 overflow-hidden">
            <ShaderEditor code={data.shaderCode} onChange={handleShaderChange} />
          </div>
        </div>
      )}

      {/* Output node: inputs, size controls, then preview */}
      {data.type === 'output' ? (
        <>
          <div className="px-4 py-3 border-b border-[#e8e8ed] overflow-y-auto flex-shrink-0">
            <PortInspector
              inputs={data.inputs}
              outputs={data.outputs}
              uniforms={data.uniforms}
              onUniformChange={handleUniformChange}
            />

            {/* Width / Height controls */}
            <div className="mt-3 flex items-center gap-3">
              <div className="flex-1">
                <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">Width</label>
                <input
                  type="number"
                  min={1}
                  max={8192}
                  value={String(data.width ?? '')}
                  onChange={(e) => updateNodeData(selectedNodeId!, { width: parseInt(e.target.value) || undefined })}
                  className="w-full text-[12px] text-[#1d1d1f] bg-[#f5f5f7] rounded px-2 py-1 border border-[#d2d2d7] outline-none focus:border-[#007aff]"
                  placeholder="auto"
                />
              </div>
              <div className="flex-1">
                <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">Height</label>
                <input
                  type="number"
                  min={1}
                  max={8192}
                  value={String(data.height ?? '')}
                  onChange={(e) => updateNodeData(selectedNodeId!, { height: parseInt(e.target.value) || undefined })}
                  className="w-full text-[12px] text-[#1d1d1f] bg-[#f5f5f7] rounded px-2 py-1 border border-[#d2d2d7] outline-none focus:border-[#007aff]"
                  placeholder="auto"
                />
              </div>
            </div>
          </div>
          <div className="flex-1 flex flex-col min-h-0 border-t border-[#e8e8ed]">
            <div className="px-4 py-1.5 text-[11px] text-[#86868b] font-medium">
              PREVIEW
            </div>
            <div className="flex-1 flex items-center justify-center bg-[#f5f5f7] overflow-hidden p-2">
              {outputPreviews[selectedNodeId!] ? (
                <img
                  src={outputPreviews[selectedNodeId!]}
                  alt="output"
                  className="max-w-full max-h-full object-contain rounded border border-[#d2d2d7]"
                />
              ) : (
                <span className="text-[12px] text-[#aeaeb2]">Press Run to preview</span>
              )}
            </div>
          </div>
        </>
      ) : (
        <>
          {/* Port inspector for shader/input nodes */}
          <div className="px-4 py-3 border-t border-[#e8e8ed] overflow-y-auto flex-shrink-0 max-h-64">
            <PortInspector
              inputs={data.inputs}
              outputs={data.outputs}
              uniforms={data.uniforms}
              onUniformChange={handleUniformChange}
            />
          </div>
        </>
      )}
    </aside>
  );
}
