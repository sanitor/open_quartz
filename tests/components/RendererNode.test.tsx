import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

vi.mock('@xyflow/react', () => ({
  Handle: ({ type, id }: { type: string; position: string; id: string }) => (
    <span data-testid={`handle-${type}-${id}`} />
  ),
  Position: { Left: 'left', Right: 'right', Top: 'top', Bottom: 'bottom' },
}));

interface StoreState {
  edges: Array<{ targetHandle: string }>;
  nodeErrors: Record<string, string>;
  loopState: 'stopped' | 'playing' | 'paused';
}

const defaultStoreState: StoreState = {
  edges: [],
  nodeErrors: {},
  loopState: 'stopped',
};

const mockSetActiveRenderer = vi.fn();
const mockUpdateNodeData = vi.fn();

vi.mock('../../src/store/useGraphStore', () => ({
  useGraphStore: Object.assign(
    vi.fn((selector: unknown) => {
      if (typeof selector === 'function') {
        return (selector as (s: StoreState) => unknown)(defaultStoreState);
      }
      return {};
    }),
    {
      getState: () => ({
        setActiveRenderer: mockSetActiveRenderer,
        updateNodeData: mockUpdateNodeData,
      }),
    },
  ),
}));

import { RendererNode } from '../../src/components/NodeGraph/nodes/RendererNode';
import type { ShaderNodeData, Port } from '../../src/types';

function port(overrides: Partial<Port> = {}): Port {
  return {
    id: 'r1-input',
    label: 'inputTexture',
    dataType: 'sampler2D',
    direction: 'input',
    ...overrides,
  };
}

