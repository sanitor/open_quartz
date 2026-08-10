import { describe, expect, it } from 'vitest';
import type { Edge, Node } from '@xyflow/react';
import type { ProjectFile, ShaderNodeData } from '../src/types';
import {
  collectScreenSaverGraph,
  prepareScreenSaverGraph,
  rendererCandidates,
  screenSaverInputCandidates,
} from '../src/screensaver';

function node(id: string, data: Partial<ShaderNodeData>): Node<ShaderNodeData> {
  return {
    id,
    type: data.type ?? 'shader',
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

const image = node('image', {
  type: 'input',
  inputMode: 'image',
  inputDataType: 'sampler2D',
  imageDataUrl: 'data:image/png;base64,AA==',
});
const video = node('video', {
  type: 'input',
  inputMode: 'video',
  inputDataType: 'sampler2D',
  videoFilePath: 'C:\\media\\clip.mp4',
  videoFileName: 'clip.mp4',
});
const shader = node('shader', {
  type: 'shader',
  outputs: [{ id: 'shader-out', label: 'color', dataType: 'sampler2D', direction: 'output' }],
});
const renderer = node('renderer', {
  type: 'renderer',
  inputs: [{ id: 'renderer-in', label: 'inputTexture', dataType: 'sampler2D', direction: 'input' }],
});
const edge: Edge = {
  id: 'renderer-edge',
  source: 'shader',
  sourceHandle: 'shader-out',
  target: 'renderer',
  targetHandle: 'renderer-in',
};

function project(nodes: Node<ShaderNodeData>[], edges: Edge[]): ProjectFile {
  return {
    version: '0.4.0',
    name: 'Saver',
    createdAt: '2026-01-01T00:00:00.000Z',
    updatedAt: '2026-01-01T00:00:00.000Z',
    graph: {
      nodes: nodes.map((item) => ({
        id: item.id,
        type: item.type ?? 'shader',
        position: item.position,
        data: item.data,
      })),
      edges: edges.map((item) => ({
        id: item.id,
        source: item.source,
        sourceHandle: item.sourceHandle ?? '',
        target: item.target,
        targetHandle: item.targetHandle ?? '',
      })),
    },
  };
}

describe('screen saver export graph', () => {
  it('offers image and video inputs to the control panel', () => {
    const system = node('time', { type: 'input', inputMode: 'system', inputDataType: 'float' });
    expect(screenSaverInputCandidates([image, video, system])).toEqual([
      { nodeId: 'image', label: 'image', kind: 'image' },
      { nodeId: 'video', label: 'video', kind: 'video' },
    ]);
  });

  it('reports whether each Renderer is connected', () => {
    const disconnected = node('renderer-2', { type: 'renderer' });
    expect(rendererCandidates([renderer, disconnected], [edge])).toEqual([
      { id: 'renderer', label: 'renderer', connected: true },
      { id: 'renderer-2', label: 'renderer-2', connected: false },
    ]);
  });

  it('retains only the selected Renderer and its ancestors', () => {
    const sourceEdge: Edge = { id: 'source-edge', source: 'image', target: 'shader' };
    const result = collectScreenSaverGraph(
      project([image, video, shader, renderer], [sourceEdge, edge]),
      'renderer',
    );
    expect(result.graph.nodes.map((item) => item.id)).toEqual(['image', 'shader', 'renderer']);
    expect(result.graph.edges.map((item) => item.id)).toEqual(['source-edge', 'renderer-edge']);
  });

  it('keeps file-backed videos as path references', () => {
    const videoEdge: Edge = { ...edge, source: 'video' };
    const result = collectScreenSaverGraph(project([video, renderer], [videoEdge]), 'renderer');
    expect(result.graph.nodes.find((item) => item.id === 'video')?.data.videoFilePath)
      .toBe('C:\\media\\clip.mp4');
  });

  it('inserts a final resample pass at the display resolution', () => {
    const result = prepareScreenSaverGraph([shader, renderer], [edge], 'renderer', 3840, 2160);
    const output = result.nodes.find((item) => item.id === '__screen_saver_output_resample');
    expect(output?.data).toMatchObject({ autoSize: false, width: 3840, height: 2160 });
    expect(result.edges.some((item) => item.source === 'shader' && item.target === output?.id)).toBe(true);
    expect(result.edges.some((item) => item.source === output?.id && item.target === 'renderer')).toBe(true);
    expect(result.edges.some((item) => item.id === 'renderer-edge')).toBe(false);
  });
});
