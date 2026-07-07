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
      isRunning: false,
      nodes: [],
      edges: [],
      setOutputPreview: vi.fn(),
      clearOutputPreviews: vi.fn(),
      setNodeError: vi.fn(),
      clearNodeErrors: vi.fn(),
      setSelectedNode: vi.fn(),
      updateNodeData: vi.fn(),
      setRunning: vi.fn(),
    })),
  }),
}));

// Mock execution engine
const { mockEngineRun, mockEngineStop } = vi.hoisted(() => ({
  mockEngineRun: vi.fn(() => Promise.resolve()),
  mockEngineStop: vi.fn(),
}));
vi.mock('../../src/engine/executionEngine', () => ({
  ExecutionEngine: vi.fn(function (this: Record<string, unknown>) {
    this.run = mockEngineRun;
    this.stop = mockEngineStop;
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

  it('runs execution engine when isRunning transitions to true', async () => {
    let subscribeCb: ((state: Record<string, unknown>, prev: Record<string, unknown>) => void) | null = null;
    mockSubscribe.mockImplementation((cb: (state: Record<string, unknown>, prev: Record<string, unknown>) => void) => {
      subscribeCb = cb;
      return vi.fn();
    });

    render(<App />);
    expect(subscribeCb).not.toBeNull();

    // Simulate isRunning transition from false to true
    subscribeCb!({ isRunning: true, nodes: [], edges: [] }, { isRunning: false });

    // Wait for async execution
    await vi.waitFor(() => {
      expect(mockEngineRun).toHaveBeenCalled();
    });
  });

  it('does not run engine when isRunning stays false', () => {
    let subscribeCb: ((state: Record<string, unknown>, prev: Record<string, unknown>) => void) | null = null;
    mockSubscribe.mockImplementation((cb: (state: Record<string, unknown>, prev: Record<string, unknown>) => void) => {
      subscribeCb = cb;
      return vi.fn();
    });

    render(<App />);
    subscribeCb!({ isRunning: false }, { isRunning: false });
    expect(mockEngineRun).not.toHaveBeenCalled();
  });
});
