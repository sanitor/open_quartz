import { useGraphStore } from '../../store/useGraphStore';
import { ShaderEditor } from './ShaderEditor';
import { PortInspector } from './PortInspector';
import { OnnxPanel } from './OnnxPanel';
import { useCallback, useMemo, useRef, useState } from 'react';
import { ImageLightbox } from '../ImageLightbox';
import type { FramebufferFormat, TextureFilter, TextureWrap, DataType } from '../../types';
import { generateRawPreview } from '../../utils/rawPreview';
import { MATH_OPS, MATH_CATEGORIES, getMathPorts } from '../../catalog/mathOps';

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

// --- Accordion section header ---
function SectionHeader({ title, expanded, onClick, extra }: { title: string; expanded: boolean; onClick: () => void; extra?: React.ReactNode }) {
  return (
    <div
      onClick={onClick}
      className="flex items-center justify-between px-4 py-1.5 border-t border-[#e8e8ed] cursor-pointer hover:bg-[#f5f5f7] select-none"
    >
      <span className="flex items-center gap-1.5 text-[11px] text-[#86868b] font-medium">
        <span className="text-[16px]">{expanded ? '▾' : '▸'}</span>
        {title}
      </span>
      {extra}
    </div>
  );
}

export function SidePanel() {
  const { nodes, selectedNodeId, updateNodeData, removeNode, outputPreviews, nodeErrors } = useGraphStore();
  const selectedNode = nodes.find((n) => n.id === selectedNodeId);
  const data = selectedNode?.data;
  const nodeError = selectedNodeId ? nodeErrors[selectedNodeId] : undefined;
  const [lightboxSrc, setLightboxSrc] = useState<string | null>(null);
  const [collapsedSections, setCollapsedSections] = useState<Set<string>>(new Set());
  const [panelWidth, setPanelWidth] = useState(320);
  const resizing = useRef(false);
  const startX = useRef(0);
  const startW = useRef(320);

  const handleResizeStart = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    resizing.current = true;
    startX.current = e.clientX;
    startW.current = panelWidth;
    const onMove = (ev: MouseEvent) => {
      if (!resizing.current) return;
      const delta = startX.current - ev.clientX;
      setPanelWidth(Math.max(240, Math.min(640, startW.current + delta)));
    };
    const onUp = () => {
      resizing.current = false;
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
    };
    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
  }, [panelWidth]);

  const toggleSection = useCallback((section: string) => {
    setCollapsedSections((prev) => {
      const next = new Set(prev);
      if (next.has(section)) next.delete(section);
      else next.add(section);
      return next;
    });
  }, []);

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
      if (selectedNodeId && data) {
        updateNodeData(selectedNodeId, {
          uniforms: { ...data.uniforms, [label]: value },
        });
      }
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
    if (data.inputMode === 'video') return data.videoUrl ?? null;
    return data.imageDataUrl ?? null;
  }, [data, isSampler2D, isFramebuffer, data?.rawDataUrl, data?.fbFormat, data?.fbWidth, data?.fbHeight, data?.fbStride, data?.imageDataUrl, data?.videoUrl, data?.inputMode]);

  if (!selectedNode || !data) return null;

  // Build sections array based on node type
  const sections: { id: string; title: string; content: React.ReactNode; extra?: React.ReactNode; flexFill?: boolean }[] = [];

  // --- SHADER EDITOR (editable for custom, read-only for prebuilt) ---
  if (data.type === 'shader') {
    const isPrebuilt = !!data.shaderTemplateId;
    sections.push({
      id: 'editor',
      title: isPrebuilt ? 'SHADER (READ-ONLY)' : 'SHADER EDITOR',
      flexFill: true,
      content: (
        <div className="flex-1 overflow-hidden">
          <ShaderEditor key={selectedNodeId} code={data.shaderCode} onChange={handleShaderChange} readOnly={isPrebuilt} />
        </div>
      ),
    });
  }

  // --- SYSTEM SOURCE (read-only live values) ---
  if (data.type === 'input' && data.inputMode === 'system') {
    const { currentTime, currentFrame, loopState } = useGraphStore.getState();
    const source = data.systemSource;
    const outputType = source === 'mouse' ? 'vec4' : source === 'resolution' ? 'vec3' : 'float';
    const formatVal = (): string => {
      if (loopState !== 'playing') return '—';
      switch (source) {
        case 'time': return currentTime.toFixed(3) + 's';
        case 'timeDelta': return '~0.016s';
        case 'frame': return String(currentFrame);
        case 'mouse': return 'vec4 (live)';
        case 'resolution': return 'vec3 (live)';
        default: return '—';
      }
    };
    sections.push({
      id: 'systemsource',
      title: (data.templateName ?? data.label).toUpperCase(),
      content: (
        <div className="px-4 py-3 space-y-2">
          <div className="flex items-center justify-between">
            <span className="text-[10px] text-[#86868b] font-medium">OUTPUT TYPE</span>
            <span className="text-[11px] text-[#1d1d1f] font-mono">{outputType}</span>
          </div>
          <div className="flex items-center justify-between">
            <span className="text-[10px] text-[#86868b] font-medium">CURRENT VALUE</span>
            <span className="text-[13px] text-[#1d1d1f] font-mono tabular-nums">{formatVal()}</span>
          </div>
          <div className="text-[9px] text-[#aeaeb2]">Read-only · auto-updated each frame</div>
        </div>
      ),
    });
  }

  // --- PORTS (all nodes) ---
  sections.push({
    id: 'ports',
    title: 'PORTS',
    content: (
      <div className="px-4 py-3 overflow-y-auto">
        <PortInspector
          inputs={data.inputs}
          outputs={data.outputs}
          uniforms={data.uniforms}
          onUniformChange={handleUniformChange}
        />
      </div>
    ),
  });

  // --- MATH CONFIG (math nodes only) ---
  if (data.type === 'math') {
    const edges = useGraphStore.getState().edges;
    const allNodes = useGraphStore.getState().nodes;

    // Infer port types from connected peers
    function inferPortType(portId: string, isInput: boolean): DataType {
      if (isInput) {
        const edge = edges.find((e) => e.target === selectedNodeId && e.targetHandle === portId);
        if (edge) {
          const srcNode = allNodes.find((n) => n.id === edge.source);
          if (srcNode) {
            const srcPort = srcNode.data.outputs.find((p) => p.id === edge.sourceHandle);
            if (srcPort && srcPort.dataType !== 'auto') return srcPort.dataType;
          }
        }
      } else {
        // Output type = widest input type
        const inputTypes = data!.inputs.map((p) => inferPortType(p.id, true));
        const resolved = inputTypes.filter((t) => t !== 'auto');
        if (resolved.length > 0) {
          const widthOrder: DataType[] = ['bool', 'int', 'uint', 'float', 'vec2', 'vec3', 'vec4'];
          let widest = resolved[0];
          for (const t of resolved) {
            if (widthOrder.indexOf(t) > widthOrder.indexOf(widest)) widest = t;
          }
          return widest;
        }
      }
      return 'auto';
    }

    sections.push({
      id: 'mathconfig',
      title: 'MATH CONFIG',
      content: (
        <div className="px-4 py-3 space-y-3">
          {/* Operation selector */}
          <div>
            <label className="text-[10px] text-[#86868b] font-medium block mb-1">OPERATION</label>
            <select
              value={data.mathOp ?? 'add'}
              onChange={(e) => {
                if (!selectedNodeId) return;
                const newOp = MATH_OPS[e.target.value];
                if (!newOp) return;
                const ports = getMathPorts(newOp);
                updateNodeData(selectedNodeId, {
                  mathOp: e.target.value,
                  label: newOp.label,
                  inputs: ports.inputs.map((p) => ({ ...p, id: `${selectedNodeId}_${p.label}` })),
                  outputs: ports.outputs.map((p) => ({ ...p, id: `${selectedNodeId}_${p.label}` })),
                });
              }}
              className="w-full rounded border border-[#d2d2d7] bg-white px-2 py-1 text-[11px] text-[#1d1d1f]"
            >
              {MATH_CATEGORIES.map((cat) => (
                <optgroup key={cat.category} label={cat.category}>
                  {cat.ops.map((opId) => {
                    const op = MATH_OPS[opId];
                    return op ? (
                      <option key={opId} value={opId}>{op.label}</option>
                    ) : null;
                  })}
                </optgroup>
              ))}
            </select>
          </div>

          {/* Ports with inferred types */}
          <div>
            <label className="text-[10px] text-[#86868b] font-medium block mb-1">INPUTS</label>
            {data.inputs.map((port) => {
              const inferred = inferPortType(port.id, true);
              const isConnected = edges.some((e) => e.target === selectedNodeId && e.targetHandle === port.id);
              return (
                <div key={port.id} className="flex items-center gap-2 mb-1.5">
                  <span className="text-[11px] text-[#1d1d1f] font-medium w-4">{port.label}</span>
                  <span className="text-[9px] text-[#86868b] w-10">{inferred}</span>
                  {!isConnected && (
                    <input
                      type="number"
                      step="any"
                      value={Number(data.uniforms?.[port.label] ?? 0)}
                      onChange={(e) => handleUniformChange(port.label, parseFloat(e.target.value) || 0)}
                      className="flex-1 rounded border border-[#d2d2d7] bg-white px-2 py-0.5 text-[11px] text-[#1d1d1f] tabular-nums"
                    />
                  )}
                  {isConnected && (
                    <span className="text-[10px] text-[#86868b] italic">connected</span>
                  )}
                </div>
              );
            })}
          </div>

          {/* Output */}
          <div>
            <label className="text-[10px] text-[#86868b] font-medium block mb-1">OUTPUT</label>
            {data.outputs.map((port) => {
              const inferred = inferPortType(port.id, false);
              return (
                <div key={port.id} className="flex items-center gap-2">
                  <span className="text-[11px] text-[#1d1d1f] font-medium">{port.label}</span>
                  <span className="text-[9px] text-[#86868b]">{inferred}</span>
                </div>
              );
            })}
          </div>
        </div>
      ),
    });
  }

  // --- ONNX CONFIG ---
  if (data.type === 'onnx') {
    sections.push({
      id: 'onnx',
      title: 'ONNX CONFIG',
      content: (
        <OnnxPanel
          nodeId={selectedNodeId!}
          modelId={data.onnxModelId ?? data.onnxCatalogId}
          source={data.onnxSource}
          status={data.onnxStatus}
          backend={data.onnxBackend}
          score={data.onnxScoreThreshold}
          iou={data.onnxIouThreshold}
        />
      ),
    });
  }

  // --- OUTPUT CONFIG (shader / constant) ---
  if (data.type === 'shader' || data.type === 'constant') {
    sections.push({
      id: 'output',
      title: 'OUTPUT CONFIG',
      content: (
        <div className="px-4 py-3 overflow-y-auto">
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
                  <input type="number" min={1} max={8192} value={data.width ?? 512}
                    onChange={(e) => updateNodeData(selectedNodeId!, { width: e.target.value === '' ? undefined : parseInt(e.target.value) })}
                    onBlur={(e) => { if (!e.target.value) updateNodeData(selectedNodeId!, { width: 512 }); }}
                    className="w-full text-[12px] text-[#1d1d1f] bg-[#f5f5f7] rounded px-2 py-1 border border-[#d2d2d7] outline-none focus:border-[#007aff]" />
                </div>
                <div className="flex-1">
                  <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">Height</label>
                  <input type="number" min={1} max={8192} value={data.height ?? 512}
                    onChange={(e) => updateNodeData(selectedNodeId!, { height: e.target.value === '' ? undefined : parseInt(e.target.value) })}
                    onBlur={(e) => { if (!e.target.value) updateNodeData(selectedNodeId!, { height: 512 }); }}
                    className="w-full text-[12px] text-[#1d1d1f] bg-[#f5f5f7] rounded px-2 py-1 border border-[#d2d2d7] outline-none focus:border-[#007aff]" />
                </div>
              </div>
            ) : (
              <div className="flex items-center gap-3">
                <div className="flex-1">
                  <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">Width</label>
                  <input type="number" disabled value={String(data.resolvedWidth ?? '')}
                    className="w-full text-[12px] text-[#aeaeb2] bg-[#f5f5f7] rounded px-2 py-1 border border-[#e8e8ed] cursor-default" placeholder="—" />
                </div>
                <div className="flex-1">
                  <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">Height</label>
                  <input type="number" disabled value={String(data.resolvedHeight ?? '')}
                    className="w-full text-[12px] text-[#aeaeb2] bg-[#f5f5f7] rounded px-2 py-1 border border-[#e8e8ed] cursor-default" placeholder="—" />
                </div>
              </div>
            )}
          </div>
          <div>
            <div className="text-[10px] text-[#86868b] font-medium mb-2">SAMPLING</div>
            <div className="flex items-center gap-3">
              <div className="flex-1">
                <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">Filter</label>
                <select value={data.texFilter ?? 'linear'} onChange={(e) => updateNodeData(selectedNodeId!, { texFilter: e.target.value as TextureFilter })}
                  className="w-full text-[12px] text-[#1d1d1f] bg-[#f5f5f7] rounded px-2 py-1 border border-[#d2d2d7] outline-none focus:border-[#007aff]">
                  <option value="linear">LINEAR</option>
                  <option value="nearest">NEAREST</option>
                </select>
              </div>
              <div className="flex-1">
                <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">Wrap</label>
                <select value={data.texWrap ?? 'clamp'} onChange={(e) => updateNodeData(selectedNodeId!, { texWrap: e.target.value as TextureWrap })}
                  className="w-full text-[12px] text-[#1d1d1f] bg-[#f5f5f7] rounded px-2 py-1 border border-[#d2d2d7] outline-none focus:border-[#007aff]">
                  <option value="clamp">CLAMP</option>
                  <option value="repeat">REPEAT</option>
                  <option value="mirror">MIRROR</option>
                </select>
              </div>
            </div>
          </div>
        </div>
      ),
    });
  }

  // --- FEEDBACK CONFIG (shader / constant) ---
  if (data.type === 'shader' || data.type === 'constant') {
    const needsFeedback = /\bpreviousFrame\b/.test(data.shaderCode);
    sections.push({
      id: 'feedback',
      title: 'FEEDBACK',
      content: (
        <div className="px-4 py-3 space-y-3">
          <div className="flex items-center gap-2">
            <div className={`text-[11px] font-medium px-2 py-0.5 rounded ${needsFeedback ? 'text-[#30b94e] bg-[#e8f8eb]' : 'text-[#aeaeb2] bg-[#f5f5f7]'}`}>
              {needsFeedback ? '● Active' : '○ Inactive'}
            </div>
            <span className="text-[10px] text-[#86868b]">
              {needsFeedback ? 'reads previousFrame' : 'no previousFrame reference'}
            </span>
          </div>
          {needsFeedback && (
            <div>
              <label className="block text-[10px] text-[#86868b] font-medium mb-1">Clear Color (RGBA)</label>
              <div className="flex items-center gap-2">
                {(['R', 'G', 'B', 'A'] as const).map((ch, i) => (
                  <div key={ch} className="flex-1">
                    <label className="block text-[9px] text-[#aeaeb2] text-center">{ch}</label>
                    <input
                      type="number" step="0.01" min="0" max="1"
                      value={data.feedbackClearColor?.[i] ?? 0}
                      onChange={(e) => {
                        const arr: [number, number, number, number] = [...(data.feedbackClearColor ?? [0, 0, 0, 0])] as [number, number, number, number];
                        arr[i] = parseFloat(e.target.value) || 0;
                        updateNodeData(selectedNodeId!, { feedbackClearColor: arr });
                      }}
                      className="w-full text-[11px] text-[#1d1d1f] bg-[#f5f5f7] rounded px-1 py-0.5 border border-[#d2d2d7] outline-none focus:border-[#007aff] text-center tabular-nums"
                    />
                  </div>
                ))}
                <div
                  className="w-8 h-8 rounded border border-[#d2d2d7] flex-shrink-0"
                  style={{
                    backgroundColor: data.feedbackClearColor
                      ? `rgba(${data.feedbackClearColor.map((v, i) => i < 3 ? Math.round(v * 255) : v).join(',')})`
                      : 'rgba(0,0,0,0)',
                  }}
                />
              </div>
              <div className="text-[9px] text-[#aeaeb2] mt-1">
                Initial buffer state on Play. Gray-Scott: (R=1, G=0, B=0, A=0)
              </div>
            </div>
          )}
        </div>
      ),
    });
  }

  // --- RENDERER CONFIG ---
  if (data.type === 'renderer') {
    sections.push({
      id: 'renderer',
      title: 'RENDERER CONFIG',
      content: (
        <div className="px-4 py-3">
          <div className="text-[11px] text-[#86868b] mb-2">
            Size follows upstream output{data.resolvedWidth && data.resolvedHeight ? `: ${data.resolvedWidth} × ${data.resolvedHeight}` : ''}
          </div>
          <label className="flex items-center gap-2 text-[11px] text-[#1d1d1f]">
            <input type="checkbox" checked={data.expanded !== false}
              onChange={(e) => updateNodeData(selectedNodeId!, { expanded: e.target.checked })} />
            In-place preview
          </label>
        </div>
      ),
    });
  }

  // --- FRAMEBUFFER CONFIG ---
  if (data.type === 'input' && data.inputMode === 'framebuffer') {
    sections.push({
      id: 'fbconfig',
      title: 'FRAMEBUFFER CONFIG',
      content: (
        <div className="px-4 py-3 overflow-y-auto">
          <div className="mb-2">
            <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">Format</label>
            <select value={data.fbFormat ?? 'rgba8'} onChange={(e) => updateNodeData(selectedNodeId!, { fbFormat: e.target.value as FramebufferFormat })}
              className="w-full text-[12px] text-[#1d1d1f] bg-[#f5f5f7] rounded px-2 py-1 border border-[#d2d2d7] outline-none focus:border-[#007aff]">
              {FB_FORMATS.map((f) => (<option key={f.value} value={f.value}>{f.label}</option>))}
            </select>
          </div>
          <div className="flex items-center gap-3 mb-2">
            <div className="flex-1">
              <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">Width</label>
              <input type="number" min={1} max={8192} value={String(data.fbWidth ?? '')}
                onChange={(e) => updateNodeData(selectedNodeId!, { fbWidth: parseInt(e.target.value) || undefined })}
                className="w-full text-[12px] text-[#1d1d1f] bg-[#f5f5f7] rounded px-2 py-1 border border-[#d2d2d7] outline-none focus:border-[#007aff]" placeholder="required" />
            </div>
            <div className="flex-1">
              <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">Height</label>
              <input type="number" min={1} max={8192} value={String(data.fbHeight ?? '')}
                onChange={(e) => updateNodeData(selectedNodeId!, { fbHeight: parseInt(e.target.value) || undefined })}
                className="w-full text-[12px] text-[#1d1d1f] bg-[#f5f5f7] rounded px-2 py-1 border border-[#d2d2d7] outline-none focus:border-[#007aff]" placeholder="required" />
            </div>
          </div>
          <div>
            <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">Stride (bytes per row)</label>
            <input type="number" min={0} value={String(data.fbStride ?? '')}
              onChange={(e) => updateNodeData(selectedNodeId!, { fbStride: parseInt(e.target.value) || undefined })}
              className="w-full text-[12px] text-[#1d1d1f] bg-[#f5f5f7] rounded px-2 py-1 border border-[#d2d2d7] outline-none focus:border-[#007aff]" placeholder="auto" />
          </div>
        </div>
      ),
    });
  }

  // --- VIDEO CONFIG ---
  if (data.type === 'input' && data.inputMode === 'video') {
    sections.push({
      id: 'videoconfig',
      title: 'VIDEO CONFIG',
      content: (
        <div className="px-4 py-3">
          <div className="mb-2">
            <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">Source</label>
            <select value={data.videoSourceType ?? 'file'} onChange={(e) => updateNodeData(selectedNodeId!, { videoSourceType: e.target.value as 'camera' | 'file' })}
              className="w-full text-[12px] text-[#1d1d1f] bg-[#f5f5f7] rounded px-2 py-1 border border-[#d2d2d7] outline-none focus:border-[#007aff]">
              <option value="file">FILE</option>
              <option value="camera">CAMERA</option>
            </select>
          </div>
          {(data.imageWidth || data.resolvedWidth) && (data.imageHeight || data.resolvedHeight) && (
            <div className="text-[11px] text-[#86868b] mb-2">
              {(data.imageWidth ?? data.resolvedWidth)} × {(data.imageHeight ?? data.resolvedHeight)}
            </div>
          )}
          <div className="flex items-center gap-3">
            <label className="flex items-center gap-2 text-[11px] text-[#1d1d1f]">
              <input type="checkbox" checked={data.videoLoop ?? true}
                onChange={(e) => updateNodeData(selectedNodeId!, { videoLoop: e.target.checked })} />
              Loop
            </label>
            <div className="flex-1">
              <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">Rate</label>
              <input type="number" step="0.1" min="0.1" value={String(data.videoPlaybackRate ?? 1)}
                onChange={(e) => updateNodeData(selectedNodeId!, { videoPlaybackRate: parseFloat(e.target.value) || 1 })}
                className="w-full text-[12px] text-[#1d1d1f] bg-[#f5f5f7] rounded px-2 py-1 border border-[#d2d2d7] outline-none focus:border-[#007aff]" />
            </div>
          </div>
        </div>
      ),
    });
  }

  // --- SAMPLING (sampler2D inputs) ---
  if (data.type === 'input' && data.inputDataType === 'sampler2D') {
    sections.push({
      id: 'sampling',
      title: 'SAMPLING',
      content: (
        <div className="px-4 py-3">
          <div className="flex items-center gap-3">
            <div className="flex-1">
              <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">Filter</label>
              <select value={data.texFilter ?? 'linear'} onChange={(e) => updateNodeData(selectedNodeId!, { texFilter: e.target.value as TextureFilter })}
                className="w-full text-[12px] text-[#1d1d1f] bg-[#f5f5f7] rounded px-2 py-1 border border-[#d2d2d7] outline-none focus:border-[#007aff]">
                <option value="linear">LINEAR</option>
                <option value="nearest">NEAREST</option>
              </select>
            </div>
            <div className="flex-1">
              <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">Wrap</label>
              <select value={data.texWrap ?? 'clamp'} onChange={(e) => updateNodeData(selectedNodeId!, { texWrap: e.target.value as TextureWrap })}
                className="w-full text-[12px] text-[#1d1d1f] bg-[#f5f5f7] rounded px-2 py-1 border border-[#d2d2d7] outline-none focus:border-[#007aff]">
                <option value="clamp">CLAMP</option>
                <option value="repeat">REPEAT</option>
                <option value="mirror">MIRROR</option>
              </select>
            </div>
          </div>
        </div>
      ),
    });
  }

  // --- PREVIEW ---
  const hasPreview = (data.type === 'shader' || data.type === 'constant') ||
    (data.type === 'renderer' && data.expanded === false) ||
    isSampler2D;

  if (hasPreview) {
    const previewExtra = data.type === 'renderer' ? (
      <button
        onClick={(e) => {
          e.stopPropagation();
          if (!selectedNodeId) return;
          setLightboxSrc(`renderer:${selectedNodeId}`);
          requestAnimationFrame(() => window.dispatchEvent(new CustomEvent('renderer-remount')));
        }}
        className="text-[10px] text-[#007aff] hover:text-[#0066d6] font-medium cursor-default"
      >
        FULLSCREEN
      </button>
    ) : undefined;

    sections.push({
      id: 'preview',
      title: 'PREVIEW',
      flexFill: true,
      extra: previewExtra,
      content: (
        <div className="flex-1 flex items-center justify-center bg-[#f5f5f7] overflow-hidden p-2">
          {data.type === 'renderer' && data.expanded === false ? (
            <canvas
              id={`renderer-mirror-${selectedNodeId}`}
              className="rounded border border-[#e8e8ed] bg-[#1d1d1f]"
              width={data.resolvedWidth ?? 512}
              height={data.resolvedHeight ?? 512}
              style={{
                width: '100%',
                maxHeight: '100%',
                aspectRatio: `${data.resolvedWidth ?? 16} / ${data.resolvedHeight ?? 9}`,
              }}
            />
          ) : isSampler2D ? (
            inputPreviewSrc ? (
              data.inputMode === 'video' ? (
                <video src={`${inputPreviewSrc}#t=0.1`} muted playsInline preload="metadata"
                  onClick={() => setLightboxSrc(inputPreviewSrc)}
                  className="max-w-full max-h-full object-contain rounded border border-[#d2d2d7] cursor-pointer hover:opacity-90 transition-opacity" />
              ) : (
                <img src={inputPreviewSrc} alt="preview"
                  onClick={() => setLightboxSrc(inputPreviewSrc)}
                  className="max-w-full max-h-full object-contain rounded border border-[#d2d2d7] cursor-pointer hover:opacity-90 transition-opacity" />
              )
            ) : (
              <span className="text-[12px] text-[#aeaeb2]">
                {isFramebuffer ? 'Load file and set width/height' : data.inputMode === 'video' ? 'Load a video' : 'Load an image'}
              </span>
            )
          ) : outputPreviews[selectedNodeId!] ? (
            <img src={outputPreviews[selectedNodeId!]} alt="output"
              onClick={() => setLightboxSrc(outputPreviews[selectedNodeId!])}
              className="max-w-full max-h-full object-contain rounded border border-[#d2d2d7] cursor-pointer hover:opacity-90 transition-opacity" />
          ) : (
            <span className="text-[12px] text-[#aeaeb2]">Press Play to preview</span>
          )}
        </div>
      ),
    });
  }

  return (
    <aside className="bg-white border-l border-[#d2d2d7] flex-shrink-0 flex flex-col overflow-hidden relative" style={{ width: panelWidth }}>
      {/* Resize handle */}
      <div
        onMouseDown={handleResizeStart}
        className="absolute left-0 top-0 bottom-0 w-1 cursor-col-resize hover:bg-[#007aff]/30 z-10"
      />
      {/* Node header — always visible */}
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

      {/* Error — always visible when present */}
      {nodeError && (
        <div className="px-4 py-2 bg-[#fff0f0] border-b border-[#ffd0d0]">
          <div className="text-[10px] text-[#ff3b30] font-medium mb-0.5">Error</div>
          <div className="text-[11px] text-[#1d1d1f] font-mono whitespace-pre-wrap break-words leading-snug max-h-24 overflow-y-auto">{nodeError}</div>
        </div>
      )}

      {/* Accordion sections */}
      {sections.map((section) => {
        const isExpanded = !collapsedSections.has(section.id);
        return (
          <div
            key={section.id}
            className={`flex flex-col min-h-0 ${isExpanded && section.flexFill ? 'flex-1' : ''}`}
          >
            <SectionHeader
              title={section.title}
              expanded={isExpanded}
              onClick={() => toggleSection(section.id)}
              extra={section.extra}
            />
            {isExpanded && (
              <div className={`overflow-hidden ${section.flexFill ? 'flex-1 flex flex-col min-h-0' : ''}`}>
                {section.content}
              </div>
            )}
          </div>
        );
      })}

      {/* Image dimensions badge (always visible, not a section) */}
      {data.type === 'input' && data.inputDataType === 'sampler2D' && data.inputMode !== 'framebuffer' && data.inputMode !== 'video' && data.imageWidth && data.imageHeight && (
        <div className="px-4 py-1 border-t border-[#e8e8ed] flex-shrink-0">
          <span className="text-[11px] text-[#86868b]">{data.imageWidth} × {data.imageHeight}</span>
        </div>
      )}

      {/* Lightbox overlays */}
      {lightboxSrc && !lightboxSrc.startsWith('renderer:') && (
        <ImageLightbox src={lightboxSrc} onClose={() => setLightboxSrc(null)} />
      )}
      {lightboxSrc?.startsWith('renderer:') && (() => {
        const rid = lightboxSrc.slice('renderer:'.length);
        const rNode = nodes.find((n) => n.id === rid);
        const rw = rNode?.data.resolvedWidth ?? 16;
        const rh = rNode?.data.resolvedHeight ?? 9;
        return (
          <div className="fixed inset-0 z-50 bg-black/90 flex flex-col items-center justify-center"
            onClick={() => setLightboxSrc(null)}>
            <div className="absolute top-4 right-4 flex items-center gap-3 z-10">
              <button onClick={(e) => { e.stopPropagation(); const capture = useGraphStore.getState().captureScreenshot; const dataUrl = capture?.(rid); if (!dataUrl) return; const a = document.createElement('a'); a.href = dataUrl; a.download = `renderer-${rid}.png`; a.click(); }}
                className="text-[11px] text-white/80 hover:text-white font-medium px-3 py-1 rounded bg-white/10 hover:bg-white/20">SAVE</button>
              <button onClick={(e) => { e.stopPropagation(); setLightboxSrc(null); }}
                className="text-[11px] text-white/80 hover:text-white font-medium px-3 py-1 rounded bg-white/10 hover:bg-white/20">CLOSE</button>
            </div>
            <canvas id={`renderer-mirror-fullscreen-${rid}`} className="rounded overflow-hidden"
              width={rw} height={rh}
              style={{ width: '90vw', maxHeight: '85vh', aspectRatio: `${rw} / ${rh}` }}
              onClick={(e) => e.stopPropagation()} />
          </div>
        );
      })()}
    </aside>
  );
}
