import { useGraphStore } from '../../store/useGraphStore';
import { ShaderEditor } from './ShaderEditor';
import { PortInspector } from './PortInspector';
import { useCallback, useMemo, useState } from 'react';
import { ImageLightbox } from '../ImageLightbox';
import type { FramebufferFormat, TextureFilter, TextureWrap } from '../../types';
import { generateRawPreview } from '../../utils/rawPreview';

const FB_FORMATS: { label: string; value: FramebufferFormat }[] = [
  { label: 'RGBA8', value: 'rgba8' },
  { label: 'RGBA32F', value: 'rgba32f' },
  { label: 'RG8', value: 'rg8' },
  { label: 'RG32F', value: 'rg32f' },
  { label: 'R8', value: 'r8' },
  { label: 'R32F', value: 'r32f' },
  { label: 'NV12', value: 'nv12' },
];


const OUT_FORMATS: { label: string; value: FramebufferFormat }[] = [
  { label: 'RGBA8', value: 'rgba8' },
  { label: 'RGBA32F', value: 'rgba32f' },
  { label: 'RG8', value: 'rg8' },
  { label: 'RG32F', value: 'rg32f' },
  { label: 'R8', value: 'r8' },
  { label: 'R32F', value: 'r32f' },
];

export function SidePanel() {
  const { nodes, selectedNodeId, updateNodeData, removeNode, outputPreviews, nodeErrors } = useGraphStore();
  const selectedNode = nodes.find((n) => n.id === selectedNodeId);
  const data = selectedNode?.data;
  const nodeError = selectedNodeId ? nodeErrors[selectedNodeId] : undefined;
  const [lightboxSrc, setLightboxSrc] = useState<string | null>(null);

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

  const isSampler2D = !!data && data.type === 'input' && data.inputDataType === 'sampler2D';
  const isFramebuffer = !!data && data.inputMode === 'framebuffer';

  const inputPreviewSrc = useMemo(() => {
    if (!data || !isSampler2D) return null;
    if (isFramebuffer) {
      if (!data.rawDataUrl || !data.fbWidth || !data.fbHeight) return null;
      return generateRawPreview(data.rawDataUrl, (data.fbFormat ?? 'rgba8') as FramebufferFormat, data.fbWidth, data.fbHeight, data.fbStride);
    }
    return data.imageDataUrl ?? null;
  }, [data, isSampler2D, isFramebuffer, data?.rawDataUrl, data?.fbFormat, data?.fbWidth, data?.fbHeight, data?.fbStride, data?.imageDataUrl]);

  if (!selectedNode || !data) return null;

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
            <ShaderEditor key={selectedNodeId} code={data.shaderCode} onChange={handleShaderChange} />
          </div>
        </div>
      )}

      {/* Output node: format, size, sampling, preview */}
      {data.type === 'output' ? (
        <>
          <div className="px-4 py-3 border-b border-[#e8e8ed] overflow-y-auto flex-shrink-0">
            {/* Format */}
            <div className="mb-3">
              <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">Format</label>
              <select
                value={data.outFormat ?? 'rgba8'}
                onChange={(e) => updateNodeData(selectedNodeId!, { outFormat: e.target.value as FramebufferFormat })}
                className="w-full text-[12px] text-[#1d1d1f] bg-[#f5f5f7] rounded px-2 py-1 border border-[#d2d2d7] outline-none focus:border-[#007aff]"
              >
                {OUT_FORMATS.map((f) => (
                  <option key={f.value} value={f.value}>{f.label}</option>
                ))}
              </select>
            </div>

            {/* Width / Height controls */}
            <div className="mb-3">
              <label className="flex items-center gap-1.5 text-[10px] text-[#86868b] font-medium mb-1.5 cursor-pointer">
                <input
                  type="checkbox"
                  checked={data.autoSize !== false}
                  onChange={(e) => {
                    if (e.target.checked) {
                      updateNodeData(selectedNodeId!, { autoSize: true, width: undefined, height: undefined });
                    } else {
                      updateNodeData(selectedNodeId!, { autoSize: false, width: 512, height: 512 });
                    }
                  }}
                  className="accent-[#007aff]"
                />
                Auto Size
              </label>
              {data.autoSize === false ? (
                <div className="flex items-center gap-3">
                  <div className="flex-1">
                    <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">Width</label>
                    <input
                      type="number"
                      min={1}
                      max={8192}
                      value={String(data.width ?? 512)}
                      onChange={(e) => updateNodeData(selectedNodeId!, { width: parseInt(e.target.value) || 512 })}
                      className="w-full text-[12px] text-[#1d1d1f] bg-[#f5f5f7] rounded px-2 py-1 border border-[#d2d2d7] outline-none focus:border-[#007aff]"
                    />
                  </div>
                  <div className="flex-1">
                    <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">Height</label>
                    <input
                      type="number"
                      min={1}
                      max={8192}
                      value={String(data.height ?? 512)}
                      onChange={(e) => updateNodeData(selectedNodeId!, { height: parseInt(e.target.value) || 512 })}
                      className="w-full text-[12px] text-[#1d1d1f] bg-[#f5f5f7] rounded px-2 py-1 border border-[#d2d2d7] outline-none focus:border-[#007aff]"
                    />
                  </div>
                </div>
              ) : (
                <div className="flex items-center gap-3">
                  <div className="flex-1">
                    <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">Width</label>
                    <input
                      type="number"
                      disabled
                      value={String(data.resolvedWidth ?? '')}
                      className="w-full text-[12px] text-[#aeaeb2] bg-[#f5f5f7] rounded px-2 py-1 border border-[#e8e8ed] cursor-default"
                      placeholder="—"
                    />
                  </div>
                  <div className="flex-1">
                    <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">Height</label>
                    <input
                      type="number"
                      disabled
                      value={String(data.resolvedHeight ?? '')}
                      className="w-full text-[12px] text-[#aeaeb2] bg-[#f5f5f7] rounded px-2 py-1 border border-[#e8e8ed] cursor-default"
                      placeholder="—"
                    />
                  </div>
                </div>
              )}
            </div>

            {/* Sampling */}
            <div>
              <div className="text-[10px] text-[#86868b] font-medium mb-2">SAMPLING</div>
              <div className="flex items-center gap-3">
                <div className="flex-1">
                  <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">Filter</label>
                  <select
                    value={data.texFilter ?? 'linear'}
                    onChange={(e) => updateNodeData(selectedNodeId!, { texFilter: e.target.value as TextureFilter })}
                    className="w-full text-[12px] text-[#1d1d1f] bg-[#f5f5f7] rounded px-2 py-1 border border-[#d2d2d7] outline-none focus:border-[#007aff]"
                  >
                    <option value="linear">LINEAR</option>
                    <option value="nearest">NEAREST</option>
                  </select>
                </div>
                <div className="flex-1">
                  <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">Wrap</label>
                  <select
                    value={data.texWrap ?? 'clamp'}
                    onChange={(e) => updateNodeData(selectedNodeId!, { texWrap: e.target.value as TextureWrap })}
                    className="w-full text-[12px] text-[#1d1d1f] bg-[#f5f5f7] rounded px-2 py-1 border border-[#d2d2d7] outline-none focus:border-[#007aff]"
                  >
                    <option value="clamp">CLAMP</option>
                    <option value="repeat">REPEAT</option>
                    <option value="mirror">MIRROR</option>
                  </select>
                </div>
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
                  onClick={() => setLightboxSrc(outputPreviews[selectedNodeId!])}
                  className="max-w-full max-h-full object-contain rounded border border-[#d2d2d7] cursor-pointer hover:opacity-90 transition-opacity"
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

          {/* Image dimensions (read-only) */}
          {data.type === 'input' && data.inputDataType === 'sampler2D' && data.inputMode !== 'framebuffer' && data.imageWidth && data.imageHeight && (
            <div className="px-4 py-2 border-t border-[#e8e8ed] flex-shrink-0">
              <span className="text-[11px] text-[#86868b]">{data.imageWidth} × {data.imageHeight}</span>
            </div>
          )}

          {/* Framebuffer config */}
          {data.type === 'input' && data.inputMode === 'framebuffer' && (
            <div className="px-4 py-3 border-t border-[#e8e8ed] flex-shrink-0">
              <div className="text-[10px] text-[#86868b] font-medium mb-2">FRAMEBUFFER CONFIG</div>
              <div className="mb-2">
                <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">Format</label>
                <select
                  value={data.fbFormat ?? 'rgba8'}
                  onChange={(e) => updateNodeData(selectedNodeId!, { fbFormat: e.target.value as FramebufferFormat })}
                  className="w-full text-[12px] text-[#1d1d1f] bg-[#f5f5f7] rounded px-2 py-1 border border-[#d2d2d7] outline-none focus:border-[#007aff]"
                >
                  {FB_FORMATS.map((f) => (
                    <option key={f.value} value={f.value}>{f.label}</option>
                  ))}
                </select>
              </div>
              <div className="flex items-center gap-3 mb-2">
                <div className="flex-1">
                  <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">Width</label>
                  <input
                    type="number"
                    min={1}
                    max={8192}
                    value={String(data.fbWidth ?? '')}
                    onChange={(e) => updateNodeData(selectedNodeId!, { fbWidth: parseInt(e.target.value) || undefined })}
                    className="w-full text-[12px] text-[#1d1d1f] bg-[#f5f5f7] rounded px-2 py-1 border border-[#d2d2d7] outline-none focus:border-[#007aff]"
                    placeholder="required"
                  />
                </div>
                <div className="flex-1">
                  <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">Height</label>
                  <input
                    type="number"
                    min={1}
                    max={8192}
                    value={String(data.fbHeight ?? '')}
                    onChange={(e) => updateNodeData(selectedNodeId!, { fbHeight: parseInt(e.target.value) || undefined })}
                    className="w-full text-[12px] text-[#1d1d1f] bg-[#f5f5f7] rounded px-2 py-1 border border-[#d2d2d7] outline-none focus:border-[#007aff]"
                    placeholder="required"
                  />
                </div>
              </div>
              <div>
                <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">Stride (bytes per row)</label>
                <input
                  type="number"
                  min={0}
                  value={String(data.fbStride ?? '')}
                  onChange={(e) => updateNodeData(selectedNodeId!, { fbStride: parseInt(e.target.value) || undefined })}
                  className="w-full text-[12px] text-[#1d1d1f] bg-[#f5f5f7] rounded px-2 py-1 border border-[#d2d2d7] outline-none focus:border-[#007aff]"
                  placeholder="auto"
                />
              </div>
            </div>
          )}

          {/* Sampling config for all sampler2D inputs */}
          {data.type === 'input' && data.inputDataType === 'sampler2D' && (
            <div className="px-4 py-3 border-t border-[#e8e8ed] flex-shrink-0">
              <div className="text-[10px] text-[#86868b] font-medium mb-2">SAMPLING</div>
              <div className="flex items-center gap-3">
                <div className="flex-1">
                  <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">Filter</label>
                  <select
                    value={data.texFilter ?? 'linear'}
                    onChange={(e) => updateNodeData(selectedNodeId!, { texFilter: e.target.value as TextureFilter })}
                    className="w-full text-[12px] text-[#1d1d1f] bg-[#f5f5f7] rounded px-2 py-1 border border-[#d2d2d7] outline-none focus:border-[#007aff]"
                  >
                    <option value="linear">LINEAR</option>
                    <option value="nearest">NEAREST</option>
                  </select>
                </div>
                <div className="flex-1">
                  <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">Wrap</label>
                  <select
                    value={data.texWrap ?? 'clamp'}
                    onChange={(e) => updateNodeData(selectedNodeId!, { texWrap: e.target.value as TextureWrap })}
                    className="w-full text-[12px] text-[#1d1d1f] bg-[#f5f5f7] rounded px-2 py-1 border border-[#d2d2d7] outline-none focus:border-[#007aff]"
                  >
                    <option value="clamp">CLAMP</option>
                    <option value="repeat">REPEAT</option>
                    <option value="mirror">MIRROR</option>
                  </select>
                </div>
              </div>
            </div>
          )}

          {/* Preview for sampler2D inputs */}
          {isSampler2D && (
            <div className="flex-1 flex flex-col min-h-0 border-t border-[#e8e8ed]">
              <div className="px-4 py-1.5 text-[11px] text-[#86868b] font-medium">
                PREVIEW
              </div>
              <div className="flex-1 flex items-center justify-center bg-[#f5f5f7] overflow-hidden p-2">
                {inputPreviewSrc ? (
                  <img
                    src={inputPreviewSrc}
                    alt="preview"
                    onClick={() => setLightboxSrc(inputPreviewSrc)}
                    className="max-w-full max-h-full object-contain rounded border border-[#d2d2d7] cursor-pointer hover:opacity-90 transition-opacity"
                  />
                ) : (
                  <span className="text-[12px] text-[#aeaeb2]">
                    {isFramebuffer ? 'Load file and set width/height' : 'Load an image'}
                  </span>
                )}
              </div>
            </div>
          )}
        </>
      )}

      {lightboxSrc && <ImageLightbox src={lightboxSrc} onClose={() => setLightboxSrc(null)} />}
    </aside>
  );
}
