import { useMemo, useState, useRef, useCallback } from 'react';
import { useGraphStore } from '../../store/useGraphStore';
import { ONNX_MODELS } from '../../engine/onnxRegistry';
import { ONNX_CATALOG } from '../../engine/onnxCatalog';
import type { CatalogEntry } from '../../engine/onnxCatalog';
import type { OnnxDetection } from '../../engine/onnxSession';
import type { OnnxModelDescriptor } from '../../engine/onnxRegistry';

interface OnnxDetectionsPayload {
  detections: OnnxDetection[];
}

interface SegmentationPayload {
  segmentation: {
    classCounts: number[];
    numClasses: number;
    maskW: number;
    maskH: number;
  };
}

function isDetectionsPayload(v: unknown): v is OnnxDetectionsPayload {
  if (!v || typeof v !== 'object') return false;
  if (!('detections' in v)) return false;
  return Array.isArray(v.detections);
}

function isSegmentationPayload(v: unknown): v is SegmentationPayload {
  if (!v || typeof v !== 'object') return false;
  if (!('segmentation' in v)) return false;
  const seg = v.segmentation;
  if (!seg || typeof seg !== 'object') return false;
  return 'classCounts' in seg && Array.isArray(seg.classCounts);
}

const CITYSCAPES_CLASSES = [
  'road', 'sidewalk', 'building', 'wall', 'fence',
  'pole', 'traffic light', 'traffic sign', 'vegetation', 'terrain',
  'sky', 'person', 'rider', 'car', 'truck',
  'bus', 'train', 'motorcycle', 'bicycle',
];

const CITYSCAPES_COLORS = [
  '#804080', '#f423e8', '#464646', '#666e96', '#be9999',
  '#999999', '#faaa1e', '#dcdc00', '#6b8e23', '#98fb98',
  '#4682b4', '#dc143c', '#ff0000', '#00008e', '#000046',
  '#003c64', '#005064', '#0000e6', '#770b20',
];

const STATUS_COLORS: Record<string, string> = {
  'not-downloaded': '#86868b',
  'downloading': '#007aff',
  'downloaded': '#34c759',
  'introspecting': '#ff9f0a',
  'ready': '#34c759',
  'error': '#ff3b30',
};

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

interface OnnxPanelProps {
  nodeId: string;
  modelId?: string;
  source?: 'catalog' | 'custom';
  status?: string;
  backend?: 'webgpu' | 'wasm';
  score?: number;
  iou?: number;
}

