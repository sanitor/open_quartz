import { useMemo, useState } from 'react';
import { useGraphStore } from '../../store/useGraphStore';
import { ONNX_MODELS } from '../../engine/onnxRegistry';
import type { OnnxDetection } from '../../engine/onnxSession';

interface OnnxDetectionsPayload {
  detections: OnnxDetection[];
}

function isDetectionsPayload(v: unknown): v is OnnxDetectionsPayload {
  if (!v || typeof v !== 'object') return false;
  if (!('detections' in v)) return false;
  const dets = (v as { detections: unknown }).detections;
  return Array.isArray(dets);
}

interface OnnxPanelProps {
  nodeId: string;
  modelId: string;
  score?: number;
  iou?: number;
}

export function OnnxPanel({ nodeId, modelId, score, iou }: OnnxPanelProps) {
  const updateNodeData = useGraphStore((s) => s.updateNodeData);
  const outputData = useGraphStore((s) => s.outputData[nodeId]);
  const descriptor = ONNX_MODELS[modelId];
  const [scoreDraft, setScoreDraft] = useState<string>(String(score ?? descriptor?.scoreThreshold ?? 0.25));
  const [iouDraft, setIouDraft] = useState<string>(String(iou ?? descriptor?.iouThreshold ?? 0.45));

  const detections = useMemo<OnnxDetection[]>(() => {
    return isDetectionsPayload(outputData) ? outputData.detections : [];
  }, [outputData]);

  if (!descriptor) {
    return (
      <div className="px-4 py-3 border-t border-[#e8e8ed] text-[11px] text-[#ff3b30]">
        Unknown ONNX model: {modelId}
      </div>
    );
  }

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

  return (
    <div className="flex flex-col min-h-0 border-t border-[#e8e8ed]">
      <div className="px-4 py-3 flex-shrink-0">
        <div className="text-[10px] text-[#86868b] font-medium mb-2">ONNX CONFIG</div>
        <div className="text-[11px] text-[#1d1d1f] mb-1">{descriptor.label}</div>
        <div className="text-[10px] text-[#86868b] mb-3">
          <div>Model: <span className="font-mono">{descriptor.modelUrl}</span></div>
          <div>Input size: {descriptor.targetSize}×{descriptor.targetSize}</div>
        </div>

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
      </div>

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
    </div>
  );
}
