import { useCallback, useRef } from 'react';
import { Handle, Position, type NodeProps, type Node } from '@xyflow/react';
import type { ShaderNodeData, DataType } from '../../../types';
import { useGraphStore } from '../../../store/useGraphStore';

const PORT_COLOR = '#8e8e93';
const ROW_H = 26;
const HEADER_H = 28;

type InputNodeType = Node<ShaderNodeData>;

export function InputNode({ id, data, selected }: NodeProps<InputNodeType>) {
  const updateNodeData = useGraphStore((s) => s.updateNodeData);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const currentType = (data.inputDataType ?? 'float') as DataType;
  const accent = '#007aff';

  const handleImageClick = useCallback(() => {
    fileInputRef.current?.click();
  }, []);

  const handleFileChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const file = e.target.files?.[0];
      if (!file) return;
      const reader = new FileReader();
      reader.onload = (ev) => {
        const dataUrl = ev.target?.result as string;
        updateNodeData(id, { imageDataUrl: dataUrl, imageFileName: file.name });
      };
      reader.readAsDataURL(file);
      e.target.value = '';
    },
    [id, updateNodeData],
  );

  return (
    <div
      className={`rounded-xl border bg-white shadow-sm min-w-[180px] ${
        selected ? 'border-[#007aff] shadow-md' : 'border-[#d2d2d7]'
      }`}
    >
      <div
        className="flex items-center px-3 rounded-t-xl"
        style={{ height: HEADER_H, backgroundColor: accent }}
      >
        <span className="text-xs font-semibold text-white">{currentType.toUpperCase()}</span>
        <span className="ml-auto text-[10px] text-white/60 font-medium">{data.label}</span>
      </div>

      {currentType === 'sampler2D' ? (
        <div className="flex items-stretch">
          <div onClick={handleImageClick} className="cursor-pointer flex-1 min-w-0">
            <input ref={fileInputRef} type="file" accept="image/*" onChange={handleFileChange} className="hidden" />
            {data.imageDataUrl ? (
              <div className="p-2">
                <img src={data.imageDataUrl} alt={data.imageFileName ?? ''} className="w-full h-24 object-contain rounded border border-[#e8e8ed]" />
                <div className="text-[10px] text-[#86868b] text-center mt-1 truncate px-2">
                  {data.imageFileName ?? 'loaded'}
                </div>
              </div>
            ) : (
              <div className="flex items-center justify-center text-[11px] text-[#aeaeb2] mx-3 my-2 border-2 border-dashed border-[#d2d2d7] rounded" style={{ height: 80 }}>
                Click to load image
              </div>
            )}
          </div>
          {data.outputs[0] && (
            <div className="relative w-3 flex items-center">
              <Handle
                type="source"
                position={Position.Right}
                id={data.outputs[0].id}
                className="!w-2.5 !h-2.5 !border-2 !border-white"
                style={{ backgroundColor: PORT_COLOR }}
              />
            </div>
          )}
        </div>
      ) : (
        <div style={{ paddingTop: 2, paddingBottom: 2 }}>
          {data.inputs.map((port) => (
            <div
              key={port.id}
              className="flex items-center text-[11px] text-[#1d1d1f] px-3"
              style={{ height: ROW_H, position: 'relative' }}
            >
                <input
                type="text"
                value={String(data.uniforms?.[port.label] ?? port.defaultValue ?? '')}
                onChange={(e) => updateNodeData(id, { uniforms: { ...data.uniforms, [port.label]: e.target.value } })}
                className="flex-1 bg-white border border-[#d2d2d7] rounded px-1.5 py-0.5 text-right text-[#1d1d1f] text-[10px] outline-none focus:border-[#007aff] mr-7"
                placeholder="—"
              />
              {data.outputs[0] && (
                <Handle
                  type="source"
                  position={Position.Right}
                  id={data.outputs[0].id}
                  className="!w-2.5 !h-2.5 !border-2 !border-white"
                  style={{ backgroundColor: PORT_COLOR }}
                />
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
