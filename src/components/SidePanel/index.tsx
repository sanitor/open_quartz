import { useGraphStore } from '../../store/useGraphStore';
import { ShaderEditor } from './ShaderEditor';
import { PortInspector } from './PortInspector';
import { useCallback } from 'react';


export function SidePanel() {
  const { nodes, selectedNodeId, updateNodeData, removeNode } = useGraphStore();
  const selectedNode = nodes.find((n) => n.id === selectedNodeId);
  const data = selectedNode?.data;

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
          <span className="text-[10px] text-[#86868b] font-medium">{data.type}</span>
          <h3 className="text-[13px] font-semibold text-[#1d1d1f]">{data.label}</h3>
        </div>
        <button
          onClick={() => { if (selectedNodeId) removeNode(selectedNodeId); }}
          className="text-[11px] text-[#ff3b30] hover:text-[#d70015]"
        >
          Delete
        </button>
      </div>

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

      {/* Port inspector */}
      <div className="px-4 py-3 border-t border-[#e8e8ed] overflow-y-auto flex-shrink-0 max-h-64">
        <PortInspector
          inputs={data.inputs}
          outputs={data.outputs}
          uniforms={data.uniforms}
          onUniformChange={handleUniformChange}
        />
      </div>
    </aside>
  );
}
