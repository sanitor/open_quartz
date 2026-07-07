import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import type { ShaderNodeData, Port } from '../../src/types';

// Mock child components
vi.mock('../../src/components/SidePanel/ShaderEditor', () => ({
  ShaderEditor: ({ code, onChange }: { code: string; onChange: (v: string) => void }) => (
    <div data-testid="shader-editor" data-code={code} onClick={() => onChange('modified')}>
      ShaderEditor
    </div>
  ),
}));

vi.mock('../../src/components/SidePanel/PortInspector', () => ({
  PortInspector: ({ inputs, outputs }: { inputs: Port[]; outputs: Port[] }) => (
    <div data-testid="port-inspector" data-inputs={inputs.length} data-outputs={outputs.length}>
      PortInspector
    </div>
  ),
}));

vi.mock('../../src/components/ImageLightbox', () => ({
  ImageLightbox: ({ src, onClose }: { src: string; onClose: () => void }) => (
    <div data-testid="lightbox" data-src={src}>
      <button data-testid="lightbox-close" onClick={onClose}>Close</button>
    </div>
  ),
}));

// Mock rawPreview utility
vi.mock('../../src/utils/rawPreview', () => ({
  generateRawPreview: vi.fn(() => 'data:image/png;base64,mockpreview'),
}));

// Store mock
const mockUpdateNodeData = vi.fn();
const mockRemoveNode = vi.fn();

function makePort(id: string, label: string, dataType: string, direction: 'input' | 'output'): Port {
  return { id, label, dataType: dataType as Port['dataType'], direction };
}

function makeShaderNodeData(overrides: Partial<ShaderNodeData> = {}): ShaderNodeData {
  return {
    type: 'shader',
    label: 'TestShader',
    shaderCode: 'void main() {}',
    inputs: [makePort('p1', 'inputImage', 'sampler2D', 'input')],
    outputs: [makePort('p2', 'fragColor', 'vec4', 'output')],
    uniforms: {},
    ...overrides,
  };
}

const defaultStoreState = {
  nodes: [] as Array<{ id: string; type: string; position: { x: number; y: number }; data: ShaderNodeData; selected?: boolean }>,
  selectedNodeId: null as string | null,
  updateNodeData: mockUpdateNodeData,
  removeNode: mockRemoveNode,
  outputPreviews: {} as Record<string, string>,
  nodeErrors: {} as Record<string, string>,
};

const mockUseGraphStore = vi.fn(() => defaultStoreState);

vi.mock('../../src/store/useGraphStore', () => ({
  useGraphStore: (...args: unknown[]) => mockUseGraphStore(...(args as [])),
}));

import { SidePanel } from '../../src/components/SidePanel';

function renderSidePanel(overrides: Partial<typeof defaultStoreState> = {}) {
  const state = { ...defaultStoreState, ...overrides };
  mockUseGraphStore.mockReturnValue(state);
  return render(<SidePanel />);
}

