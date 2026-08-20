import { describe, expect, it } from 'vitest';
import type { Edge, Node } from '@xyflow/react';
import type { ShaderNodeData } from '../src/types';
import {
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
});
