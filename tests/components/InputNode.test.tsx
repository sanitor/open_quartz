import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

vi.mock('@xyflow/react', () => ({
  Handle: ({ type, position, id }: { type: string; position: string; id: string }) => (
    <span data-testid={`handle-${type}-${id}`} data-position={position} />
  ),
  Position: { Left: 'left', Right: 'right', Top: 'top', Bottom: 'bottom' },
}));

const mockUpdateNodeData = vi.fn();

vi.mock('../../src/store/useGraphStore', () => ({
  useGraphStore: vi.fn((selector: unknown) => {
    if (typeof selector === 'function') {
      const state = {
        updateNodeData: mockUpdateNodeData,
        nodeErrors: {} as Record<string, string>,
      };
      return (selector as (s: typeof state) => unknown)(state);
    }
    return {};
  }),
}));

vi.mock('../../src/utils/rawPreview', () => ({
  generateRawPreview: vi.fn(() => 'data:image/png;base64,mockpreview'),
}));

import { InputNode } from '../../src/components/NodeGraph/nodes/InputNode';
import type { ShaderNodeData, Port } from '../../src/types';
import { useGraphStore } from '../../src/store/useGraphStore';

function makePort(overrides: Partial<Port> = {}): Port {
  return {
    id: 'port-1',
    label: 'value',
    dataType: 'float',
    direction: 'input',
    ...overrides,
  };
}

