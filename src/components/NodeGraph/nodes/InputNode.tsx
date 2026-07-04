import { useCallback, useRef } from 'react';
import { Handle, Position, type NodeProps, type Node } from '@xyflow/react';
import type { ShaderNodeData, DataType } from '../../../types';
import { DATA_TYPE_COLORS } from '../../../types';
import { useGraphStore } from '../../../store/useGraphStore';

const ROW_H = 26;
const HEADER_H = 28;

type InputNodeType = Node<ShaderNodeData>;

const INPUT_DATA_TYPES: DataType[] = ['float', 'int', 'bool', 'vec2', 'vec3', 'vec4', 'sampler2D'];

export function InputNode({ id, data, selected }: NodeProps<InputNodeType>) {
  const updateNodeInputType = useGraphStore((s) => s.updateNodeInputType);
  const updateNodeData = useGraphStore((s) => s.updateNodeData);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const currentType = (data.inputDataType ?? 'float') as DataType;
  const accent = '#007aff';

  const handleTypeChange = useCallback(
    (e: React.ChangeEvent<HTMLSelectElement>) => {
      const newType = e.target.value as DataType;
      if (newType === 'sampler2D' && data.imageDataUrl) {
        updateNodeInputType(id, newType);
      } else if (newType !== 'sampler2D') {
        updateNodeData(id, { imageDataUrl: undefined, imageFileName: undefined });
        updateNodeInputType(id, newType);
      } else {
        updateNodeInputType(id, newType);
      }
    },
    [id, data.imageDataUrl, updateNodeInputType, updateNodeData],
  );

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
        className="flex items-center justify-between px-3 text-[12px] font-semibold text-white rounded-t-xl"
        style={{ height: HEADER_H, backgroundColor: accent }}
      >
        <span>{data.label}</span>
        <select
          value={currentType}
          onChange={handleTypeChange}
          className="text-[10px] bg-white border border-[#d2d2d7] rounded px-1 py-0.5 text-[#1d1d1f] outline-none cursor-pointer"
        >
          {INPUT_DATA_TYPES.map((t) => (
            <option key={t} value={t}>{t}</option>
          ))}
        </select>
      </div>

      {currentType === 'sampler2D' ? (
        <div onClick={handleImageClick} className="cursor-pointer">
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
      ) : (
        <div style={{ paddingTop: 2, paddingBottom: 2 }}>
          {data.inputs.map((port) => (
            <div
              key={port.id}
              className="flex items-center justify-between text-[11px] text-[#1d1d1f] px-3"
              style={{ height: ROW_H }}
            >
              <span>{port.label}</span>
              <input
                type="text"
                defaultValue={String(port.defaultValue ?? '')}
                className="w-20 bg-white border border-[#d2d2d7] rounded px-1.5 py-0.5 text-right text-[#1d1d1f] text-[10px] outline-none focus:border-[#007aff]"
                placeholder="value"
              />
            </div>
          ))}
        </div>
      )}

      {/* Output port */}
      <div style={{ paddingTop: 2, paddingBottom: 6 }}>
        {data.outputs.map((port) => (
          <div
            key={port.id}
            className="flex items-center justify-end text-[11px] text-[#1d1d1f] px-3"
            style={{ height: ROW_H, position: 'relative' }}
          >
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