describe('SidePanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockUseGraphStore.mockReturnValue(defaultStoreState);
  });

  // --- Empty state ---

  it('returns null when no node is selected', () => {
    const { container } = renderSidePanel({ selectedNodeId: null, nodes: [] });
    expect(container.innerHTML).toBe('');
  });

  it('returns null when selectedNodeId does not match any node', () => {
    const { container } = renderSidePanel({
      selectedNodeId: 'nonexistent',
      nodes: [{ id: 'n1', type: 'shader', position: { x: 0, y: 0 }, data: makeShaderNodeData() }],
    });
    expect(container.innerHTML).toBe('');
  });

  // --- Shader node selected ---

  it('renders type badge for shader node', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{ id: 'n1', type: 'shader', position: { x: 0, y: 0 }, data: makeShaderNodeData({ type: 'shader' }) }],
    });
    expect(screen.getByText('SHADER')).toBeInTheDocument();
  });

  it('renders label input for selected node', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{ id: 'n1', type: 'shader', position: { x: 0, y: 0 }, data: makeShaderNodeData({ label: 'MyShader' }) }],
    });
    const input = screen.getByDisplayValue('MyShader');
    expect(input).toBeInTheDocument();
  });

  it('renders Delete button for selected node', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{ id: 'n1', type: 'shader', position: { x: 0, y: 0 }, data: makeShaderNodeData() }],
    });
    expect(screen.getByText('Delete')).toBeInTheDocument();
  });

  it('Delete button calls removeNode', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{ id: 'n1', type: 'shader', position: { x: 0, y: 0 }, data: makeShaderNodeData() }],
    });
    const deleteBtn = screen.getByText('Delete');
    fireEvent.click(deleteBtn);
    expect(mockRemoveNode).toHaveBeenCalledWith('n1');
  });

  it('label change calls updateNodeData', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{ id: 'n1', type: 'shader', position: { x: 0, y: 0 }, data: makeShaderNodeData({ label: 'Old' }) }],
    });
    const input = screen.getByDisplayValue('Old');
    fireEvent.change(input, { target: { value: 'NewLabel' } });
    expect(mockUpdateNodeData).toHaveBeenCalledWith('n1', { label: 'NewLabel' });
  });

  it('renders ShaderEditor for shader nodes', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{ id: 'n1', type: 'shader', position: { x: 0, y: 0 }, data: makeShaderNodeData({ type: 'shader' }) }],
    });
    expect(screen.getByTestId('shader-editor')).toBeInTheDocument();
    expect(screen.getByText('Shader Editor')).toBeInTheDocument();
  });

  it('ShaderEditor receives correct code prop', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{ id: 'n1', type: 'shader', position: { x: 0, y: 0 }, data: makeShaderNodeData({ shaderCode: 'custom code' }) }],
    });
    expect(screen.getByTestId('shader-editor').getAttribute('data-code')).toBe('custom code');
  });

  it('does NOT render ShaderEditor for input nodes', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{ id: 'n1', type: 'input', position: { x: 0, y: 0 }, data: makeShaderNodeData({ type: 'input', inputDataType: 'float' }) }],
    });
    expect(screen.queryByTestId('shader-editor')).not.toBeInTheDocument();
  });

  // --- Input node selected ---

  it('renders INPUT type badge for input node', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{ id: 'n1', type: 'input', position: { x: 0, y: 0 }, data: makeShaderNodeData({ type: 'input', inputDataType: 'float' }) }],
    });
    expect(screen.getByText('INPUT')).toBeInTheDocument();
  });

  it('renders PortInspector for input nodes', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{ id: 'n1', type: 'input', position: { x: 0, y: 0 }, data: makeShaderNodeData({ type: 'input' }) }],
    });
    expect(screen.getByTestId('port-inspector')).toBeInTheDocument();
  });

  it('PortInspector receives correct input/output counts', () => {
    const inputs = [makePort('p1', 'u1', 'float', 'input'), makePort('p2', 'u2', 'int', 'input')];
    const outputs = [makePort('p3', 'out1', 'vec4', 'output')];
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{ id: 'n1', type: 'input', position: { x: 0, y: 0 }, data: makeShaderNodeData({ type: 'input', inputs, outputs }) }],
    });
    const inspector = screen.getByTestId('port-inspector');
    expect(inspector.getAttribute('data-inputs')).toBe('2');
    expect(inspector.getAttribute('data-outputs')).toBe('1');
  });

  // --- Output node selected ---

  it('renders OUTPUT type badge for output node', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{ id: 'n1', type: 'output', position: { x: 0, y: 0 }, data: makeShaderNodeData({ type: 'output' }) }],
    });
    expect(screen.getByText('OUTPUT')).toBeInTheDocument();
  });

  it('renders PREVIEW label for output node', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{ id: 'n1', type: 'output', position: { x: 0, y: 0 }, data: makeShaderNodeData({ type: 'output' }) }],
    });
    expect(screen.getByText('PREVIEW')).toBeInTheDocument();
  });

  it('shows "Press Run to preview" when no output preview', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{ id: 'n1', type: 'output', position: { x: 0, y: 0 }, data: makeShaderNodeData({ type: 'output' }) }],
      outputPreviews: {},
    });
    expect(screen.getByText('Press Run to preview')).toBeInTheDocument();
  });

  it('renders preview image when outputPreview exists', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{ id: 'n1', type: 'output', position: { x: 0, y: 0 }, data: makeShaderNodeData({ type: 'output' }) }],
      outputPreviews: { n1: 'data:image/png;base64,abc123' },
    });
    const img = screen.getByAltText('output');
    expect(img).toBeInTheDocument();
    expect(img.getAttribute('src')).toBe('data:image/png;base64,abc123');
  });

  it('renders format select for output node', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{ id: 'n1', type: 'output', position: { x: 0, y: 0 }, data: makeShaderNodeData({ type: 'output' }) }],
    });
    expect(screen.getByText('Format')).toBeInTheDocument();
  });

  it('renders Auto Size checkbox for output node', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{ id: 'n1', type: 'output', position: { x: 0, y: 0 }, data: makeShaderNodeData({ type: 'output' }) }],
    });
    expect(screen.getByText('Auto Size')).toBeInTheDocument();
  });

  it('renders sampling config for output node', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{ id: 'n1', type: 'output', position: { x: 0, y: 0 }, data: makeShaderNodeData({ type: 'output' }) }],
    });
    expect(screen.getByText('SAMPLING')).toBeInTheDocument();
    expect(screen.getByText('Filter')).toBeInTheDocument();
    expect(screen.getByText('Wrap')).toBeInTheDocument();
  });

  it('renders Width/Height inputs for output node when autoSize is false', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{ id: 'n1', type: 'output', position: { x: 0, y: 0 }, data: makeShaderNodeData({ type: 'output', autoSize: false, width: 1024, height: 768 }) }],
    });
    expect(screen.getByDisplayValue('1024')).toBeInTheDocument();
    expect(screen.getByDisplayValue('768')).toBeInTheDocument();
  });

  it('renders disabled Width/Height for output node when autoSize is true', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{ id: 'n1', type: 'output', position: { x: 0, y: 0 }, data: makeShaderNodeData({ type: 'output', autoSize: true, resolvedWidth: 512, resolvedHeight: 512 }) }],
    });
    const widthInputs = screen.getAllByDisplayValue('512');
    expect(widthInputs.length).toBeGreaterThanOrEqual(2);
    expect(widthInputs[0]).toBeDisabled();
  });

  // --- Error display ---

  it('renders error message when nodeError exists', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{ id: 'n1', type: 'shader', position: { x: 0, y: 0 }, data: makeShaderNodeData() }],
      nodeErrors: { n1: 'Compilation failed' },
    });
    expect(screen.getByText('Error')).toBeInTheDocument();
    expect(screen.getByText('Compilation failed')).toBeInTheDocument();
  });

  it('does not render error section when no error', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{ id: 'n1', type: 'shader', position: { x: 0, y: 0 }, data: makeShaderNodeData() }],
      nodeErrors: {},
    });
    expect(screen.queryByText('Error')).not.toBeInTheDocument();
  });

  // --- Sampler2D input node ---

  it('renders PREVIEW section for sampler2D input node', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{
        id: 'n1', type: 'input', position: { x: 0, y: 0 },
        data: makeShaderNodeData({ type: 'input', inputDataType: 'sampler2D', inputMode: 'image' }),
      }],
    });
    expect(screen.getByText('PREVIEW')).toBeInTheDocument();
  });

  it('shows "Load an image" for sampler2D image input without image', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{
        id: 'n1', type: 'input', position: { x: 0, y: 0 },
        data: makeShaderNodeData({ type: 'input', inputDataType: 'sampler2D', inputMode: 'image' }),
      }],
    });
    expect(screen.getByText('Load an image')).toBeInTheDocument();
  });

  it('shows preview image for sampler2D input with imageDataUrl', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{
        id: 'n1', type: 'input', position: { x: 0, y: 0 },
        data: makeShaderNodeData({
          type: 'input',
          inputDataType: 'sampler2D',
          inputMode: 'image',
          imageDataUrl: 'data:image/png;base64,testimage',
        }),
      }],
    });
    const img = screen.getByAltText('preview');
    expect(img).toBeInTheDocument();
  });

  // --- Framebuffer input node ---

  it('renders FRAMEBUFFER CONFIG for framebuffer input', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{
        id: 'n1', type: 'input', position: { x: 0, y: 0 },
        data: makeShaderNodeData({ type: 'input', inputDataType: 'sampler2D', inputMode: 'framebuffer' }),
      }],
    });
    expect(screen.getByText('FRAMEBUFFER CONFIG')).toBeInTheDocument();
  });

  it('shows "Load file and set width/height" for framebuffer without data', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{
        id: 'n1', type: 'input', position: { x: 0, y: 0 },
        data: makeShaderNodeData({ type: 'input', inputDataType: 'sampler2D', inputMode: 'framebuffer' }),
      }],
    });
    expect(screen.getByText('Load file and set width/height')).toBeInTheDocument();
  });

  it('renders sampling config for sampler2D input', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{
        id: 'n1', type: 'input', position: { x: 0, y: 0 },
        data: makeShaderNodeData({ type: 'input', inputDataType: 'sampler2D', inputMode: 'image' }),
      }],
    });
    expect(screen.getByText('SAMPLING')).toBeInTheDocument();
  });

  // --- Image dimensions ---

  it('renders image dimensions for sampler2D image input with width/height', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{
        id: 'n1', type: 'input', position: { x: 0, y: 0 },
        data: makeShaderNodeData({
          type: 'input',
          inputDataType: 'sampler2D',
          inputMode: 'image',
          imageWidth: 1920,
          imageHeight: 1080,
          imageDataUrl: 'data:image/png;base64,test',
        }),
      }],
    });
    expect(screen.getByText('1920 × 1080')).toBeInTheDocument();
  });

  // --- Aside element ---

  it('renders as aside element', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{ id: 'n1', type: 'shader', position: { x: 0, y: 0 }, data: makeShaderNodeData() }],
    });
    const aside = document.querySelector('aside');
    expect(aside).toBeInTheDocument();
  });

  // --- Lightbox interaction ---

  it('clicking output preview image opens lightbox', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{ id: 'n1', type: 'output', position: { x: 0, y: 0 }, data: makeShaderNodeData({ type: 'output' }) }],
      outputPreviews: { n1: 'data:image/png;base64,preview123' },
    });
    const img = screen.getByAltText('output');
    fireEvent.click(img);
    expect(screen.getByTestId('lightbox')).toBeInTheDocument();
  });

  it('closing lightbox removes it', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{ id: 'n1', type: 'output', position: { x: 0, y: 0 }, data: makeShaderNodeData({ type: 'output' }) }],
      outputPreviews: { n1: 'data:image/png;base64,preview123' },
    });
    const img = screen.getByAltText('output');
    fireEvent.click(img);
    const closeBtn = screen.getByTestId('lightbox-close');
    fireEvent.click(closeBtn);
    expect(screen.queryByTestId('lightbox')).not.toBeInTheDocument();
  });

  // --- Format/filter/wrap selects for output node ---

  it('format select defaults to rgba8', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{ id: 'n1', type: 'output', position: { x: 0, y: 0 }, data: makeShaderNodeData({ type: 'output' }) }],
    });
    const selects = document.querySelectorAll('select');
    const formatSelect = selects[0] as HTMLSelectElement;
    expect(formatSelect.value).toBe('rgba8');
  });

  it('format select contains expected options', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{ id: 'n1', type: 'output', position: { x: 0, y: 0 }, data: makeShaderNodeData({ type: 'output' }) }],
    });
    expect(screen.getByText('RGBA8')).toBeInTheDocument();
    expect(screen.getByText('RGBA32F')).toBeInTheDocument();
  });

  it('filter select defaults to linear', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{ id: 'n1', type: 'output', position: { x: 0, y: 0 }, data: makeShaderNodeData({ type: 'output' }) }],
    });
    expect(screen.getByText('LINEAR')).toBeInTheDocument();
    expect(screen.getByText('NEAREST')).toBeInTheDocument();
  });

  it('wrap select defaults to clamp', () => {
    renderSidePanel({
      selectedNodeId: 'n1',
      nodes: [{ id: 'n1', type: 'output', position: { x: 0, y: 0 }, data: makeShaderNodeData({ type: 'output' }) }],
    });
    expect(screen.getByText('CLAMP')).toBeInTheDocument();
    expect(screen.getByText('REPEAT')).toBeInTheDocument();
    expect(screen.getByText('MIRROR')).toBeInTheDocument();
  });
});
