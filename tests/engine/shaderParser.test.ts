import { describe, it, expect, beforeEach } from 'vitest';
import { parseShader } from '../../src/engine/shaderParser';
import type { Port } from '../../src/types';

describe('parseShader', () => {
  // Reset regex state is handled internally (lastIndex = 0), but we keep tests independent.

  describe('uniform parsing', () => {
    it('parses a single uniform float', () => {
      const result = parseShader('uniform float x;');
      expect(result.inputs).toHaveLength(1);
      expect(result.inputs[0].label).toBe('x');
      expect(result.inputs[0].dataType).toBe('float');
      expect(result.inputs[0].direction).toBe('input');
      expect(result.inputs[0].id).toMatch(/^port_\d+_\d+$/);
    });

    it('parses multiple uniforms of different types', () => {
      const code = [
        'uniform vec2 pos;',
        'uniform vec3 color;',
        'uniform vec4 rgba;',
        'uniform mat4 transform;',
        'uniform sampler2D tex;',
        'uniform samplerCube cube;',
        'uniform int count;',
        'uniform uint flags;',
        'uniform bool enabled;',
      ].join('\n');

      const result = parseShader(code);
      expect(result.inputs).toHaveLength(9);

      const typeMap = new Map(result.inputs.map(p => [p.label, p.dataType]));
      expect(typeMap.get('pos')).toBe('vec2');
      expect(typeMap.get('color')).toBe('vec3');
      expect(typeMap.get('rgba')).toBe('vec4');
      expect(typeMap.get('transform')).toBe('mat4');
      expect(typeMap.get('tex')).toBe('sampler2D');
      expect(typeMap.get('cube')).toBe('samplerCube');
      expect(typeMap.get('count')).toBe('int');
      expect(typeMap.get('flags')).toBe('uint');
      expect(typeMap.get('enabled')).toBe('bool');
    });

    it('parses ivec/uvec/bvec/mat variants', () => {
      const code = [
        'uniform ivec2 a;',
        'uniform ivec3 b;',
        'uniform ivec4 c;',
        'uniform uvec2 d;',
        'uniform uvec3 e;',
        'uniform uvec4 f;',
        'uniform bvec2 g;',
        'uniform bvec3 h;',
        'uniform bvec4 i;',
        'uniform mat2 j;',
        'uniform mat3 k;',
      ].join('\n');

      const result = parseShader(code);
      expect(result.inputs).toHaveLength(11);

      const typeMap = new Map(result.inputs.map(p => [p.label, p.dataType]));
      expect(typeMap.get('a')).toBe('ivec2');
      expect(typeMap.get('b')).toBe('ivec3');
      expect(typeMap.get('c')).toBe('ivec4');
      expect(typeMap.get('d')).toBe('uvec2');
      expect(typeMap.get('e')).toBe('uvec3');
      expect(typeMap.get('f')).toBe('uvec4');
      expect(typeMap.get('g')).toBe('bvec2');
      expect(typeMap.get('h')).toBe('bvec3');
      expect(typeMap.get('i')).toBe('bvec4');
      expect(typeMap.get('j')).toBe('mat2');
      expect(typeMap.get('k')).toBe('mat3');
    });

    it('parses uniform with default value', () => {
      const result = parseShader('uniform float x = 1.0;');
      expect(result.inputs).toHaveLength(1);
      expect(result.inputs[0].label).toBe('x');
      expect(result.inputs[0].dataType).toBe('float');
      expect(result.inputs[0].defaultValue).toBe('1.0');
    });

    it('parses uniform with complex default value', () => {
      const result = parseShader('uniform vec3 color = vec3(1.0, 0.0, 0.5);');
      expect(result.inputs).toHaveLength(1);
      expect(result.inputs[0].defaultValue).toBe('vec3(1.0, 0.0, 0.5)');
    });

    it('parses uniform without default value → defaultValue is undefined', () => {
      const result = parseShader('uniform float x;');
      expect(result.inputs[0].defaultValue).toBeUndefined();
    });
  });

  describe('output parsing', () => {
    it('parses a single out vec4', () => {
      const result = parseShader('out vec4 fragColor;');
      expect(result.outputs).toHaveLength(1);
      expect(result.outputs[0].label).toBe('fragColor');
      expect(result.outputs[0].dataType).toBe('vec4');
      expect(result.outputs[0].direction).toBe('output');
    });

    it('parses multiple out declarations', () => {
      const code = 'out vec4 color;\nout vec3 normal;\nout float depth;';
      const result = parseShader(code);
      expect(result.outputs).toHaveLength(3);

      expect(result.outputs[0].label).toBe('color');
      expect(result.outputs[0].dataType).toBe('vec4');
      expect(result.outputs[1].label).toBe('normal');
      expect(result.outputs[1].dataType).toBe('vec3');
      expect(result.outputs[2].label).toBe('depth');
      expect(result.outputs[2].dataType).toBe('float');
    });

    it('parses out declarations with various types', () => {
      const code = 'out int idx;\nout mat3 basis;';
      const result = parseShader(code);
      expect(result.outputs).toHaveLength(2);
      expect(result.outputs[0].dataType).toBe('int');
      expect(result.outputs[1].dataType).toBe('mat3');
    });
  });

  describe('edge cases', () => {
    it('empty code → empty arrays', () => {
      const result = parseShader('');
      expect(result.inputs).toEqual([]);
      expect(result.outputs).toEqual([]);
      expect(result.raw).toBe('');
    });

    it('code with no uniforms/outs → empty arrays', () => {
      const code = 'void main() {\n  gl_FragColor = vec4(1.0);\n}';
      const result = parseShader(code);
      expect(result.inputs).toEqual([]);
      expect(result.outputs).toEqual([]);
    });

    it('preserves raw code in result', () => {
      const code = 'uniform float x;\nout vec4 fragColor;';
      const result = parseShader(code);
      expect(result.raw).toBe(code);
    });

    it('mapType fallback for unrecognized type returns float (tested via regex — unrecognized types not matched by regex)', () => {
      // The UNIFORM_RE regex only matches known types, so an invalid type
      // like "customType" won't be captured. However, we can test the
      // mapType fallback indirectly: types matched by regex but not in
      // GLSL_VALID_TYPES would fall back to 'float'.
      // The regex matches mat[234], so mat2/mat3/mat4 are captured.
      // All regex-matched types ARE in GLSL_VALID_TYPES, so the fallback
      // path is unreachable via parseShader. We verify that all matched
      // types map correctly instead.
      const code = 'uniform mat2 m;';
      const result = parseShader(code);
      expect(result.inputs[0].dataType).toBe('mat2');
    });
  });

  describe('existingInputs port id preservation', () => {
    it('preserves port id when label matches existing input', () => {
      const existingInputs: Port[] = [
        { id: 'preserved-id-123', label: 'intensity', dataType: 'float', direction: 'input' },
      ];

      const result = parseShader('uniform float intensity;', existingInputs);
      expect(result.inputs).toHaveLength(1);
      expect(result.inputs[0].id).toBe('preserved-id-123');
      expect(result.inputs[0].label).toBe('intensity');
    });

    it('generates new id when label does not match existing input', () => {
      const existingInputs: Port[] = [
        { id: 'old-id', label: 'brightness', dataType: 'float', direction: 'input' },
      ];

      const result = parseShader('uniform float intensity;', existingInputs);
      expect(result.inputs).toHaveLength(1);
      expect(result.inputs[0].id).not.toBe('old-id');
      expect(result.inputs[0].id).toMatch(/^port_\d+_\d+$/);
    });

    it('preserves ids for matching labels and generates new for non-matching', () => {
      const existingInputs: Port[] = [
        { id: 'keep-me', label: 'x', dataType: 'float', direction: 'input' },
        { id: 'also-keep', label: 'y', dataType: 'float', direction: 'input' },
      ];

      const result = parseShader('uniform float x;\nuniform float z;', existingInputs);
      expect(result.inputs).toHaveLength(2);
      expect(result.inputs[0].id).toBe('keep-me');
      expect(result.inputs[1].id).toMatch(/^port_\d+_\d+$/);
    });
  });

  describe('existingOutputs port id preservation', () => {
    it('preserves port id when label matches existing output', () => {
      const existingOutputs: Port[] = [
        { id: 'output-id-456', label: 'fragColor', dataType: 'vec4', direction: 'output' },
      ];

      const result = parseShader('out vec4 fragColor;', undefined, existingOutputs);
      expect(result.outputs).toHaveLength(1);
      expect(result.outputs[0].id).toBe('output-id-456');
    });

    it('generates new id when label does not match existing output', () => {
      const existingOutputs: Port[] = [
        { id: 'old-out-id', label: 'color', dataType: 'vec4', direction: 'output' },
      ];

      const result = parseShader('out vec4 fragColor;', undefined, existingOutputs);
      expect(result.outputs).toHaveLength(1);
      expect(result.outputs[0].id).not.toBe('old-out-id');
    });
  });

  describe('combined uniform and output parsing', () => {
    it('parses a full shader with uniforms and outputs', () => {
      const code = [
        'uniform sampler2D inputImage;',
        'uniform float intensity;',
        '',
        'out vec4 fragColor;',
        '',
        'void main() {',
        '  vec4 color = texture(inputImage, v_uv);',
        '  color.rgb *= intensity;',
        '  fragColor = color;',
        '}',
      ].join('\n');

      const result = parseShader(code);
      expect(result.inputs).toHaveLength(2);
      expect(result.outputs).toHaveLength(1);
      expect(result.inputs[0].label).toBe('inputImage');
      expect(result.inputs[0].dataType).toBe('sampler2D');
      expect(result.inputs[1].label).toBe('intensity');
      expect(result.inputs[1].dataType).toBe('float');
      expect(result.outputs[0].label).toBe('fragColor');
      expect(result.outputs[0].dataType).toBe('vec4');
    });
  });
});
