import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

vi.mock('@xyflow/react', () => ({
  Handle: ({ type, id, style }: { type: string; position: string; id: string; style?: Record<string, string> }) => (
    <span data-testid={`handle-${type}-${id}`} style={style} />
  ),
  Position: { Left: 'left', Right: 'right', Top: 'top', Bottom: 'bottom' },
}));

interface StoreState {
  edges: Array<{ source: string; sourceHandle: string; targetHandle: string }>;
  nodes: Array<{ id: string; data: { outputs: Array<{ id: string; dataType: string }> } }>;
}

const defaultStoreState: StoreState = {
  edges: [],
  nodes: [],
};

vi.mock('../../src/store/useGraphStore', () => ({
  useGraphStore: vi.fn((selector: unknown) => {
    if (typeof selector === 'function') {
      return (selector as (s: StoreState) => unknown)(defaultStoreState);
    }
    return {};
  }),
}));

import { MathNode } from '../../src/components/NodeGraph/nodes/MathNode';
import type { ShaderNodeData, Port } from '../../src/types';

function port(overrides: Partial<Port> = {}): Port {
  return {
    id: 'in_a',
    label: 'a',
    dataType: 'auto',
    direction: 'input',
    ...overrides,
  };
}

function makeMathProps(
  dataOverrides: Partial<ShaderNodeData> = {},
  storeOverrides: Partial<StoreState> = {},
) {
  Object.assign(defaultStoreState, {
    edges: [],
    nodes: [],
    ...storeOverrides,
  });

  const data: ShaderNodeData = {
    type: 'math',
    label: 'Add',
    shaderCode: '',
    inputs: [
      port({ id: 'in_a', label: 'a', dataType: 'auto', direction: 'input' }),
      port({ id: 'in_b', label: 'b', dataType: 'auto', direction: 'input' }),
    ],
    outputs: [
      port({ id: 'out_result', label: 'result', dataType: 'auto', direction: 'output' }),
    ],
    uniforms: {},
    mathOp: 'add',
    ...dataOverrides,
  };

  return {
    id: 'math-1',
    data,
    selected: false,
    type: 'math' as const,
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

describe('MathNode', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    Object.assign(defaultStoreState, { edges: [], nodes: [] });
  });

  it("renders the op typeName 'Add' in the header", () => {
    render(<MathNode {...makeMathProps({ label: 'math-1' })} />);
    expect(screen.getByText('ADD')).toBeInTheDocument();
  });

  it('renders the op label from MATH_OPS when mathOp is set', () => {
    render(<MathNode {...makeMathProps({ mathOp: 'multiply' })} />);
    expect(screen.getByText('MULTIPLY')).toBeInTheDocument();
  });

  it('falls back to data.label when mathOp is not defined', () => {
    render(<MathNode {...makeMathProps({ mathOp: undefined, label: 'CustomNode' })} />);
    expect(screen.getByText('MATH')).toBeInTheDocument();
    expect(screen.getByText('customnode')).toBeInTheDocument();
  });

  it('renders the operation symbol for a known mathOp', () => {
    render(<MathNode {...makeMathProps({ mathOp: 'add' })} />);
    expect(screen.getByText('+')).toBeInTheDocument();
  });

  it('shows "?" symbol when no mathOp is set', () => {
    render(<MathNode {...makeMathProps({ mathOp: undefined })} />);
    expect(screen.getByText('?')).toBeInTheDocument();
  });

  it('renders unique symbols for different math operations', () => {
    const { unmount } = render(<MathNode {...makeMathProps({ mathOp: 'divide' })} />);
    expect(screen.getByText('÷')).toBeInTheDocument();
    unmount();

    render(<MathNode {...makeMathProps({ mathOp: 'sqrt' })} />);
    expect(screen.getByText('√')).toBeInTheDocument();
  });

  it('renders one target handle per input port', () => {
    render(<MathNode {...makeMathProps()} />);
    expect(screen.getByTestId('handle-target-in_a')).toBeInTheDocument();
    expect(screen.getByTestId('handle-target-in_b')).toBeInTheDocument();
  });

  it('renders one source handle per output port', () => {
    render(<MathNode {...makeMathProps()} />);
    expect(screen.getByTestId('handle-source-out_result')).toBeInTheDocument();
  });

  it('renders port labels', () => {
    render(<MathNode {...makeMathProps()} />);
    expect(screen.getByText('a')).toBeInTheDocument();
    expect(screen.getByText('b')).toBeInTheDocument();
    expect(screen.getByText('result')).toBeInTheDocument();
  });

  it('renders three input ports for ternary ops like clamp', () => {
    const props = makeMathProps({
      mathOp: 'clamp',
      inputs: [
        port({ id: 'in_a', label: 'a', direction: 'input' }),
        port({ id: 'in_b', label: 'b', direction: 'input' }),
        port({ id: 'in_c', label: 'c', direction: 'input' }),
      ],
    });
    render(<MathNode {...props} />);
    expect(screen.getByTestId('handle-target-in_a')).toBeInTheDocument();
    expect(screen.getByTestId('handle-target-in_b')).toBeInTheDocument();
    expect(screen.getByTestId('handle-target-in_c')).toBeInTheDocument();
  });

  it('renders single input port for unary ops like negate', () => {
    const props = makeMathProps({
      mathOp: 'negate',
      inputs: [port({ id: 'in_a', label: 'a', direction: 'input' })],
    });
    render(<MathNode {...props} />);
    expect(screen.getByTestId('handle-target-in_a')).toBeInTheDocument();
    expect(screen.queryByTestId('handle-target-in_b')).toBeNull();
  });

  it('paints the header with unified dark-blue background', () => {
    // HEADER_BG = '#1e293b' → rgb(30, 41, 59)
    const { container } = render(<MathNode {...makeMathProps()} />);
    const header = container.querySelector('[style*="background-color: rgb(30, 41, 59)"]');
    expect(header).toBeTruthy();
  });

  it('selected state adds the special border class', () => {
    const props = makeMathProps();
    props.selected = true;
    const { container } = render(<MathNode {...props} />);
    expect(container.querySelector('.border-\\[\\#007aff\\]')).toBeTruthy();
  });

  it('unselected state has default border class', () => {
    const { container } = render(<MathNode {...makeMathProps()} />);
    expect(container.querySelector('.border-\\[\\#d2d2d7\\]')).toBeTruthy();
  });

  it('infers input port color from connected source port dataType', () => {
    // Connect in_a to a source node whose output port has dataType 'float'
    // DATA_TYPE_COLORS.float = '#4fc3f7'
    const props = makeMathProps({}, {
      edges: [{ source: 'src-1', sourceHandle: 'src_out', targetHandle: 'in_a' }],
      nodes: [{
        id: 'src-1',
        data: { outputs: [{ id: 'src_out', dataType: 'float' }] },
      }],
    });
    render(<MathNode {...props} />);
    const handle = screen.getByTestId('handle-target-in_a');
    expect(handle.style.borderColor).toBe('rgb(79, 195, 247)');
  });

  it('uses PORT_COLOR for unconnected input ports', () => {
    // PORT_COLOR = '#8e8e93' → rgb(142, 142, 147)
    render(<MathNode {...makeMathProps()} />);
    const handle = screen.getByTestId('handle-target-in_a');
    expect(handle.style.borderColor).toBe('rgb(142, 142, 147)');
  });

  it('output port color inferred as widest connected input type', () => {
    // Connect both inputs: in_a→float, in_b→vec3. vec3 is wider.
    // DATA_TYPE_COLORS.vec3 = '#e57373'
    const props = makeMathProps({}, {
      edges: [
        { source: 'src-1', sourceHandle: 'src_out_a', targetHandle: 'in_a' },
        { source: 'src-2', sourceHandle: 'src_out_b', targetHandle: 'in_b' },
      ],
      nodes: [
        { id: 'src-1', data: { outputs: [{ id: 'src_out_a', dataType: 'float' }] } },
        { id: 'src-2', data: { outputs: [{ id: 'src_out_b', dataType: 'vec3' }] } },
      ],
    });
    render(<MathNode {...props} />);
    const outputHandle = screen.getByTestId('handle-source-out_result');
    expect(outputHandle.style.backgroundColor).toBe('rgb(229, 115, 115)');
  });

  it('connected input port handle gets filled background', () => {
    // When connected, backgroundColor = color (not transparent)
    const props = makeMathProps({}, {
      edges: [{ source: 'src-1', sourceHandle: 'src_out', targetHandle: 'in_a' }],
      nodes: [{
        id: 'src-1',
        data: { outputs: [{ id: 'src_out', dataType: 'float' }] },
      }],
    });
    render(<MathNode {...props} />);
    const connectedHandle = screen.getByTestId('handle-target-in_a');
    expect(connectedHandle.style.backgroundColor).toBe('rgb(79, 195, 247)');
  });

  it('unconnected input port handle has transparent background', () => {
    render(<MathNode {...makeMathProps()} />);
    const handle = screen.getByTestId('handle-target-in_a');
    expect(handle.style.backgroundColor).toBe('transparent');
  });

  it('falls back to data.label as symbol when mathOp is not in OP_SYMBOLS', () => {
    // symbol = data.mathOp ? (OP_SYMBOLS[data.mathOp] ?? data.label) : '?'
    // When mathOp is set but not in OP_SYMBOLS or MATH_OPS:
    // typeName = op?.label ?? 'Math' = 'Math' (op is undefined) → UPPERCASE → 'MATH'
    // symbol = OP_SYMBOLS['unknownOp'] ?? data.label = 'Fallback'
    // label = 'Fallback' → lowercase → 'fallback'
    const props = makeMathProps({ mathOp: 'unknownOp', label: 'Fallback' });
    render(<MathNode {...props} />);
    expect(screen.getByText('MATH')).toBeInTheDocument();
    expect(screen.getByText('Fallback')).toBeInTheDocument();
    expect(screen.getByText('fallback')).toBeInTheDocument();
  });
});
