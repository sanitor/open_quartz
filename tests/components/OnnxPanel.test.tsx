import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import type { OnnxDetection } from '../../src/engine/onnxSession';

interface StoreState {
  updateNodeData: ReturnType<typeof vi.fn>;
  outputData: Record<string, unknown>;
}

const mockUpdateNodeData = vi.fn();
const storeState: StoreState = {
  updateNodeData: mockUpdateNodeData,
  outputData: {},
};

vi.mock('../../src/store/useGraphStore', () => ({
  useGraphStore: vi.fn((selector: unknown) => {
    if (typeof selector === 'function') {
      return (selector as (s: StoreState) => unknown)(storeState);
    }
    return {};
  }),
}));

import { OnnxPanel } from '../../src/components/SidePanel/OnnxPanel';

function setOutputData(nodeId: string, value: unknown): void {
  storeState.outputData = { [nodeId]: value };
}

function makeDet(overrides: Partial<OnnxDetection> = {}): OnnxDetection {
  return {
    bbox: [0.1, 0.2, 0.3, 0.4],
    score: 0.75,
    class_id: 0,
    class_name: 'person',
    ...overrides,
  };
}

describe('OnnxPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    storeState.outputData = {};
  });

  it('renders a red banner for an unknown modelId', () => {
    render(<OnnxPanel nodeId="onnx_1" modelId="not-real" />);
    const banner = screen.getByText(/Unknown ONNX model/);
    expect(banner).toBeInTheDocument();
    expect(banner.textContent).toContain('not-real');
    expect(banner).toHaveClass('text-[#ff3b30]');
  });

  it("shows the descriptor label, modelUrl, and input size for a known model", () => {
    render(<OnnxPanel nodeId="onnx_1" modelId="yolov8n" />);
    expect(screen.getByText('YOLOv8n Detector')).toBeInTheDocument();
    expect(screen.getByText('/models/yolov8n.onnx')).toBeInTheDocument();
    expect(screen.getByText(/640×640/)).toBeInTheDocument();
  });

  it('displays the descriptor default thresholds when no score/iou props supplied', () => {
    render(<OnnxPanel nodeId="onnx_1" modelId="yolov8n" />);
    const inputs = screen.getAllByRole('spinbutton');
    // Score input first, IoU second — matches DOM order in the source.
    expect(inputs).toHaveLength(2);
    expect((inputs[0] as HTMLInputElement).value).toBe('0.25');
    expect((inputs[1] as HTMLInputElement).value).toBe('0.45');
  });

  it('prefers explicit score/iou props over descriptor defaults', () => {
    render(<OnnxPanel nodeId="onnx_1" modelId="yolov8n" score={0.6} iou={0.3} />);
    const inputs = screen.getAllByRole('spinbutton');
    expect((inputs[0] as HTMLInputElement).value).toBe('0.6');
    expect((inputs[1] as HTMLInputElement).value).toBe('0.3');
  });

  it('score onBlur → updateNodeData(nodeId, { onnxScoreThreshold: value }) for an in-range value', () => {
    render(<OnnxPanel nodeId="onnx_1" modelId="yolov8n" />);
    const [scoreInput] = screen.getAllByRole('spinbutton');
    fireEvent.change(scoreInput, { target: { value: '0.5' } });
    fireEvent.blur(scoreInput);
    expect(mockUpdateNodeData).toHaveBeenCalledWith('onnx_1', { onnxScoreThreshold: 0.5 });
  });

  it('score onBlur clamps values > 1 down to 1', () => {
    render(<OnnxPanel nodeId="onnx_1" modelId="yolov8n" />);
    const [scoreInput] = screen.getAllByRole('spinbutton');
    fireEvent.change(scoreInput, { target: { value: '1.5' } });
    fireEvent.blur(scoreInput);
    expect(mockUpdateNodeData).toHaveBeenCalledWith('onnx_1', { onnxScoreThreshold: 1 });
  });

  it('score onBlur clamps values < 0 up to 0', () => {
    render(<OnnxPanel nodeId="onnx_1" modelId="yolov8n" />);
    const [scoreInput] = screen.getAllByRole('spinbutton');
    fireEvent.change(scoreInput, { target: { value: '-0.2' } });
    fireEvent.blur(scoreInput);
    expect(mockUpdateNodeData).toHaveBeenCalledWith('onnx_1', { onnxScoreThreshold: 0 });
  });

  it('score onBlur ignores non-numeric input (no updateNodeData call)', () => {
    render(<OnnxPanel nodeId="onnx_1" modelId="yolov8n" />);
    const [scoreInput] = screen.getAllByRole('spinbutton');
    fireEvent.change(scoreInput, { target: { value: 'not-a-number' } });
    fireEvent.blur(scoreInput);
    expect(mockUpdateNodeData).not.toHaveBeenCalled();
  });

  it('iou onBlur updates with parsed + clamped value', () => {
    render(<OnnxPanel nodeId="onnx_1" modelId="yolov8n" />);
    const [, iouInput] = screen.getAllByRole('spinbutton');
    fireEvent.change(iouInput, { target: { value: '0.7' } });
    fireEvent.blur(iouInput);
    expect(mockUpdateNodeData).toHaveBeenCalledWith('onnx_1', { onnxIouThreshold: 0.7 });
  });

  it('iou onBlur clamps > 1 → 1', () => {
    render(<OnnxPanel nodeId="onnx_1" modelId="yolov8n" />);
    const [, iouInput] = screen.getAllByRole('spinbutton');
    fireEvent.change(iouInput, { target: { value: '2.5' } });
    fireEvent.blur(iouInput);
    expect(mockUpdateNodeData).toHaveBeenCalledWith('onnx_1', { onnxIouThreshold: 1 });
  });

  it('iou onBlur clamps < 0 → 0', () => {
    render(<OnnxPanel nodeId="onnx_1" modelId="yolov8n" />);
    const [, iouInput] = screen.getAllByRole('spinbutton');
    fireEvent.change(iouInput, { target: { value: '-3' } });
    fireEvent.blur(iouInput);
    expect(mockUpdateNodeData).toHaveBeenCalledWith('onnx_1', { onnxIouThreshold: 0 });
  });

  it('iou onBlur ignores non-numeric input', () => {
    render(<OnnxPanel nodeId="onnx_1" modelId="yolov8n" />);
    const [, iouInput] = screen.getAllByRole('spinbutton');
    fireEvent.change(iouInput, { target: { value: 'xyz' } });
    fireEvent.blur(iouInput);
    expect(mockUpdateNodeData).not.toHaveBeenCalled();
  });

  it("renders 'Press Run to detect' when there is no outputData for the node", () => {
    render(<OnnxPanel nodeId="onnx_1" modelId="yolov8n" />);
    expect(screen.getByText('Press Run to detect')).toBeInTheDocument();
  });

  it("shows 'Press Run to detect' when outputData is null / undefined / empty object", () => {
    setOutputData('onnx_1', null);
    const { rerender } = render(<OnnxPanel nodeId="onnx_1" modelId="yolov8n" />);
    expect(screen.getByText('Press Run to detect')).toBeInTheDocument();

    setOutputData('onnx_1', undefined);
    rerender(<OnnxPanel nodeId="onnx_1" modelId="yolov8n" />);
    expect(screen.getByText('Press Run to detect')).toBeInTheDocument();

    setOutputData('onnx_1', {});
    rerender(<OnnxPanel nodeId="onnx_1" modelId="yolov8n" />);
    expect(screen.getByText('Press Run to detect')).toBeInTheDocument();
  });

  it("shows 'Press Run to detect' when detections is not an array", () => {
    setOutputData('onnx_1', { detections: 'not-an-array' });
    render(<OnnxPanel nodeId="onnx_1" modelId="yolov8n" />);
    expect(screen.getByText('Press Run to detect')).toBeInTheDocument();
  });

  it('renders one table row per detection with class_name, score%, and bbox joined string', () => {
    const dets: OnnxDetection[] = [
      makeDet({ class_name: 'person', score: 0.87, class_id: 0, bbox: [0.11, 0.22, 0.33, 0.44] }),
      makeDet({ class_name: 'dog', score: 0.61, class_id: 16, bbox: [0.5, 0.5, 0.9, 0.9] }),
    ];
    setOutputData('onnx_1', { detections: dets });
    render(<OnnxPanel nodeId="onnx_1" modelId="yolov8n" />);

    expect(screen.getByText('person')).toBeInTheDocument();
    expect(screen.getByText('dog')).toBeInTheDocument();
    // Scores are formatted with one decimal + %.
    expect(screen.getByText('87.0%')).toBeInTheDocument();
    expect(screen.getByText('61.0%')).toBeInTheDocument();
    // Bbox is joined with ", " and each element rendered with .toFixed(2).
    expect(screen.getByText('0.11, 0.22, 0.33, 0.44')).toBeInTheDocument();
    expect(screen.getByText('0.50, 0.50, 0.90, 0.90')).toBeInTheDocument();
  });

  it('renders the detection count in the DETECTIONS header', () => {
    setOutputData('onnx_1', {
      detections: [makeDet(), makeDet(), makeDet()],
    });
    render(<OnnxPanel nodeId="onnx_1" modelId="yolov8n" />);
    expect(screen.getByText(/DETECTIONS \(3\)/)).toBeInTheDocument();
  });

  it('does not crash when outputData shape is an array or primitive', () => {
    setOutputData('onnx_1', [1, 2, 3]);
    const { rerender } = render(<OnnxPanel nodeId="onnx_1" modelId="yolov8n" />);
    expect(screen.getByText('Press Run to detect')).toBeInTheDocument();

    setOutputData('onnx_1', 42);
    rerender(<OnnxPanel nodeId="onnx_1" modelId="yolov8n" />);
    expect(screen.getByText('Press Run to detect')).toBeInTheDocument();

    setOutputData('onnx_1', 'string');
    rerender(<OnnxPanel nodeId="onnx_1" modelId="yolov8n" />);
    expect(screen.getByText('Press Run to detect')).toBeInTheDocument();
  });
});
