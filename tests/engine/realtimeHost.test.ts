import { describe, it, expect } from 'vitest';
import type { Node } from '@xyflow/react';
import type { ShaderNodeData } from '../../src/types';
import { isStaticPipeline } from '../../src/engine/realtimeHost';

/**
 * Minimal node factory — only the fields isStaticPipeline inspects:
 *   data.type, data.inputMode, data.systemSource, data.shaderCode
 */
function node(data: Partial<ShaderNodeData>): Node<ShaderNodeData> {
  return {
    id: 'n',
    position: { x: 0, y: 0 },
    data: {
      type: 'shader',
      label: '',
      shaderCode: '',
      inputs: [],
      outputs: [],
      uniforms: {},
      ...data,
    },
  };
}

describe('isStaticPipeline', () => {
  it('returns true for an empty node array', () => {
    expect(isStaticPipeline([])).toBe(true);
  });

  describe('shader / constant nodes — dynamic builtins', () => {
    const staticCode = 'void main(){ gl_FragColor = vec4(1.0); }';

    it.each([
      { nodeType: 'shader' as const, code: staticCode },
      { nodeType: 'constant' as const, code: staticCode },
    ])('returns true for $nodeType node without dynamic builtins', ({ nodeType, code }) => {
      expect(isStaticPipeline([node({ type: nodeType, shaderCode: code })])).toBe(true);
    });

    it.each([
      { builtin: 'iTime',      code: 'float t = iTime;' },
      { builtin: 'iMouse',     code: 'vec4 m = iMouse;' },
      { builtin: 'iTimeDelta', code: 'float d = iTimeDelta;' },
      { builtin: 'iFrame',     code: 'int f = iFrame;' },
    ])('returns false for shader node referencing $builtin', ({ code }) => {
      expect(isStaticPipeline([node({ type: 'shader', shaderCode: code })])).toBe(false);
    });

    it.each([
      { builtin: 'iTime',      code: 'float t = iTime;' },
      { builtin: 'iTimeDelta', code: 'float d = iTimeDelta;' },
      { builtin: 'iFrame',     code: 'int f = iFrame;' },
      { builtin: 'iMouse',     code: 'vec4 m = iMouse;' },
    ])('returns false for constant node referencing $builtin', ({ code }) => {
      expect(isStaticPipeline([node({ type: 'constant', shaderCode: code })])).toBe(false);
    });

    it('returns true for shader node referencing iResolution (static builtin)', () => {
      expect(
        isStaticPipeline([node({ type: 'shader', shaderCode: 'vec3 r = iResolution;' })]),
      ).toBe(true);
    });

    it('does not false-positive on substrings like "iTimekeeper"', () => {
      expect(
        isStaticPipeline([node({ type: 'shader', shaderCode: 'float x = iTimekeeper;' })]),
      ).toBe(true);
    });

    it('returns false for shader node referencing previousFrame (feedback)', () => {
      expect(
        isStaticPipeline([node({ type: 'shader', shaderCode: 'texture(previousFrame, v_uv)' })]),
      ).toBe(false);
    });

    it('returns false for constant node referencing previousFrame', () => {
      expect(
        isStaticPipeline([node({ type: 'constant', shaderCode: 'texture(previousFrame, v_uv)' })]),
      ).toBe(false);
    });
  });

  describe('input nodes — video', () => {
    it('returns false for a video input node', () => {
      expect(
        isStaticPipeline([node({ type: 'input', inputMode: 'video' })]),
      ).toBe(false);
    });
  });

  describe('input nodes — system source', () => {
    it.each([
      { source: 'time' as const },
      { source: 'mouse' as const },
    ])('returns false for system input with source=$source', ({ source }) => {
      expect(
        isStaticPipeline([node({ type: 'input', inputMode: 'system', systemSource: source })]),
      ).toBe(false);
    });

    it('returns true for system input with source=resolution', () => {
      expect(
        isStaticPipeline([
          node({ type: 'input', inputMode: 'system', systemSource: 'resolution' }),
        ]),
      ).toBe(true);
    });

    it.each([
      { source: 'timeDelta' as const },
      { source: 'frame' as const },
    ])('returns false for system input with source=$source', ({ source }) => {
      expect(
        isStaticPipeline([node({ type: 'input', inputMode: 'system', systemSource: source })]),
      ).toBe(false);
    });
  });

  describe('input nodes — image (static)', () => {
    it('returns true for an image input node', () => {
      expect(
        isStaticPipeline([node({ type: 'input', inputMode: 'image' })]),
      ).toBe(true);
    });
  });

  describe('non-shader node types', () => {
    it('returns true for an onnx node', () => {
      expect(isStaticPipeline([node({ type: 'onnx' })])).toBe(true);
    });

    it('returns true for a renderer node', () => {
      expect(isStaticPipeline([node({ type: 'renderer' })])).toBe(true);
    });
  });

  describe('mixed pipelines', () => {
    it('returns true when shader has no dynamic builtins and input is image', () => {
      expect(
        isStaticPipeline([
          node({ type: 'shader', shaderCode: 'void main(){ gl_FragColor = texture2D(iChannel0, vUv); }' }),
          node({ type: 'input', inputMode: 'image' }),
        ]),
      ).toBe(true);
    });

    it('returns false when any shader uses iTime, even alongside static nodes', () => {
      expect(
        isStaticPipeline([
          node({ type: 'shader', shaderCode: 'float t = iTime;' }),
          node({ type: 'input', inputMode: 'image' }),
          node({ type: 'renderer' }),
        ]),
      ).toBe(false);
    });

    it('returns false when one of many nodes is a video input', () => {
      expect(
        isStaticPipeline([
          node({ type: 'shader', shaderCode: 'void main(){}' }),
          node({ type: 'input', inputMode: 'video' }),
          node({ type: 'onnx' }),
        ]),
      ).toBe(false);
    });
  });
});
