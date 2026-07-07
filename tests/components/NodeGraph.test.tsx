import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

// Mock @xyflow/react
vi.mock('@xyflow/react', () => ({
  ReactFlow: ({ children, ...props }: Record<string, unknown>) => (
    <div data-testid="react-flow" data-node-types={props.nodeTypes ? 'custom' : undefined} data-edge-types={props.edgeTypes ? 'custom' : undefined}>
      {children as React.ReactNode}
    </div>
  ),
  Background: (props: Record<string, unknown>) => <div data-testid="background" data-variant={props.variant} />,
  BackgroundVariant: { Cross: 'cross', Dots: 'dots', Lines: 'lines' },
  Controls: () => <div data-testid="controls" />,
  MiniMap: (props: Record<string, unknown>) => <div data-testid="minimap" data-node-color={props.nodeColor} />,
  Handle: ({ type, position, id }: { type: string; position: string; id: string }) => (
    <span data-testid={`handle-${type}-${id}`} data-position={position} />
  ),
  Position: { Left: 'left', Right: 'right', Top: 'top', Bottom: 'bottom' },
  SelectionMode: { Partial: 'partial', Full: 'full' },
  useReactFlow: () => ({
    getNodes: () => [],
    getViewport: () => ({ x: 0, y: 0, zoom: 1 }),
    getNodesBounds: () => ({ x: 0, y: 0, width: 100, height: 100 }),
    fitView: vi.fn(),
  }),
  useOnViewportChange: vi.fn(),
  useConnection: () => ({ isValid: null }),
  getBezierPath: () => ['M0,0 C50,0 50,100 100,100'],
  BaseEdge: ({ id, path }: { id: string; path: string }) => (
    <path data-testid={`edge-${id}`} d={path} />
  ),
}));

vi.mock('@xyflow/react/dist/style.css', () => ({}));

// Mock useGraphStore
vi.mock('../../src/store/useGraphStore', () => ({
  useGraphStore: Object.assign(
    vi.fn(() => ({
      nodes: [],
      edges: [],
      onNodesChange: vi.fn(),
      onEdgesChange: vi.fn(),
      onConnect: vi.fn(),
      setSelectedNode: vi.fn(),
      removeSelectedElements: vi.fn(),
    })),
    {
      getState: vi.fn(() => ({
        nodes: [],
        edges: [],
      })),
      subscribe: vi.fn(() => vi.fn()),
      setState: vi.fn(),
    },
  ),
}));

// Mock child node/edge components
vi.mock('../../src/components/NodeGraph/nodes/ShaderNode', () => ({
  ShaderNode: () => <div data-testid="shader-node" />,
}));
vi.mock('../../src/components/NodeGraph/nodes/InputNode', () => ({
  InputNode: () => <div data-testid="input-node" />,
}));
vi.mock('../../src/components/NodeGraph/edges/CustomEdge', () => ({
  CustomEdge: () => <div data-testid="custom-edge" />,
}));

import { NodeGraph } from '../../src/components/NodeGraph';

describe('NodeGraph', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders ReactFlow container', () => {
    render(<NodeGraph />);
    expect(screen.getByTestId('react-flow')).toBeInTheDocument();
  });

  it('registers custom node types and edge types', () => {
    render(<NodeGraph />);
    const rf = screen.getByTestId('react-flow');
    expect(rf).toHaveAttribute('data-node-types', 'custom');
    expect(rf).toHaveAttribute('data-edge-types', 'custom');
  });

  it('renders Background with Cross variant', () => {
    render(<NodeGraph />);
    expect(screen.getByTestId('background')).toBeInTheDocument();
    expect(screen.getByTestId('background')).toHaveAttribute('data-variant', 'cross');
  });

  it('renders Controls', () => {
    render(<NodeGraph />);
    expect(screen.getByTestId('controls')).toBeInTheDocument();
  });
});
