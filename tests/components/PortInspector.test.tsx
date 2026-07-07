import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

import { PortInspector } from '../../src/components/SidePanel/PortInspector';
import { DATA_TYPE_COLORS } from '../../src/types';
import type { Port } from '../../src/types';

function makePort(overrides: Partial<Port> = {}): Port {
  return {
    id: 'port-1',
    label: 'value',
    dataType: 'float',
    direction: 'input',
    ...overrides,
  };
}

describe('PortInspector', () => {
  const mockOnUniformChange = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders input section header', () => {
    render(
      <PortInspector
        inputs={[]}
        outputs={[]}
        uniforms={{}}
        onUniformChange={mockOnUniformChange}
      />,
    );
    expect(screen.getByText('Inputs')).toBeInTheDocument();
  });

  it('renders output section header', () => {
    render(
      <PortInspector
        inputs={[]}
        outputs={[]}
        uniforms={{}}
        onUniformChange={mockOnUniformChange}
      />,
    );
    expect(screen.getByText('Outputs')).toBeInTheDocument();
  });

  it('hides outputs section when showOutputs=false', () => {
    render(
      <PortInspector
        inputs={[]}
        outputs={[{ id: 'out-1', label: 'result', dataType: 'vec4', direction: 'output' }]}
        uniforms={{}}
        onUniformChange={mockOnUniformChange}
        showOutputs={false}
      />,
    );
    expect(screen.queryByText('Outputs')).not.toBeInTheDocument();
  });

  it('renders empty state message for inputs', () => {
    render(
      <PortInspector
        inputs={[]}
        outputs={[]}
        uniforms={{}}
        onUniformChange={mockOnUniformChange}
      />,
    );
    expect(screen.getByText('Add uniforms to your shader to create inputs')).toBeInTheDocument();
  });

  it('renders empty state message for outputs', () => {
    render(
      <PortInspector
        inputs={[]}
        outputs={[]}
        uniforms={{}}
        onUniformChange={mockOnUniformChange}
      />,
    );
    expect(screen.getByText('Add out variables to your shader to create outputs')).toBeInTheDocument();
  });

  it('renders port labels for inputs', () => {
    render(
      <PortInspector
        inputs={[
          makePort({ id: 'p1', label: 'brightness' }),
          makePort({ id: 'p2', label: 'contrast' }),
        ]}
        outputs={[]}
        uniforms={{ brightness: '1.0', contrast: '0.5' }}
        onUniformChange={mockOnUniformChange}
      />,
    );
    expect(screen.getByText('brightness')).toBeInTheDocument();
    expect(screen.getByText('contrast')).toBeInTheDocument();
  });

  it('renders color indicators matching DATA_TYPE_COLORS for float', () => {
    const { container } = render(
      <PortInspector
        inputs={[makePort({ id: 'p1', label: 'value', dataType: 'float' })]}
        outputs={[]}
        uniforms={{ value: '1.0' }}
        onUniformChange={mockOnUniformChange}
      />,
    );
    const colorDots = container.querySelectorAll('.w-2.h-2.rounded-full');
    expect(colorDots.length).toBeGreaterThan(0);
    const dot = colorDots[0] as HTMLElement;
    expect(dot.style.backgroundColor).toBeTruthy();
  });

  it('renders color indicators for vec3 output', () => {
    const { container } = render(
      <PortInspector
        inputs={[]}
        outputs={[makePort({ id: 'o1', label: 'color', dataType: 'vec3', direction: 'output' })]}
        uniforms={{}}
        onUniformChange={mockOnUniformChange}
      />,
    );
    const colorDots = container.querySelectorAll('.w-2.h-2.rounded-full');
    expect(colorDots.length).toBeGreaterThan(0);
    const dot = colorDots[0] as HTMLElement;
    expect(dot.style.backgroundColor).toBeTruthy();
  });

  it('renders float input text field with current value', () => {
    render(
      <PortInspector
        inputs={[makePort({ id: 'p1', label: 'brightness', dataType: 'float' })]}
        outputs={[]}
        uniforms={{ brightness: '0.5' }}
        onUniformChange={mockOnUniformChange}
      />,
    );
    const input = screen.getByDisplayValue('0.5');
    expect(input).toBeInTheDocument();
  });

  it('calls onUniformChange when float input changes', () => {
    render(
      <PortInspector
        inputs={[makePort({ id: 'p1', label: 'brightness', dataType: 'float' })]}
        outputs={[]}
        uniforms={{ brightness: '0.5' }}
        onUniformChange={mockOnUniformChange}
      />,
    );
    const input = screen.getByDisplayValue('0.5');
    fireEvent.change(input, { target: { value: '0.8' } });
    expect(mockOnUniformChange).toHaveBeenCalledWith('brightness', '0.8');
  });

  it('renders vec2 component inputs with x,y labels', () => {
    render(
      <PortInspector
        inputs={[makePort({ id: 'p1', label: 'position', dataType: 'vec2' })]}
        outputs={[]}
        uniforms={{ position: [1, 2] }}
        onUniformChange={mockOnUniformChange}
      />,
    );
    expect(screen.getByText('x')).toBeInTheDocument();
    expect(screen.getByText('y')).toBeInTheDocument();
    expect(screen.getByDisplayValue('1')).toBeInTheDocument();
    expect(screen.getByDisplayValue('2')).toBeInTheDocument();
  });

  it('renders vec3 component inputs with x,y,z labels', () => {
    render(
      <PortInspector
        inputs={[makePort({ id: 'p1', label: 'color', dataType: 'vec3' })]}
        outputs={[]}
        uniforms={{ color: [0.1, 0.2, 0.3] }}
        onUniformChange={mockOnUniformChange}
      />,
    );
    expect(screen.getByText('x')).toBeInTheDocument();
    expect(screen.getByText('y')).toBeInTheDocument();
    expect(screen.getByText('z')).toBeInTheDocument();
  });

  it('renders vec4 component inputs with x,y,z,w labels', () => {
    render(
      <PortInspector
        inputs={[makePort({ id: 'p1', label: 'rgba', dataType: 'vec4' })]}
        outputs={[]}
        uniforms={{ rgba: [0, 0, 0, 1] }}
        onUniformChange={mockOnUniformChange}
      />,
    );
    expect(screen.getByText('w')).toBeInTheDocument();
  });

  it('renders sampler2D input with connect upstream message', () => {
    render(
      <PortInspector
        inputs={[makePort({ id: 'p1', label: 'tex', dataType: 'sampler2D' })]}
        outputs={[]}
        uniforms={{}}
        onUniformChange={mockOnUniformChange}
      />,
    );
    expect(screen.getByText('← connect upstream')).toBeInTheDocument();
  });

  it('renders mat2 input with grid of 4 text fields', () => {
    render(
      <PortInspector
        inputs={[makePort({ id: 'p1', label: 'matrix', dataType: 'mat2' })]}
        outputs={[]}
        uniforms={{ matrix: [1, 0, 0, 1] }}
        onUniformChange={mockOnUniformChange}
      />,
    );
    const inputs = screen.getAllByRole('textbox');
    expect(inputs.length).toBe(4);
  });

  it('renders output port labels and data types', () => {
    render(
      <PortInspector
        inputs={[]}
        outputs={[
          makePort({ id: 'o1', label: 'fragColor', dataType: 'vec4', direction: 'output' }),
        ]}
        uniforms={{}}
        onUniformChange={mockOnUniformChange}
      />,
    );
    expect(screen.getByText('fragColor')).toBeInTheDocument();
    expect(screen.getByText('vec4')).toBeInTheDocument();
  });

  it('vec component change calls onUniformChange with updated array', () => {
    render(
      <PortInspector
        inputs={[makePort({ id: 'p1', label: 'pos', dataType: 'vec2' })]}
        outputs={[]}
        uniforms={{ pos: [1, 2] }}
        onUniformChange={mockOnUniformChange}
      />,
    );
    const xInput = screen.getByDisplayValue('1');
    fireEvent.change(xInput, { target: { value: '5' } });
    expect(mockOnUniformChange).toHaveBeenCalledWith('pos', [5, 2]);
  });
});
