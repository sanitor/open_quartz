import { useCallback, useMemo, useRef } from 'react';
import { Handle, Position, type NodeProps, type Node } from '@xyflow/react';
import type { ShaderNodeData, DataType, FramebufferFormat } from '../../../types';
import { useGraphStore } from '../../../store/useGraphStore';
import { generateRawPreview } from '../../../utils/rawPreview';
import { checkIsTauri, tauriOpenVideoFile, tauriConvertFileSrc } from '../../../utils/tauri';

const VEC_COMPONENTS: Record<string, string[]> = {
  vec2: ['x', 'y'],
  vec3: ['x', 'y', 'z'],
  vec4: ['x', 'y', 'z', 'w'],
  ivec2: ['x', 'y'],
  ivec3: ['x', 'y', 'z'],
  ivec4: ['x', 'y', 'z', 'w'],
  uvec2: ['x', 'y'],
  uvec3: ['x', 'y', 'z'],
  uvec4: ['x', 'y', 'z', 'w'],
  bvec2: ['x', 'y'],
  bvec3: ['x', 'y', 'z'],
  bvec4: ['x', 'y', 'z', 'w'],
};

const MAT_DIMS: Record<string, number> = {
  mat2: 2,
  mat3: 3,
  mat4: 4,
};

const PORT_COLOR = '#8e8e93';
const ROW_H = 26;
const HEADER_H = 28;

type InputNodeType = Node<ShaderNodeData>;

function arrayBufferToBase64(buf: ArrayBuffer): string {
  const bytes = new Uint8Array(buf);
  let binary = '';
  for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
  return btoa(binary);
}

