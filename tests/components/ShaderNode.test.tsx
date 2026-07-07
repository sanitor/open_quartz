import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

vi.mock('@xyflow/react', () => ({
  Handle: ({ type, id }: { type: string; position: string; id: string }) => (
    <span data-testid={`handle-${type}-${id}`} />
  ),
  Position: { Left: 'left', Right: 'right', Top: 'top', Bottom: 'bottom' },
}));

const defaultStoreState = {
  edges: [] as Array<{ targetHandle: string }>,
  nodeErrors: {} as Record<string, string>,
};

vi.mock('../../src/store/useGraphStore', () => ({
  useGraphStore: vi.fn((selector: unknown) => {
    if (typeof selector === 'function') {
      return (selector as (s: typeof defaultStoreState) => unknown)(defaultStoreState);
    }
    return {};
  }),
}));

import { ShaderNode } from '../../src/components/NodeGraph/nodes/ShaderNode';
import type { ShaderNodeData, Port } from '../../src/types';

function makePort(overrides: Partial<Port> = {}): Port {
  return {
    id: 'port-1',
    label: 'color',
    dataType: 'vec4',
    direction: 'input',
    ...overrides,
  };
}

function makeShaderProps(dataOverrides: Partial<ShaderNodeData> = {}, storeOverrides: Partial<typeof defaultStoreState> = {}) {
  Object.assign(defaultStoreState, {
    edges: [],
    nodeErrors: {},
    ...storeOverrides,
  });

  const data: ShaderNodeData = {
    type: 'shader',
    label: 'MyShader',
    shaderCode: 'void main() {}',
    inputs: [
      makePort({ id: 'in-1', label: 'color', dataType: 'vec4', direction: 'input' }),
    ],
    outputs: [
      makePort({ id: 'out-1', label: 'fragColor', dataType: 'vec4', direction: 'output' }),
    ],
    uniforms: {},
    ...dataOverrides,
  };

  return {
    id: 'shader-1',
    data,
    selected: false,
    type: 'shader' as const,
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

describe('ShaderNode', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    Object.assign(defaultStoreState, { edges: [], nodeErrors: {} });
  });

  it('renders header with SHADER text', () => {
    const props = makeShaderProps();
    render(<ShaderNode {...props} />);
    expect(screen.getByText('SHADER')).toBeInTheDocument();
  });

  it('renders label in header', () => {
    const props = makeShaderProps({ label: 'Blur' });
    render(<ShaderNode {...props} />);
    expect(screen.getByText('Blur')).toBeInTheDocument();
  });

  it('renders input handles for each input port', () => {
    const props = makeShaderProps({
      inputs: [
        makePort({ id: 'in-1', label: 'texA', direction: 'input' }),
        makePort({ id: 'in-2', label: 'texB', direction: 'input' }),
      ],
    });
    render(<ShaderNode {...props} />);
    expect(screen.getByTestId('handle-target-in-1')).toBeInTheDocument();
    expect(screen.getByTestId('handle-target-in-2')).toBeInTheDocument();
  });

  it('renders output handles for each output port', () => {
    const props = makeShaderProps({
      outputs: [
        makePort({ id: 'out-1', label: 'fragColor', direction: 'output' }),
        makePort({ id: 'out-2', label: 'fragDepth', direction: 'output' }),
      ],
    });
    render(<ShaderNode {...props} />);
    expect(screen.getByTestId('handle-source-out-1')).toBeInTheDocument();
    expect(screen.getByTestId('handle-source-out-2')).toBeInTheDocument();
  });

  it('displays port labels and data types', () => {
    const props = makeShaderProps({
      inputs: [makePort({ id: 'in-1', label: 'brightness', dataType: 'float' })],
      outputs: [makePort({ id: 'out-1', label: 'result', dataType: 'vec4' })],
    });
    render(<ShaderNode {...props} />);
    expect(screen.getByText('brightness')).toBeInTheDocument();
    expect(screen.getByText('result')).toBeInTheDocument();
    expect(screen.getByText('float')).toBeInTheDocument();
    expect(screen.getByText('vec4')).toBeInTheDocument();
  });

  it('applies purple accent for shader type', () => {
    const props = makeShaderProps({}, { edges: [{ targetHandle: 'in-1' }] });
    const { container } = render(<ShaderNode {...props} />);
    const header = container.querySelector('[style*="background-color: rgb(175, 82, 222)"]');
    expect(header).toBeTruthy();
  });

  it('applies red accent when node has error', () => {
    const props = makeShaderProps({}, { nodeErrors: { 'shader-1': 'compile error' } });
    const { container } = render(<ShaderNode {...props} />);
    const header = container.querySelector('[style*="background-color: rgb(255, 59, 48)"]');
    expect(header).toBeTruthy();
  });

  it('applies gray accent when input is unconnected', () => {
    const props = makeShaderProps();
    const { container } = render(<ShaderNode {...props} />);
    const header = container.querySelector('[style*="background-color: rgb(142, 142, 147)"]');
    expect(header).toBeTruthy();
  });

  it('renders divider between inputs and outputs', () => {
    const props = makeShaderProps();
    const { container } = render(<ShaderNode {...props} />);
    const divider = container.querySelector('.border-t');
    expect(divider).toBeTruthy();
  });

  it('selected state adds special border class', () => {
    const props = makeShaderProps();
    props.selected = true;
    const { container } = render(<ShaderNode {...props} />);
    expect(container.querySelector('.border-\\[\\#007aff\\]')).toBeTruthy();
  });
});