function makeNodeProps(dataOverrides: Partial<ShaderNodeData> = {}) {
  const data: ShaderNodeData = {
    type: 'input',
    label: 'TestInput',
    shaderCode: '',
    inputs: [makePort()],
    outputs: [{ id: 'out-1', label: 'output', dataType: 'float', direction: 'output' as const }],
    uniforms: {},
    ...dataOverrides,
  };
  return {
    id: 'node-1',
    data,
    selected: false,
    type: 'input' as const,
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

describe('InputNode', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders float input with text field', () => {
    const props = makeNodeProps({ inputDataType: 'float' });
    render(<InputNode {...props} />);
    expect(screen.getByText('FLOAT')).toBeInTheDocument();
    const input = screen.getByPlaceholderText('—');
    expect(input).toBeInTheDocument();
  });

  it('renders bool input header', () => {
    const props = makeNodeProps({
      inputDataType: 'bool',
      inputs: [makePort({ dataType: 'bool', label: 'enabled' })],
    });
    render(<InputNode {...props} />);
    expect(screen.getByText('BOOL')).toBeInTheDocument();
  });

  it('renders vec2 input with x,y component fields', () => {
    const props = makeNodeProps({
      inputDataType: 'vec2',
      inputs: [makePort({ dataType: 'vec2', label: 'position' })],
      uniforms: { position: [1, 2] },
    });
    render(<InputNode {...props} />);
    expect(screen.getByText('VEC2')).toBeInTheDocument();
    expect(screen.getByText('x')).toBeInTheDocument();
    expect(screen.getByText('y')).toBeInTheDocument();
  });

  it('renders vec3 input with x,y,z fields', () => {
    const props = makeNodeProps({
      inputDataType: 'vec3',
      inputs: [makePort({ dataType: 'vec3', label: 'color' })],
      uniforms: { color: [0.1, 0.2, 0.3] },
    });
    render(<InputNode {...props} />);
    expect(screen.getByText('VEC3')).toBeInTheDocument();
    expect(screen.getByText('z')).toBeInTheDocument();
  });

  it('renders vec4 input with x,y,z,w fields', () => {
    const props = makeNodeProps({
      inputDataType: 'vec4',
      inputs: [makePort({ dataType: 'vec4', label: 'color4' })],
      uniforms: { color4: [0, 0, 0, 1] },
    });
    render(<InputNode {...props} />);
    expect(screen.getByText('VEC4')).toBeInTheDocument();
    expect(screen.getByText('w')).toBeInTheDocument();
  });

  it('renders sampler2D image mode with empty placeholder', () => {
    const props = makeNodeProps({
      inputDataType: 'sampler2D',
      inputMode: 'image',
      inputs: [],
      outputs: [{ id: 'out-1', label: 'texture', dataType: 'sampler2D', direction: 'output' as const }],
    });
    render(<InputNode {...props} />);
    expect(screen.getByText('IMAGE')).toBeInTheDocument();
    expect(screen.getByText('Click to load image')).toBeInTheDocument();
  });

  it('renders sampler2D image mode with loaded image', () => {
    const props = makeNodeProps({
      inputDataType: 'sampler2D',
      inputMode: 'image',
      imageDataUrl: 'data:image/png;base64,abc',
      imageFileName: 'test.png',
      inputs: [],
      outputs: [{ id: 'out-1', label: 'texture', dataType: 'sampler2D', direction: 'output' as const }],
    });
    render(<InputNode {...props} />);
    expect(screen.getByText('IMAGE')).toBeInTheDocument();
    expect(screen.getByText('test.png')).toBeInTheDocument();
  });

  it('renders sampler2D framebuffer mode with placeholder when no data', () => {
    const props = makeNodeProps({
      inputDataType: 'sampler2D',
      inputMode: 'framebuffer',
      inputs: [],
      outputs: [{ id: 'out-1', label: 'texture', dataType: 'sampler2D', direction: 'output' as const }],
    });
    render(<InputNode {...props} />);
    expect(screen.getByText('FRAMEBUFFER')).toBeInTheDocument();
    expect(screen.getByText('Click to load raw file')).toBeInTheDocument();
  });

  it('renders framebuffer mode with loaded raw data and dimensions', () => {
    const props = makeNodeProps({
      inputDataType: 'sampler2D',
      inputMode: 'framebuffer',
      rawDataUrl: 'data:application/octet-stream;base64,abc',
      rawFileName: 'frame.bin',
      fbFormat: 'rgba8',
      fbWidth: 640,
      fbHeight: 480,
      inputs: [],
      outputs: [{ id: 'out-1', label: 'texture', dataType: 'sampler2D', direction: 'output' as const }],
    });
    render(<InputNode {...props} />);
    expect(screen.getByText('FRAMEBUFFER')).toBeInTheDocument();
    expect(screen.getByText('frame.bin')).toBeInTheDocument();
    expect(screen.getByText('RGBA8 640×480')).toBeInTheDocument();
  });

  it('renders output handle for scalar types', () => {
    const props = makeNodeProps({ inputDataType: 'float' });
    render(<InputNode {...props} />);
    expect(screen.getByTestId('handle-source-out-1')).toBeInTheDocument();
  });

  it('renders header with blue accent when value present', () => {
    const props = makeNodeProps({
      inputDataType: 'float',
      uniforms: { value: '1.0' },
    });
    const { container } = render(<InputNode {...props} />);
    const header = container.querySelector('[style*="background-color"]');
    expect(header).toBeTruthy();
  });

  it('renders mat2 input with grid of fields', () => {
    const props = makeNodeProps({
      inputDataType: 'mat2',
      inputs: [makePort({ dataType: 'mat2', label: 'matrix' })],
      uniforms: { matrix: [1, 0, 0, 1] },
    });
    render(<InputNode {...props} />);
    expect(screen.getByText('MAT2')).toBeInTheDocument();
    // mat2 = 2x2 = 4 input fields
    const inputs = screen.getAllByRole('textbox');
    expect(inputs.length).toBe(4);
  });


  it('renders gray accent when sampler2D with no data', () => {
    const props = makeNodeProps({
      inputDataType: 'sampler2D',
      inputs: [],
      outputs: [makePort({ dataType: 'sampler2D', label: 'out', direction: 'output' })],
    });
    const { container } = render(<InputNode {...props} />);
    const header = container.querySelector('[style*="background-color"]') as HTMLElement;
    expect(header?.style.backgroundColor).toBe('rgb(142, 142, 147)');
  });

  it('renders ivec3 input with x,y,z fields', () => {
    const props = makeNodeProps({
      inputDataType: 'ivec3',
      inputs: [makePort({ dataType: 'ivec3', label: 'value' })],
      uniforms: { value: [1, 2, 3] },
    });
    render(<InputNode {...props} />);
    expect(screen.getByText('IVEC3')).toBeInTheDocument();
    const inputs = screen.getAllByRole('textbox');
    expect(inputs.length).toBe(3);
  });

  it('renders selected node with blue border', () => {
    const props = makeNodeProps({ inputDataType: 'float', inputs: [makePort({ dataType: 'float', label: 'val' })] });
    props.selected = true;
    const { container } = render(<InputNode {...props} />);
    const wrapper = container.firstElementChild;
    expect(wrapper?.className).toContain('border-[#007aff]');
  });

  it('renders scalar type label in header', () => {
    const props = makeNodeProps({
      inputDataType: 'int',
      inputs: [makePort({ dataType: 'int', label: 'count' })],
    });
    render(<InputNode {...props} />);
    expect(screen.getByText('INT')).toBeInTheDocument();
  });

  it('renders framebuffer header text', () => {
    const props = makeNodeProps({
      inputDataType: 'sampler2D',
      inputMode: 'framebuffer',
      inputs: [],
      outputs: [makePort({ dataType: 'sampler2D', label: 'out', direction: 'output' })],
    });
    render(<InputNode {...props} />);
    expect(screen.getByText('FRAMEBUFFER')).toBeInTheDocument();
  });

  it('renders framebuffer with preview when fbPreview is generated', () => {
    const props = makeNodeProps({
      inputDataType: 'sampler2D',
      inputMode: 'framebuffer',
      rawDataUrl: 'data:application/octet-stream;base64,AAAA',
      rawFileName: 'test.raw',
      fbFormat: 'rgba8',
      fbWidth: 2,
      fbHeight: 2,
      inputs: [],
      outputs: [makePort({ dataType: 'sampler2D', label: 'out', direction: 'output' })],
    });
    render(<InputNode {...props} />);
    // Preview generated by mock
    const previewImg = screen.getByAltText('preview');
    expect(previewImg).toBeInTheDocument();
  });

  it('renders image data with filename', () => {
    const props = makeNodeProps({
      inputDataType: 'sampler2D',
      imageDataUrl: 'data:image/png;base64,abc',
      imageFileName: 'photo.png',
      inputs: [],
      outputs: [makePort({ dataType: 'sampler2D', label: 'out', direction: 'output' })],
    });
    render(<InputNode {...props} />);
    expect(screen.getByText('photo.png')).toBeInTheDocument();
  });
});