export function InputNode({ id, data, selected }: NodeProps<InputNodeType>) {
  const updateNodeData = useGraphStore((s) => s.updateNodeData);
  const nodeErrors = useGraphStore((s) => s.nodeErrors);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const rawFileInputRef = useRef<HTMLInputElement>(null);
  const videoFileInputRef = useRef<HTMLInputElement>(null);

  const currentType = (data.inputDataType ?? 'float') as DataType;
  const isFramebuffer = data.inputMode === 'framebuffer';
  const isVideo = data.inputMode === 'video';
  const error = nodeErrors[id];
  const hasNoValue = currentType === 'sampler2D' && !data.imageDataUrl && !data.rawDataUrl && !data.videoUrl;
  const accent = error ? '#ff3b30' : hasNoValue ? '#8e8e93' : '#007aff';

  const fbPreview = useMemo(() => {
    if (!isFramebuffer || !data.rawDataUrl || !data.fbWidth || !data.fbHeight) return null;
    return generateRawPreview(data.rawDataUrl, (data.fbFormat ?? 'rgba8') as FramebufferFormat, data.fbWidth, data.fbHeight, data.fbStride);
  }, [isFramebuffer, data.rawDataUrl, data.fbFormat, data.fbWidth, data.fbHeight, data.fbStride]);

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
        const img = new Image();
        img.onload = () => {
          updateNodeData(id, { imageDataUrl: dataUrl, imageFileName: file.name, imageWidth: img.naturalWidth, imageHeight: img.naturalHeight });
        };
        img.onerror = () => {
          updateNodeData(id, { imageDataUrl: dataUrl, imageFileName: file.name });
        };
        img.src = dataUrl;
      };
      reader.readAsDataURL(file);
      e.target.value = '';
    },
    [id, updateNodeData],
  );

  const loadVideoFromUrl = useCallback((url: string, fileName: string) => {
    const video = document.createElement('video');
    video.preload = 'metadata';
    video.onloadedmetadata = () => {
      updateNodeData(id, {
        videoSourceType: 'file',
        videoUrl: url,
        videoFileName: fileName,
        imageWidth: video.videoWidth,
        imageHeight: video.videoHeight,
        videoLoop: data.videoLoop ?? true,
        videoPlaybackRate: data.videoPlaybackRate ?? 1,
      });
    };
    video.onerror = () => {
      updateNodeData(id, {
        videoSourceType: 'file',
        videoUrl: url,
        videoFileName: fileName,
        videoLoop: data.videoLoop ?? true,
        videoPlaybackRate: data.videoPlaybackRate ?? 1,
      });
    };
    video.src = url;
  }, [id, data.videoLoop, data.videoPlaybackRate, updateNodeData]);

  const handleVideoClick = useCallback(() => {
    checkIsTauri().then((tauri) => {
      if (!tauri) {
        videoFileInputRef.current?.click();
        return;
      }
      tauriOpenVideoFile().then((filePath) => {
        if (!filePath) return;
        tauriConvertFileSrc(filePath).then((assetUrl) => {
          const fileName = filePath.split('/').pop() ?? filePath.split('\\').pop() ?? filePath;
          updateNodeData(id, { videoFilePath: filePath });
          loadVideoFromUrl(assetUrl, fileName);
        });
      });
    });
  }, [id, updateNodeData, loadVideoFromUrl]);

  const handleVideoFileChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const file = e.target.files?.[0];
      if (!file) return;
      const url = URL.createObjectURL(file);
      loadVideoFromUrl(url, file.name);
      e.target.value = '';
    },
    [loadVideoFromUrl],
  );

  const handleRawFileChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const file = e.target.files?.[0];
      if (!file) return;
      const reader = new FileReader();
      reader.onload = (ev) => {
        const buf = ev.target?.result as ArrayBuffer;
        const b64 = arrayBufferToBase64(buf);
        updateNodeData(id, { rawDataUrl: `data:application/octet-stream;base64,${b64}`, rawFileName: file.name });
      };
      reader.readAsArrayBuffer(file);
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
        <span className="text-xs font-semibold text-white">
          {isVideo ? 'VIDEO' : isFramebuffer ? 'FRAMEBUFFER' : currentType === 'sampler2D' ? 'IMAGE' : currentType.toUpperCase()}
        </span>
        <span className="ml-auto text-[10px] text-white/60 font-medium">{data.label}</span>
      </div>

      {currentType === 'sampler2D' && isFramebuffer ? (
        <div className="flex items-stretch">
          <div onClick={() => rawFileInputRef.current?.click()} className="cursor-pointer flex-1 min-w-0">
            <input ref={rawFileInputRef} type="file" onChange={handleRawFileChange} className="hidden" />
            {data.rawDataUrl ? (
              <div className="p-2">
                {fbPreview ? (
                  <img src={fbPreview} alt="preview" className="w-full h-24 object-contain rounded border border-[#e8e8ed]" />
                ) : (
                  <div className="flex items-center justify-center bg-[#f5f5f7] rounded border border-[#e8e8ed]" style={{ height: 80 }}>
                    <span className="text-[20px]">&#x1F4BE;</span>
                  </div>
                )}
                <div className="text-[10px] text-[#86868b] text-center mt-1 truncate px-2">
                  {data.rawFileName ?? 'loaded'}
                </div>
                {data.fbWidth && data.fbHeight && (
                  <div className="text-[9px] text-[#aeaeb2] text-center">
                    {(data.fbFormat ?? 'rgba8').toUpperCase()} {data.fbWidth}×{data.fbHeight}
                  </div>
                )}
              </div>
            ) : (
              <div className="flex items-center justify-center text-[11px] text-[#aeaeb2] mx-3 my-2 border-2 border-dashed border-[#d2d2d7] rounded" style={{ height: 80 }}>
                Click to load raw file
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
      ) : currentType === 'sampler2D' && isVideo ? (
        <div className="flex items-stretch">
          <div onClick={handleVideoClick} className="cursor-pointer flex-1 min-w-0">
            <input ref={videoFileInputRef} type="file" accept="video/*" onChange={handleVideoFileChange} className="hidden" />
            {data.videoUrl ? (
              <div className="p-2">
                <video src={data.videoUrl} muted loop playsInline className="w-full h-24 object-contain rounded border border-[#e8e8ed]" />
                <div className="text-[10px] text-[#86868b] text-center mt-1 truncate px-2">
                  {data.videoFileName ?? 'loaded'}
                </div>
              </div>
            ) : data.videoFileName ? (
              <div onClick={handleVideoClick} className="flex flex-col items-center justify-center text-[11px] text-[#aeaeb2] mx-3 my-2 border-2 border-dashed border-[#ff9500] rounded cursor-pointer" style={{ height: 80 }}>
                <span className="text-[10px] truncate px-2">{data.videoFileName}</span>
                <span className="text-[9px] mt-1">Click to reload</span>
              </div>
            ) : (
              <div className="flex items-center justify-center text-[11px] text-[#aeaeb2] mx-3 my-2 border-2 border-dashed border-[#d2d2d7] rounded" style={{ height: 80 }}>
                Click to load video
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
      ) : currentType === 'sampler2D' ? (
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
          {data.inputs.map((port) => {
            const comps = VEC_COMPONENTS[port.dataType];
            const matDim = MAT_DIMS[port.dataType];
            const arr: number[] = Array.isArray(data.uniforms?.[port.label])
              ? (data.uniforms[port.label] as number[])
              : new Array(matDim ? matDim * matDim : 4).fill(0);
            return (
              <div
                key={port.id}
                className="flex text-[11px] text-[#1d1d1f] px-3"
                style={{ minHeight: ROW_H, position: 'relative', paddingTop: comps || matDim ? 4 : 0, paddingBottom: comps || matDim ? 4 : 0 }}
              >
                {matDim ? (
                  <div className="flex-1 flex flex-col gap-0.5 mr-7 justify-center">
                    {Array.from({ length: matDim }, (_, row) => (
                      <div key={row} className="flex gap-0.5">
                        {Array.from({ length: matDim }, (_, col) => {
                          const idx = col * matDim + row;
                          return (
                            <input
                              key={col}
                              type="text"
                              value={String(arr[idx] ?? 0)}
                              onChange={(e) => {
                                const total = matDim * matDim;
                                const next = Array.from({ length: total }, (_, k) => arr[k] ?? 0);
                                const parsed = parseFloat(e.target.value);
                                next[idx] = isNaN(parsed) ? 0 : parsed;
                                updateNodeData(id, { uniforms: { ...data.uniforms, [port.label]: next } });
                              }}
                              className="flex-1 min-w-0 bg-white border border-[#d2d2d7] rounded px-0.5 py-0.5 text-center text-[#1d1d1f] text-[9px] outline-none focus:border-[#007aff]"
                            />
                          );
                        })}
                      </div>
                    ))}
                  </div>
                ) : comps ? (
                  <div className="flex-1 flex flex-col gap-0.5 mr-7 justify-center">
                    {comps.map((c, i) => (
                      <div key={c} className="flex items-center gap-1">
                        <span className="text-[9px] text-[#aeaeb2] font-mono w-2">{c}</span>
                        <input
                          type="text"
                          value={String(arr[i] ?? 0)}
                          onChange={(e) => {
                            const next = [...comps.map((_, j) => arr[j] ?? 0)];
                            const parsed = parseFloat(e.target.value);
                            next[i] = isNaN(parsed) ? 0 : parsed;
                            updateNodeData(id, { uniforms: { ...data.uniforms, [port.label]: next } });
                          }}
                          className="flex-1 bg-white border border-[#d2d2d7] rounded px-1 py-0.5 text-right text-[#1d1d1f] text-[10px] outline-none focus:border-[#007aff]"
                        />
                      </div>
                    ))}
                  </div>
                ) : (
                  <div className="flex items-center flex-1 min-h-[26px]">
                    <input
                      type="text"
                      value={String(data.uniforms?.[port.label] ?? port.defaultValue ?? '')}
                      onChange={(e) => updateNodeData(id, { uniforms: { ...data.uniforms, [port.label]: e.target.value } })}
                      className="flex-1 bg-white border border-[#d2d2d7] rounded px-1.5 py-0.5 text-right text-[#1d1d1f] text-[10px] outline-none focus:border-[#007aff] mr-7"
                      placeholder="—"
                    />
                  </div>
                )}
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
            );
          })}
        </div>
      )}
    </div>
  );
}
