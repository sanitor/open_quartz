import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

vi.mock('@xyflow/react', () => ({
  Handle: ({ type, id }: { type: string; position: string; id: string }) => (
    <span data-testid={`handle-${type}-${id}`} />
  ),
  Position: { Left: 'left', Right: 'right', Top: 'top', Bottom: 'bottom' },
}));

interface StoreState {
  edges: Array<{ targetHandle: string }>;
  nodeErrors: Record<string, string>;
  outputPreviews: Record<string, string>;
}

const defaultStoreState: StoreState = {
  edges: [],
  nodeErrors: {},
  outputPreviews: {},
};

vi.mock('../../src/store/useGraphStore', () => ({
  useGraphStore: vi.fn((selector: unknown) => {
    if (typeof selector === 'function') {
      return (selector as (s: StoreState) => unknown)(defaultStoreState);
    }
    return {};
  }),
}));

import { OnnxNode } from '../../src/components/NodeGraph/nodes/OnnxNode';
import type { ShaderNodeData, Port } from '../../src/types';

function port(overrides: Partial<Port>): Port {
  return {
    id: 'p',
    label: 'label',
    dataType: 'sampler2D',
    direction: 'input',
    ...overrides,
  };
}

function makeOnnxProps(
  dataOverrides: Partial<ShaderNodeData> = {},
  storeOverrides: Partial<StoreState> = {},
) {
  Object.assign(defaultStoreState, {
    edges: [],
    nodeErrors: {},
    outputPreviews: {},
    ...storeOverrides,
  });

  const data: ShaderNodeData = {
    type: 'onnx',
    label: 'yolov8n',
    shaderCode: '',
    inputs: [port({ id: 'onnx_1_image', label: 'image', dataType: 'sampler2D', direction: 'input' })],
    outputs: [
      port({ id: 'onnx_1_det', label: 'detections', dataType: 'roi', direction: 'output' }),
      port({ id: 'onnx_1_ov', label: 'overlay', dataType: 'sampler2D', direction: 'output' }),
    ],
    uniforms: {},
    onnxModelId: 'yolov8n',
    ...dataOverrides,
  };

  return {
    id: 'onnx-1',
    data,
    selected: false,
    type: 'onnx' as const,
    isConnectable: true,
    zIndex: 0,
    positionAbsoluteX: 0,
    positionAbsoluteY: 0,
    dragging: false,
    dragHandle: undefined,
    selectable: true,
    deletable: true,
    parentId: undefined,
    sourcePosition: undefined,
    targetPosition: undefined,
    width: undefined,
    height: undefined,
  };
}

