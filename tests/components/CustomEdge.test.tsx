import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

const mockGetBezierPath = vi.fn(() => ['M0,0 C50,0 50,100 100,100']);

vi.mock('@xyflow/react', () => ({
  getBezierPath: (...args: unknown[]) => mockGetBezierPath(...args),
  BaseEdge: ({ id, path, style }: { id: string; path: string; style: Record<string, unknown> }) => (
    <svg data-testid="svg-wrapper">
      <path
        data-testid={`edge-path-${id}`}
        d={path}
        stroke={style.stroke as string}
        strokeWidth={style.strokeWidth as number}
      />
    </svg>
  ),
  Position: { Left: 'left', Right: 'right', Top: 'top', Bottom: 'bottom' },
}));

import { CustomEdge } from '../../src/components/NodeGraph/edges/CustomEdge';

function makeEdgeProps(selected = false) {
  return {
    id: 'edge-1',
    source: 'node-1',
    target: 'node-2',
    sourceX: 0,
    sourceY: 0,
    targetX: 100,
    targetY: 100,
    sourcePosition: 'right' as const,
    targetPosition: 'left' as const,
    selected,
    animated: false,
    markerEnd: undefined,
    markerStart: undefined,
    pathOptions: undefined,
    interactionWidth: undefined,
    sourceHandleId: null,
    targetHandleId: null,
    data: undefined,
    style: {},
    label: undefined,
    labelStyle: undefined,
    labelShowBg: undefined,
    labelBgStyle: undefined,
    labelBgPadding: undefined,
    labelBgBorderRadius: undefined,
    deletable: true,
    selectable: true,
  };
}

describe('CustomEdge', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders an SVG path element', () => {
    render(<CustomEdge {...makeEdgeProps()} />);
    expect(screen.getByTestId('edge-path-edge-1')).toBeInTheDocument();
  });

  it('calls getBezierPath with source/target coordinates', () => {
    render(<CustomEdge {...makeEdgeProps()} />);
    expect(mockGetBezierPath).toHaveBeenCalledWith(
      expect.objectContaining({
        sourceX: 0,
        sourceY: 0,
        targetX: 100,
        targetY: 100,
      }),
    );
  });

  it('uses gray stroke when not selected', () => {
    render(<CustomEdge {...makeEdgeProps(false)} />);
    const path = screen.getByTestId('edge-path-edge-1');
    expect(path).toHaveAttribute('stroke', '#8e8e93');
    expect(path).toHaveAttribute('stroke-width', '1.5');
  });

  it('uses blue stroke when selected', () => {
    render(<CustomEdge {...makeEdgeProps(true)} />);
    const path = screen.getByTestId('edge-path-edge-1');
    expect(path).toHaveAttribute('stroke', '#007aff');
    expect(path).toHaveAttribute('stroke-width', '3');
  });
});
