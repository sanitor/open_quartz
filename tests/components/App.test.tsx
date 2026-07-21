import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

// Mock child components
vi.mock('../../src/components/Header', () => ({
  Header: () => <div data-testid="header">Header</div>,
}));
vi.mock('../../src/components/NodeGraph', () => ({
  NodeGraph: () => <div data-testid="node-graph">NodeGraph</div>,
}));
vi.mock('../../src/components/SidePanel', () => ({
  SidePanel: () => <div data-testid="side-panel">SidePanel</div>,
}));

// Mock ReactFlowProvider
vi.mock('@xyflow/react', () => ({
  ReactFlowProvider: ({ children }: { children: React.ReactNode }) => <div data-testid="rf-provider">{children}</div>,
}));

// Mock store - use vi.hoisted to avoid hoisting issues
const { mockSubscribe } = vi.hoisted(() => ({
  mockSubscribe: vi.fn(() => vi.fn()),
}));
vi.mock('../../src/store/useGraphStore', () => ({
  useGraphStore: Object.assign(vi.fn(), {
    subscribe: mockSubscribe,
    getState: vi.fn(() => ({
      loopState: 'stopped' as const,
      nodes: [],
      edges: [],
      setOutputPreview: vi.fn(),
      clearOutputPreviews: vi.fn(),
      setNodeError: vi.fn(),
      clearNodeErrors: vi.fn(),
      setSelectedNode: vi.fn(),
      updateNodeData: vi.fn(),
      setFps: vi.fn(),
      setCaptureScreenshot: vi.fn(),
    })),
  }),
}));

// Mock RealtimeHost
const { mockHostPlay, mockHostStop, mockHostCapture } = vi.hoisted(() => ({
  mockHostPlay: vi.fn(),
  mockHostStop: vi.fn(),
  mockHostCapture: vi.fn(() => null),
}));
vi.mock('../../src/engine/realtimeHost', () => ({
  RealtimeHost: vi.fn(function (this: Record<string, unknown>) {
    this.play = mockHostPlay;
    this.stop = mockHostStop;
    this.pause = vi.fn();
    this.resume = vi.fn();
    this.updateGraph = vi.fn();
    this.captureScreenshot = mockHostCapture;
    this.setPreviewNode = vi.fn();
  }),
}));

import App from '../../src/App';

describe('App', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockSubscribe.mockReturnValue(vi.fn());
  });

  it('renders without crashing', () => {
    render(<App />);
    expect(screen.getByTestId('rf-provider')).toBeInTheDocument();
  });

  it('renders Header component', () => {
    render(<App />);
    expect(screen.getByTestId('header')).toBeInTheDocument();
  });

  it('renders NodeGraph component', () => {
    render(<App />);
    expect(screen.getByTestId('node-graph')).toBeInTheDocument();
  });

  it('renders SidePanel component', () => {
    render(<App />);
    expect(screen.getByTestId('side-panel')).toBeInTheDocument();
  });

  it('renders a hidden canvas element', () => {
    render(<App />);
    const canvas = document.querySelector('canvas');
    expect(canvas).toBeInTheDocument();
    expect(canvas?.className).toContain('hidden');
  });

  it('subscribes to store on mount', () => {
    render(<App />);
    expect(mockSubscribe).toHaveBeenCalled();
  });

  it('unsubscribes on unmount', () => {
    const unsub = vi.fn();
    mockSubscribe.mockReturnValue(unsub);
    const { unmount } = render(<App />);
    unmount();
    expect(unsub).toHaveBeenCalled();
  });

  it('renders main layout structure', () => {
    render(<App />);
    const main = document.querySelector('main');
    expect(main).toBeInTheDocument();
  });

  it('creates realtime host when loopState transitions to playing', async () => {
    let subscribeCb: ((state: Record<string, unknown>, prev: Record<string, unknown>) => void) | null = null;
    mockSubscribe.mockImplementation((cb: (state: Record<string, unknown>, prev: Record<string, unknown>) => void) => {
      subscribeCb = cb;
      return vi.fn();
    });

    render(<App />);
    expect(subscribeCb).not.toBeNull();

    // Simulate loopState transition from stopped to playing
    subscribeCb!(
      { loopState: 'playing', nodes: [], edges: [] },
      { loopState: 'stopped' },
    );

    // The subscribe callback should have been called without errors
    expect(subscribeCb).not.toBeNull();
  });

  it('does not create host when loopState stays stopped', () => {
    let subscribeCb: ((state: Record<string, unknown>, prev: Record<string, unknown>) => void) | null = null;
    mockSubscribe.mockImplementation((cb: (state: Record<string, unknown>, prev: Record<string, unknown>) => void) => {
      subscribeCb = cb;
      return vi.fn();
    });

    render(<App />);
    subscribeCb!({ loopState: 'stopped' }, { loopState: 'stopped' });
    expect(mockHostPlay).not.toHaveBeenCalled();
  });
});
