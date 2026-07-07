import { describe, it, expect } from 'vitest';
import { DATA_TYPE_COLORS, GLSL_VALID_TYPES } from '../../src/types';
import type { DataType } from '../../src/types';

describe('DATA_TYPE_COLORS', () => {
  const allDataTypes: DataType[] = [
    'float', 'int', 'uint', 'bool',
    'vec2', 'vec3', 'vec4',
    'ivec2', 'ivec3', 'ivec4',
    'uvec2', 'uvec3', 'uvec4',
    'bvec2', 'bvec3', 'bvec4',
    'mat2', 'mat3', 'mat4',
    'sampler2D', 'samplerCube',
  ];

  it('has an entry for every DataType', () => {
    for (const dt of allDataTypes) {
      expect(DATA_TYPE_COLORS[dt]).toBeDefined();
      expect(typeof DATA_TYPE_COLORS[dt]).toBe('string');
    }
  });

  it('every color value is a non-empty string', () => {
    for (const dt of allDataTypes) {
      expect(DATA_TYPE_COLORS[dt].length).toBeGreaterThan(0);
    }
  });

  it('color values look like hex colors', () => {
    for (const dt of allDataTypes) {
      expect(DATA_TYPE_COLORS[dt]).toMatch(/^#[0-9a-fA-F]{6}$/);
    }
  });

  it('has exactly the expected number of entries', () => {
    expect(Object.keys(DATA_TYPE_COLORS)).toHaveLength(allDataTypes.length);
  });
});

describe('GLSL_VALID_TYPES', () => {
  it('is a non-empty array', () => {
    expect(Array.isArray(GLSL_VALID_TYPES)).toBe(true);
    expect(GLSL_VALID_TYPES.length).toBeGreaterThan(0);
  });

  it('contains all expected scalar types', () => {
    expect(GLSL_VALID_TYPES).toContain('float');
    expect(GLSL_VALID_TYPES).toContain('int');
    expect(GLSL_VALID_TYPES).toContain('uint');
    expect(GLSL_VALID_TYPES).toContain('bool');
  });

  it('contains all vec types', () => {
    expect(GLSL_VALID_TYPES).toContain('vec2');
    expect(GLSL_VALID_TYPES).toContain('vec3');
    expect(GLSL_VALID_TYPES).toContain('vec4');
    expect(GLSL_VALID_TYPES).toContain('ivec2');
    expect(GLSL_VALID_TYPES).toContain('ivec3');
    expect(GLSL_VALID_TYPES).toContain('ivec4');
    expect(GLSL_VALID_TYPES).toContain('uvec2');
    expect(GLSL_VALID_TYPES).toContain('uvec3');
    expect(GLSL_VALID_TYPES).toContain('uvec4');
    expect(GLSL_VALID_TYPES).toContain('bvec2');
    expect(GLSL_VALID_TYPES).toContain('bvec3');
    expect(GLSL_VALID_TYPES).toContain('bvec4');
  });

  it('contains all mat types', () => {
    expect(GLSL_VALID_TYPES).toContain('mat2');
    expect(GLSL_VALID_TYPES).toContain('mat3');
    expect(GLSL_VALID_TYPES).toContain('mat4');
  });

  it('contains sampler types', () => {
    expect(GLSL_VALID_TYPES).toContain('sampler2D');
    expect(GLSL_VALID_TYPES).toContain('samplerCube');
  });

  it('has no duplicate entries', () => {
    const unique = new Set(GLSL_VALID_TYPES);
    expect(unique.size).toBe(GLSL_VALID_TYPES.length);
  });
});

describe('cross-validation: GLSL_VALID_TYPES ↔ DATA_TYPE_COLORS', () => {
  it('every GLSL_VALID_TYPES entry is a key in DATA_TYPE_COLORS', () => {
    for (const t of GLSL_VALID_TYPES) {
      expect(DATA_TYPE_COLORS[t]).toBeDefined();
    }
  });

  it('every DATA_TYPE_COLORS key is in GLSL_VALID_TYPES', () => {
    const validSet = new Set<string>(GLSL_VALID_TYPES);
    for (const key of Object.keys(DATA_TYPE_COLORS)) {
      expect(validSet.has(key)).toBe(true);
    }
  });
});
