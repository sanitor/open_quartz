import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

vi.mock('@xyflow/react', () => ({
  Handle: ({ type, id }: { type: string; position: string; id: string }) => (
    <span data-testid={`handle-${type}-${id}`} />
  ),
  Position: { Left: 'left', Right: 'right', Top: 'top', Bottom: 'bottom' },
}));

const defaultStoreState = {
  outputPreviews: {} as Record<string, string>,
  nodeErrors: {} as Record<string, string>,
  edges: [] as Array<{ targetHandle: string }>,
};

vi.mock('../../src/store/useGraphStore', () => ({
  useGraphStore: vi.fn((selector: unknown) => {
    if (typeof selector === 'function') {
      return (selector as (s: typeof defaultStoreState) => unknown)(defaultStoreState);
    }
    return {};
  }),
}));

import { OutputNode } from '../../src/components/NodeGraph/nodes/OutputNode';
import type { ShaderNodeData } from '../../src/types';

function makeOutputProps(overrides: Partial<ShaderNodeData> = {}, storeOverrides: Partial<typeof defaultStoreState> = {}) {
  // Apply store overrides
  Object.assign(defaultStoreState, {
    outputPreviews: {},
    nodeErrors: {},
    edges: [],
    ...storeOverrides,
  });

  const data: ShaderNodeData = {
    type: 'output',
    label: 'Output_0',
    shaderCode: '',
    inputs: [{ id: 'inp-1', label: 'input', dataType: 'sampler2D', direction: 'input' as const }],
    outputs: [{ id: 'out-1', label: 'output', dataType: 'sampler2D', direction: 'output' as const }],
    uniforms: {},
    ...overrides,
  };
  return {
    id: 'output-node-1',
    data,
    selected: false,
    type: 'output' as const,
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

describe('OutputNode', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    Object.assign(defaultStoreState, {
      outputPreviews: {},
      nodeErrors: {},
      edges: [],
    });
  });

  it('renders header with OUTPUT text', () => {
    const props = makeOutputProps();
    render(<OutputNode {...props} />);
    expect(screen.getByText('OUTPUT')).toBeInTheDocument();
  });

  it('renders label in header', () => {
    const props = makeOutputProps({ label: 'MyOutput' });
    render(<OutputNode {...props} />);
    expect(screen.getByText('MyOutput')).toBeInTheDocument();
  });

  it('renders input handle', () => {
    const props = makeOutputProps();
    render(<OutputNode {...props} />);
    expect(screen.getByTestId('handle-target-inp-1')).toBeInTheDocument();
  });

  it('renders output handle', () => {
    const props = makeOutputProps();
    render(<OutputNode {...props} />);
    expect(screen.getByTestId('handle-source-out-1')).toBeInTheDocument();
  });

  it('shows "Press Run to preview" when no preview', () => {
    const props = makeOutputProps();
    render(<OutputNode {...props} />);
    expect(screen.getByText('Press Run to preview')).toBeInTheDocument();
  });

  it('shows preview image when available', () => {
    const props = makeOutputProps({}, {
      outputPreviews: { 'output-node-1': 'data:image/png;base64,abc' },
    });
    render(<OutputNode {...props} />);
    const img = screen.getByAltText('preview');
    expect(img).toBeInTheDocument();
    expect(img).toHaveAttribute('src', 'data:image/png;base64,abc');
  });

  it('shows resolved dimensions when available', () => {
    const props = makeOutputProps({ resolvedWidth: 1024, resolvedHeight: 768 });
    render(<OutputNode {...props} />);
    expect(screen.getByText('RGBA8 1024×768')).toBeInTheDocument();
  });

  it('shows custom format in dimensions', () => {
    const props = makeOutputProps({ resolvedWidth: 512, resolvedHeight: 512, outFormat: 'rgba32f' });
    render(<OutputNode {...props} />);
    expect(screen.getByText('RGBA32F 512×512')).toBeInTheDocument();
  });

  it('applies green accent when preview available', () => {
    const props = makeOutputProps({}, {
      outputPreviews: { 'output-node-1': 'data:image/png;base64,abc' },
    });
    const { container } = render(<OutputNode {...props} />);
    const header = container.querySelector('[style*="background-color: rgb(48, 209, 88)"]');
    expect(header).toBeTruthy();
  });

  it('applies red accent when error present', () => {
    const props = makeOutputProps({}, {
      nodeErrors: { 'output-node-1': 'Compilation failed' },
    });
    const { container } = render(<OutputNode {...props} />);
    const header = container.querySelector('[style*="background-color: rgb(255, 59, 48)"]');
    expect(header).toBeTruthy();
  });
});
