import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { Edge, Node } from '@xyflow/react';
import type { ProjectFile, ShaderNodeData } from '../../src/types';
import { ScreenSaverExportDialog } from '../../src/components/ScreenSaverExportDialog';

const invoke = vi.fn();
const save = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ save }));

function makeNode(id: string, data: Partial<ShaderNodeData>): Node<ShaderNodeData> {
  return {
    id,
    type: data.type,
    position: { x: 0, y: 0 },
    data: {
      type: data.type ?? 'shader',
      label: id,
      shaderCode: '',
      inputs: [],
      outputs: [],
      uniforms: {},
      ...data,
    },
  };
}

const nodes = [
  makeNode('image', { type: 'input', inputMode: 'image', inputDataType: 'sampler2D' }),
  makeNode('video', { type: 'input', inputMode: 'video', inputDataType: 'sampler2D' }),
  makeNode('renderer-a', { type: 'renderer' }),
  makeNode('renderer-b', { type: 'renderer' }),
];
const edges: Edge[] = [
  { id: 'a', source: 'image', target: 'renderer-a' },
  { id: 'b', source: 'video', target: 'renderer-b' },
];
const project: ProjectFile = {
  version: '0.4.0',
  name: 'Gallery',
  createdAt: '2026-01-01T00:00:00.000Z',
  updatedAt: '2026-01-01T00:00:00.000Z',
  graph: {
    nodes: nodes.map((node) => ({
      id: node.id,
      type: node.type ?? 'shader',
      position: node.position,
      data: node.data,
    })),
    edges: edges.map((edge) => ({
      id: edge.id,
      source: edge.source,
      sourceHandle: edge.sourceHandle ?? '',
      target: edge.target,
      targetHandle: edge.targetHandle ?? '',
    })),
  },
};

describe('ScreenSaverExportDialog', () => {
  beforeEach(() => {
    invoke.mockReset().mockResolvedValue(undefined);
    save.mockReset().mockResolvedValue('C:\\Exports\\Gallery.scr');
  });

  it('exports the chosen Renderer and only checked control-panel inputs', async () => {
    render(
      <ScreenSaverExportDialog
        nodes={nodes}
        edges={edges}
        project={project}
        activeRendererId="renderer-a"
        onClose={vi.fn()}
      />,
    );

    expect(screen.getAllByText('image')).toHaveLength(2);
    expect(screen.getAllByText('video')).toHaveLength(2);
    fireEvent.click(screen.getAllByRole('checkbox')[0]);
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'renderer-b' } });
    fireEvent.click(screen.getByText('EXPORT .SCR'));

    await waitFor(() => expect(invoke).toHaveBeenCalledWith(
      'screen_saver_export',
      {
        request: {
          outputPath: 'C:\\Exports\\Gallery.scr',
          name: 'Gallery',
          projectJson: expect.any(String),
          rendererNodeId: 'renderer-b',
          exposedInputs: [{ nodeId: 'video', label: 'video', kind: 'video' }],
        },
      },
    ));
  });
});