function makeProps(
  dataOverrides: Partial<ShaderNodeData> = {},
  storeOverrides: Partial<StoreState> = {},
) {
  Object.assign(defaultStoreState, {
    edges: [],
    nodeErrors: {},
    loopState: 'stopped',
    ...storeOverrides,
  });

  const data: ShaderNodeData = {
    type: 'renderer',
    label: 'Screen',
    shaderCode: '',
    inputs: [port()],
    outputs: [],
    uniforms: {},
    resolvedWidth: 1920,
    resolvedHeight: 1080,
    expanded: true,
    ...dataOverrides,
  };

  return {
    id: 'renderer-1',
    data,
    selected: false,
    type: 'renderer' as const,
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

describe('RendererNode', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    Object.assign(defaultStoreState, {
      edges: [],
      nodeErrors: {},
      loopState: 'stopped',
    });
  });

  it('renders green header with RENDERER text when connected', () => {
    const props = makeProps({}, { edges: [{ targetHandle: 'r1-input' }] });
    const { container } = render(<RendererNode {...props} />);
    expect(screen.getByText('RENDERER')).toBeInTheDocument();
    // #34c759 = rgb(52, 199, 89) — green accent
    const header = container.querySelector('[style*="background-color: rgb(52, 199, 89)"]');
    expect(header).toBeTruthy();
  });

  it('renders label from data.label', () => {
    const props = makeProps({ label: 'MainOutput' });
    render(<RendererNode {...props} />);
    expect(screen.getByText('MainOutput')).toBeInTheDocument();
  });

  it('renders input handle for inputTexture (sampler2D)', () => {
    const props = makeProps();
    render(<RendererNode {...props} />);
    expect(screen.getByTestId('handle-target-r1-input')).toBeInTheDocument();
    expect(screen.getByText('inputTexture')).toBeInTheDocument();
    expect(screen.getByText('sampler2D')).toBeInTheDocument();
  });

  it('has no output handles', () => {
    const props = makeProps();
    render(<RendererNode {...props} />);
    // No handle-source-* should exist
    expect(screen.queryByTestId(/^handle-source-/)).toBeNull();
  });

  it('shows expand/collapse toggle button', () => {
    const props = makeProps();
    render(<RendererNode {...props} />);
    // expanded=true → shows ▴
    expect(screen.getByText('▴')).toBeInTheDocument();
  });

  it('toggle button shows ▾ when collapsed', () => {
    const props = makeProps({ expanded: false });
    render(<RendererNode {...props} />);
    expect(screen.getByText('▾')).toBeInTheDocument();
  });

  it('clicking toggle calls updateNodeData to flip expanded', () => {
    const props = makeProps();
    render(<RendererNode {...props} />);
    fireEvent.click(screen.getByText('▴'));
    expect(mockUpdateNodeData).toHaveBeenCalledWith('renderer-1', { expanded: false });
  });

  it('when expanded + playing: shows mirror canvas with correct id', () => {
    const props = makeProps({ expanded: true }, { loopState: 'playing' });
    const { container } = render(<RendererNode {...props} />);
    const canvas = container.querySelector('canvas#renderer-mirror-renderer-1');
    expect(canvas).toBeTruthy();
  });

  it('when expanded + paused: shows mirror canvas (isPlaying = loopState !== stopped)', () => {
    const props = makeProps({ expanded: true }, { loopState: 'paused' });
    const { container } = render(<RendererNode {...props} />);
    const canvas = container.querySelector('canvas#renderer-mirror-renderer-1');
    expect(canvas).toBeTruthy();
  });

  it('when expanded + stopped: does NOT show mirror canvas', () => {
    const props = makeProps({ expanded: true }, { loopState: 'stopped' });
    const { container } = render(<RendererNode {...props} />);
    const canvas = container.querySelector('canvas');
    expect(canvas).toBeNull();
  });

  it('when collapsed: does not show preview area or resolution', () => {
    const props = makeProps({ expanded: false }, { loopState: 'playing' });
    const { container } = render(<RendererNode {...props} />);
    expect(container.querySelector('canvas')).toBeNull();
    expect(screen.queryByText('1920×1080')).toBeNull();
  });

  it('resolution display shows resolvedWidth×resolvedHeight', () => {
    const props = makeProps(
      { resolvedWidth: 1920, resolvedHeight: 1080 },
      { loopState: 'playing' },
    );
    render(<RendererNode {...props} />);
    expect(screen.getByText('1920×1080')).toBeInTheDocument();
  });

  it('canvas dimensions match resolvedWidth/resolvedHeight', () => {
    const props = makeProps(
      { resolvedWidth: 800, resolvedHeight: 600 },
      { loopState: 'playing' },
    );
    const { container } = render(<RendererNode {...props} />);
    const canvas = container.querySelector('canvas') as HTMLCanvasElement;
    expect(canvas.width).toBe(800);
    expect(canvas.height).toBe(600);
  });

  it('preview width is clamped to MAX_PREVIEW_W=200 with proportional height', () => {
    const props = makeProps(
      { resolvedWidth: 1920, resolvedHeight: 1080 },
      { loopState: 'playing' },
    );
    const { container } = render(<RendererNode {...props} />);
    const canvas = container.querySelector('canvas') as HTMLCanvasElement;
    // previewW = min(1920, 200) = 200
    // previewH = round(200 * (1080/1920)) = round(112.5) = 113
    expect(canvas.style.width).toBe('200px');
    expect(canvas.style.height).toBe('113px');
  });

  it('click preview calls setActiveRenderer with node id', () => {
    const props = makeProps({}, { loopState: 'playing' });
    const { container } = render(<RendererNode {...props} />);
    const canvas = container.querySelector('canvas')!;
    fireEvent.click(canvas);
    expect(mockSetActiveRenderer).toHaveBeenCalledWith('renderer-1');
  });

  it('selected state adds blue border class', () => {
    const props = makeProps();
    props.selected = true;
    const { container } = render(<RendererNode {...props} />);
    expect(container.querySelector('.border-\\[\\#007aff\\]')).toBeTruthy();
  });

  it('unselected state uses default border class', () => {
    const props = makeProps();
    const { container } = render(<RendererNode {...props} />);
    expect(container.querySelector('.border-\\[\\#d2d2d7\\]')).toBeTruthy();
    expect(container.querySelector('.border-\\[\\#007aff\\]')).toBeNull();
  });

  it('error state changes header to red', () => {
    const props = makeProps({}, { nodeErrors: { 'renderer-1': 'GPU error' } });
    const { container } = render(<RendererNode {...props} />);
    // #ff3b30 = rgb(255, 59, 48)
    const header = container.querySelector('[style*="background-color: rgb(255, 59, 48)"]');
    expect(header).toBeTruthy();
  });

  it('error state also colors port border and label', () => {
    const props = makeProps(
      {},
      { nodeErrors: { 'renderer-1': 'GPU error' }, edges: [{ targetHandle: 'r1-input' }] },
    );
    const { container } = render(<RendererNode {...props} />);
    // port label gets red text class
    const portLabel = screen.getByText('inputTexture');
    expect(portLabel.className).toContain('text-[#ff3b30]');
  });

  it('unconnected input shows gray header', () => {
    const props = makeProps({}, { edges: [] });
    const { container } = render(<RendererNode {...props} />);
    // #8e8e93 = rgb(142, 142, 147)
    const header = container.querySelector('[style*="background-color: rgb(142, 142, 147)"]');
    expect(header).toBeTruthy();
  });

  it('small resolution does not exceed actual width for preview', () => {
    // When resolvedWidth < MAX_PREVIEW_W, previewW = resolvedWidth
    const props = makeProps(
      { resolvedWidth: 128, resolvedHeight: 128 },
      { loopState: 'playing' },
    );
    const { container } = render(<RendererNode {...props} />);
    const canvas = container.querySelector('canvas') as HTMLCanvasElement;
    expect(canvas.style.width).toBe('128px');
    expect(canvas.style.height).toBe('128px');
  });

  it('no input ports renders no handles or port labels', () => {
    const props = makeProps({ inputs: [] });
    render(<RendererNode {...props} />);
    expect(screen.queryByTestId(/^handle-/)).toBeNull();
    expect(screen.queryByText('inputTexture')).toBeNull();
  });

  it('collapsed input port row uses larger bottom padding', () => {
    const props = makeProps({ expanded: false });
    const { container } = render(<RendererNode {...props} />);
    // paddingBottom: expanded ? 2 : 6 → collapsed uses 6
    const portRow = container.querySelector('[style*="padding-bottom: 6px"]');
    expect(portRow).toBeTruthy();
  });

  it('expanded input port row uses smaller bottom padding', () => {
    const props = makeProps({ expanded: true });
    const { container } = render(<RendererNode {...props} />);
    const portRow = container.querySelector('[style*="padding-bottom: 2px"]');
    expect(portRow).toBeTruthy();
  });

  it('unconnected port has transparent background', () => {
    // No edges → not connected → port bg transparent
    const props = makeProps({}, { edges: [] });
    const { container } = render(<RendererNode {...props} />);
    const handle = container.querySelector('[data-testid="handle-target-r1-input"]');
    expect(handle).toBeTruthy();
    // The port label should NOT have error styling
    const label = screen.getByText('inputTexture');
    expect(label.className).not.toContain('text-[#ff3b30]');
  });
});