describe('OnnxNode', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    Object.assign(defaultStoreState, { edges: [], nodeErrors: {}, outputPreviews: {} });
  });

  it("renders the 'ONNX' header badge", () => {
    render(<OnnxNode {...makeOnnxProps()} />);
    expect(screen.getByText('ONNX')).toBeInTheDocument();
  });

  it('renders the label from data.label', () => {
    render(<OnnxNode {...makeOnnxProps({ label: 'my-model' })} />);
    expect(screen.getByText('my-model')).toBeInTheDocument();
  });

  it('renders one target handle per input port', () => {
    render(<OnnxNode {...makeOnnxProps()} />);
    expect(screen.getByTestId('handle-target-onnx_1_image')).toBeInTheDocument();
  });

  it('renders one source handle per output port', () => {
    render(<OnnxNode {...makeOnnxProps()} />);
    expect(screen.getByTestId('handle-source-onnx_1_det')).toBeInTheDocument();
    expect(screen.getByTestId('handle-source-onnx_1_ov')).toBeInTheDocument();
  });

  it("renders both output rows with their dataType labels (roi + sampler2D)", () => {
    render(<OnnxNode {...makeOnnxProps()} />);
    expect(screen.getByText('detections')).toBeInTheDocument();
    expect(screen.getByText('overlay')).toBeInTheDocument();
    expect(screen.getByText('roi')).toBeInTheDocument();
    // sampler2D appears twice (input + output) — getAllByText proves both.
    const samplers = screen.getAllByText('sampler2D');
    expect(samplers.length).toBeGreaterThanOrEqual(2);
  });

  it('renders the input port label with its dataType', () => {
    render(<OnnxNode {...makeOnnxProps()} />);
    expect(screen.getByText('image')).toBeInTheDocument();
  });

  it('paints the header with the ONNX accent when input is connected and no error', () => {
    // Accent constant in source: '#ff8a65' → rgb(255, 138, 101).
    const props = makeOnnxProps({}, { edges: [{ targetHandle: 'onnx_1_image' }] });
    const { container } = render(<OnnxNode {...props} />);
    const header = container.querySelector('[style*="background-color: rgb(255, 138, 101)"]');
    expect(header).toBeTruthy();
  });

  it('paints the header grey when the input is unconnected and no error', () => {
    // '#8e8e93' → rgb(142, 142, 147).
    const { container } = render(<OnnxNode {...makeOnnxProps()} />);
    const header = container.querySelector('[style*="background-color: rgb(142, 142, 147)"]');
    expect(header).toBeTruthy();
  });

  it('paints the header red when nodeErrors[id] is set', () => {
    // '#ff3b30' → rgb(255, 59, 48).
    const props = makeOnnxProps({}, {
      edges: [{ targetHandle: 'onnx_1_image' }],
      nodeErrors: { 'onnx-1': 'run failed' },
    });
    const { container } = render(<OnnxNode {...props} />);
    const header = container.querySelector('[style*="background-color: rgb(255, 59, 48)"]');
    expect(header).toBeTruthy();
  });

  it('marks the unconnected input port red when there is an error', () => {
    const props = makeOnnxProps({}, { nodeErrors: { 'onnx-1': 'boom' } });
    const { container } = render(<OnnxNode {...props} />);
    // Port label appears in a red-text span.
    const redPortLabel = container.querySelector('.text-\\[\\#ff3b30\\]');
    expect(redPortLabel).toBeTruthy();
  });

  it('shows a divider between inputs and outputs', () => {
    const { container } = render(<OnnxNode {...makeOnnxProps()} />);
    const divider = container.querySelector('.border-t');
    expect(divider).toBeTruthy();
  });

  it('renders the preview img when outputPreviews[id] is set', () => {
    const preview = 'data:image/png;base64,preview';
    const props = makeOnnxProps({}, { outputPreviews: { 'onnx-1': preview } });
    render(<OnnxNode {...props} />);

    const img = screen.getByAltText('detections');
    expect(img).toBeInTheDocument();
    expect(img.getAttribute('src')).toBe(preview);
  });

  it('renders resolvedWidth×resolvedHeight badge under the preview', () => {
    const props = makeOnnxProps(
      { resolvedWidth: 640, resolvedHeight: 360 },
      { outputPreviews: { 'onnx-1': 'data:image/png;base64,preview' } },
    );
    render(<OnnxNode {...props} />);

    expect(screen.getByText('ONNX 640×360')).toBeInTheDocument();
  });

  it('omits the resolved dimension badge when either dimension is missing', () => {
    const props = makeOnnxProps(
      { resolvedWidth: 640 },
      { outputPreviews: { 'onnx-1': 'data:image/png;base64,preview' } },
    );
    render(<OnnxNode {...props} />);

    expect(screen.queryByText(/ONNX 640/)).toBeNull();
  });

  it('omits the preview section entirely when no outputPreviews entry is set', () => {
    render(<OnnxNode {...makeOnnxProps()} />);
    expect(screen.queryByAltText('detections')).toBeNull();
  });

  it('selected state adds the special border class', () => {
    const props = makeOnnxProps();
    props.selected = true;
    const { container } = render(<OnnxNode {...props} />);
    expect(container.querySelector('.border-\\[\\#007aff\\]')).toBeTruthy();
  });
});