export function OnnxPanel({ nodeId, modelId, source, status, backend, score, iou }: OnnxPanelProps) {
  const updateNodeData = useGraphStore((s) => s.updateNodeData);
  const loadCustomOnnxModel = useGraphStore((s) => s.loadCustomOnnxModel);
  const outputData = useGraphStore((s) => s.outputData[nodeId]);
  const nodeData = useGraphStore((s) => {
    const node = s.nodes?.find((n) => n.id === nodeId);
    return node?.data;
  });
  const fileInputRef = useRef<HTMLInputElement>(null);

  const handleCustomFileChange = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = () => {
      const buffer = reader.result as ArrayBuffer;
      loadCustomOnnxModel(nodeId, buffer, file.name);
    };
    reader.readAsArrayBuffer(file);
    e.target.value = '';
  }, [nodeId, loadCustomOnnxModel]);

  // Resolve descriptor: catalog first, then legacy registry
  const catalogEntry: CatalogEntry | undefined = modelId ? ONNX_CATALOG[modelId] : undefined;
  const legacyDescriptor: OnnxModelDescriptor | undefined = modelId ? ONNX_MODELS[modelId] : undefined;

  // Derive default thresholds from catalog params or legacy descriptor
  const defaultScore = catalogEntry?.defaultParams?.scoreThreshold
    ? Number(catalogEntry.defaultParams.scoreThreshold.default)
    : legacyDescriptor?.scoreThreshold ?? 0.25;
  const defaultIou = catalogEntry?.defaultParams?.iouThreshold
    ? Number(catalogEntry.defaultParams.iouThreshold.default)
    : legacyDescriptor?.iouThreshold ?? 0.45;

  const [scoreDraft, setScoreDraft] = useState<string>(String(score ?? defaultScore));
  const [iouDraft, setIouDraft] = useState<string>(String(iou ?? defaultIou));

  const detections = useMemo<OnnxDetection[]>(() => {
    return isDetectionsPayload(outputData) ? outputData.detections : [];
  }, [outputData]);

  const segData = useMemo(() => {
    return isSegmentationPayload(outputData) ? outputData.segmentation : null;
  }, [outputData]);

  const isSegmentation = catalogEntry?.task === 'segmentation';

  const commitScore = (raw: string) => {
    const n = parseFloat(raw);
    if (!Number.isFinite(n)) return;
    updateNodeData(nodeId, { onnxScoreThreshold: Math.max(0, Math.min(1, n)) });
  };
  const commitIou = (raw: string) => {
    const n = parseFloat(raw);
    if (!Number.isFinite(n)) return;
    updateNodeData(nodeId, { onnxIouThreshold: Math.max(0, Math.min(1, n)) });
  };

  const effectiveSource = source ?? (catalogEntry ? 'catalog' : legacyDescriptor ? 'catalog' : 'custom');
  const label = catalogEntry?.label ?? legacyDescriptor?.label ?? modelId ?? 'Unknown Model';

  return (
    <div className="flex flex-col min-h-0 border-t border-[#e8e8ed]">
      {/* Header + Status */}
      <div className="px-4 py-3 flex-shrink-0">
        <div className="text-[10px] text-[#86868b] font-medium mb-2">ONNX CONFIG</div>
        <div className="text-[11px] text-[#1d1d1f] mb-1">{label}</div>

        {/* Status badge */}
        {status && (
          <div className="mb-2">
            <span
              className="inline-block text-[10px] font-medium px-1.5 py-0.5 rounded"
              style={{
                color: STATUS_COLORS[status] ?? '#86868b',
                backgroundColor: `${STATUS_COLORS[status] ?? '#86868b'}18`,
              }}
            >
              {status}
            </span>
            {backend === 'wasm' && (
              <span
                className="inline-block text-[10px] font-medium px-1.5 py-0.5 rounded ml-1"
                style={{ color: '#ff9f0a', backgroundColor: '#ff9f0a18' }}
                title="WebGPU not supported for this model on your GPU — running on CPU via WASM"
              >
                CPU fallback
              </span>
            )}
          </div>
        )}

        {/* Catalog model info */}
        {effectiveSource === 'catalog' && catalogEntry && (
          <div className="text-[10px] text-[#86868b] mb-3">
            <div>Category: <span className="text-[#1d1d1f]">{catalogEntry.category}</span></div>
            <div>Task: <span className="text-[#1d1d1f]">{catalogEntry.task}</span></div>
            <div>Size: <span className="text-[#1d1d1f]">{formatBytes(catalogEntry.fileSize)}</span></div>
            <div>
              URL: <span className="font-mono text-[9px] break-all">{catalogEntry.downloadUrl}</span>
            </div>
          </div>
        )}

        {/* Legacy descriptor fallback info */}
        {effectiveSource === 'catalog' && !catalogEntry && legacyDescriptor && (
          <div className="text-[10px] text-[#86868b] mb-3">
            <div>Model: <span className="font-mono">{legacyDescriptor.modelUrl}</span></div>
            <div>Input size: {legacyDescriptor.targetSize}×{legacyDescriptor.targetSize}</div>
          </div>
        )}

        {/* Custom model info */}
        {effectiveSource === 'custom' && (
          <div className="text-[10px] text-[#86868b] mb-3">
            {nodeData?.onnxCustomFileName && (
              <div>File: <span className="text-[#1d1d1f] font-mono">{nodeData.onnxCustomFileName}</span></div>
            )}
            {nodeData?.onnxCustomPath && (
              <div>Path: <span className="font-mono text-[9px] break-all">{nodeData.onnxCustomPath}</span></div>
            )}
            {!nodeData?.onnxCustomFileName && !nodeData?.onnxCustomPath && (
              <div className="text-[#aeaeb2] italic">No model file selected</div>
            )}
            <input ref={fileInputRef} type="file" accept=".onnx" onChange={handleCustomFileChange} className="hidden" />
            <button
              onClick={() => fileInputRef.current?.click()}
              className="mt-2 w-full text-[11px] text-[#007aff] bg-[#f5f5f7] rounded px-3 py-1.5 border border-[#d2d2d7] cursor-default hover:bg-[#e8e8ed] transition-colors"
            >
              Select Model File…
            </button>
          </div>
        )}

        {/* Catalog params (score/iou thresholds) — detection only */}
        {!isSegmentation && (catalogEntry?.defaultParams || legacyDescriptor) && (
          <div className="flex items-center gap-3 mb-2">
            <div className="flex-1">
              <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">Score ≥</label>
              <input
                type="number"
                step={0.05}
                min={0}
                max={1}
                value={scoreDraft}
                onChange={(e) => setScoreDraft(e.target.value)}
                onBlur={(e) => commitScore(e.target.value)}
                className="w-full text-[12px] text-[#1d1d1f] bg-[#f5f5f7] rounded px-2 py-1 border border-[#d2d2d7] outline-none focus:border-[#007aff]"
              />
            </div>
            <div className="flex-1">
              <label className="block text-[10px] text-[#86868b] font-medium mb-0.5">IoU ≤</label>
              <input
                type="number"
                step={0.05}
                min={0}
                max={1}
                value={iouDraft}
                onChange={(e) => setIouDraft(e.target.value)}
                onBlur={(e) => commitIou(e.target.value)}
                className="w-full text-[12px] text-[#1d1d1f] bg-[#f5f5f7] rounded px-2 py-1 border border-[#d2d2d7] outline-none focus:border-[#007aff]"
              />
            </div>
          </div>
        )}
      </div>

      {/* Segmentation class distribution */}
      {isSegmentation && (
        <div className="px-4 pb-3 flex-shrink-0">
          <div className="text-[10px] text-[#86868b] font-medium mb-1">
            CLASS DISTRIBUTION
          </div>
          <div className="max-h-40 overflow-y-auto rounded border border-[#e8e8ed] bg-[#fafafa]">
            {!segData ? (
              <div className="px-3 py-2 text-[11px] text-[#aeaeb2] italic">Press Run to segment</div>
            ) : (
              <table className="w-full text-[10px] text-[#1d1d1f]">
                <thead>
                  <tr className="text-[9px] text-[#86868b] border-b border-[#e8e8ed]">
                    <th className="text-left px-2 py-1">Class</th>
                    <th className="text-right px-2 py-1">%</th>
                  </tr>
                </thead>
                <tbody>
                  {segData.classCounts.map((count, i) => {
                    if (count === 0) return null;
                    const total = segData.maskW * segData.maskH;
                    return (
                      <tr key={i} className="odd:bg-white">
                        <td className="px-2 py-1 flex items-center gap-1.5">
                          <span
                            className="inline-block w-2.5 h-2.5 rounded-sm flex-shrink-0"
                            style={{ backgroundColor: CITYSCAPES_COLORS[i] ?? '#888' }}
                          />
                          {CITYSCAPES_CLASSES[i] ?? `class_${i}`}
                        </td>
                        <td className="text-right px-2 py-1 font-mono">
                          {(count / total * 100).toFixed(1)}%
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            )}
          </div>
        </div>
      )}

      {/* Detections list — detection only */}
      {!isSegmentation && (
      <div className="px-4 pb-3 flex-shrink-0">
        <div className="text-[10px] text-[#86868b] font-medium mb-1">
          DETECTIONS ({detections.length})
        </div>
        <div className="max-h-40 overflow-y-auto rounded border border-[#e8e8ed] bg-[#fafafa]">
          {detections.length === 0 ? (
            <div className="px-3 py-2 text-[11px] text-[#aeaeb2] italic">Press Run to detect</div>
          ) : (
            <table className="w-full text-[10px] text-[#1d1d1f]">
              <thead>
                <tr className="text-[9px] text-[#86868b] border-b border-[#e8e8ed]">
                  <th className="text-left px-2 py-1">Class</th>
                  <th className="text-right px-2 py-1">Score</th>
                  <th className="text-right px-2 py-1">Box</th>
                </tr>
              </thead>
              <tbody>
                {detections.map((d, i) => (
                  <tr key={`${d.class_id}_${i}`} className="odd:bg-white">
                    <td className="px-2 py-1">{d.class_name}</td>
                    <td className="text-right px-2 py-1 font-mono">
                      {(d.score * 100).toFixed(1)}%
                    </td>
                    <td className="text-right px-2 py-1 font-mono text-[9px]">
                      {d.bbox.map((v: number) => v.toFixed(2)).join(', ')}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      </div>
      )}
    </div>
  );
}
