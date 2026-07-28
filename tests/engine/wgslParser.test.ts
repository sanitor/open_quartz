import { describe, it, expect } from 'vitest';
import { parseWgslShader } from '../../src/engine/gpu/wgslParser';

// =============================================================================
// 1. AST path — shaders WITH @group/@binding declarations
// =============================================================================

describe('parseWgslShader — AST path (with @group/@binding)', () => {
  it('1a. texture + uniform: extracts inputImage (sampler2D) and intensity (float)', () => {
    const code = `\
@group(0) @binding(0) var inputImage: texture_2d<f32>;
@group(0) @binding(1) var inputImageSampler: sampler;
@group(0) @binding(2) var<uniform> intensity: f32;

@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  var color = textureSample(inputImage, inputImageSampler, v_uv);
  color = vec4f(color.rgb * intensity, color.a);
  return color;
}`;
    const result = parseWgslShader(code);

    expect(result.parseError).toBeUndefined();
    expect(result.inputs).toHaveLength(2);
    expect(result.inputs[0].label).toBe('inputImage');
    expect(result.inputs[0].dataType).toBe('sampler2D');
    expect(result.inputs[0].direction).toBe('input');
    expect(result.inputs[1].label).toBe('intensity');
    expect(result.inputs[1].dataType).toBe('float');
    expect(result.inputs[1].direction).toBe('input');
    expect(result.outputs).toHaveLength(1);
    expect(result.outputs[0].label).toBe('fragColor');
    expect(result.outputs[0].dataType).toBe('vec4');
    expect(result.outputs[0].direction).toBe('output');
  });

  it('1b. uniform only: extracts color (vec4)', () => {
    const code = `\
@group(0) @binding(0) var<uniform> color: vec4f;
@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f { return color; }`;
    const result = parseWgslShader(code);

    expect(result.parseError).toBeUndefined();
    expect(result.inputs).toHaveLength(1);
    expect(result.inputs[0].label).toBe('color');
    expect(result.inputs[0].dataType).toBe('vec4');
    expect(result.inputs[0].direction).toBe('input');
    expect(result.outputs).toHaveLength(1);
  });

  it('1c. sampler2D input: extracts value (sampler2D), filters out valueSampler', () => {
    const code = `\
@group(0) @binding(0) var value: texture_2d<f32>;
@group(0) @binding(1) var valueSampler: sampler;
@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f { return textureSample(value, valueSampler, v_uv); }`;
    const result = parseWgslShader(code);

    expect(result.parseError).toBeUndefined();
    expect(result.inputs).toHaveLength(1);
    expect(result.inputs[0].label).toBe('value');
    expect(result.inputs[0].dataType).toBe('sampler2D');
    expect(result.outputs).toHaveLength(1);
  });
});

// =============================================================================
// 2. Regex fallback path — shaders WITHOUT @group/@binding declarations
// =============================================================================

describe('parseWgslShader — regex fallback (no declarations)', () => {
  it('2a. REGRESSION: resample detects only inputImage, not fragment/location', () => {
    const code = `\
@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  return textureSample(inputImage, inputImageSampler, v_uv);
}`;
    const result = parseWgslShader(code);

    expect(result.parseError).toBeUndefined();
    expect(result.inputs).toHaveLength(1);
    expect(result.inputs[0].label).toBe('inputImage');
    expect(result.inputs[0].dataType).toBe('sampler2D');
    expect(result.inputs[0].direction).toBe('input');

    // Regression guard: attribute names must NOT appear as ports
    const labels = result.inputs.map(p => p.label);
    expect(labels).not.toContain('fragment');
    expect(labels).not.toContain('location');

    expect(result.outputs).toHaveLength(1);
  });

  it('2b. blend: detects exactly two texture inputs (inputA, inputB)', () => {
    const code = `\
@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let a = textureSample(inputA, inputASampler, v_uv);
  let b = textureSample(inputB, inputBSampler, v_uv);
  return min(a + b, vec4f(1.0));
}`;
    const result = parseWgslShader(code);

    expect(result.parseError).toBeUndefined();
    expect(result.inputs).toHaveLength(2);
    const labels = result.inputs.map(p => p.label);
    expect(labels).toContain('inputA');
    expect(labels).toContain('inputB');
    expect(result.inputs.every(p => p.dataType === 'sampler2D')).toBe(true);
    expect(result.outputs).toHaveLength(1);
  });

  it('2c. texture + inferred scalar uniform: inputImage + intensity', () => {
    const code = `\
@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let col = textureSample(inputImage, inputImageSampler, v_uv);
  return vec4f(col.rgb * intensity, col.a);
}`;
    const result = parseWgslShader(code);

    expect(result.parseError).toBeUndefined();
    expect(result.inputs).toHaveLength(2);
    const map = new Map(result.inputs.map(p => [p.label, p.dataType]));
    expect(map.get('inputImage')).toBe('sampler2D');
    expect(map.get('intensity')).toBe('float');
    expect(result.outputs).toHaveLength(1);
  });

  it('2d. generator: inferred scalar uniform only', () => {
    const code = `\
@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  return color;
}`;
    const result = parseWgslShader(code);

    expect(result.parseError).toBeUndefined();
    expect(result.inputs).toHaveLength(1);
    expect(result.inputs[0].label).toBe('color');
    expect(result.inputs[0].dataType).toBe('float');
    expect(result.outputs).toHaveLength(1);
  });

  it('2e. helper function names and local variables are NOT ports', () => {
    const code = `\
fn rgb2hsv(c: vec3f) -> vec3f {
  let K = vec4f(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
  let p = mix(vec4f(c.bg, K.wz), vec4f(c.gb, K.xy), step(c.b, c.g));
  let q = mix(vec4f(p.xyw, c.r), vec4f(c.r, p.yzx), step(p.x, c.r));
  let d = q.x - min(q.w, q.y);
  let e = 1.0e-10;
  return vec3f(abs(q.z + (q.w - q.y) / (6.0 * d + e)), d / (q.x + e), q.x);
}

@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let col = textureSample(inputImage, inputImageSampler, v_uv);
  let hsv = rgb2hsv(col.rgb);
  let h = fract(hsv.x + angle / 6.28318530718);
  return vec4f(col.rgb, col.a);
}`;
    const result = parseWgslShader(code);

    expect(result.parseError).toBeUndefined();

    const labels = result.inputs.map(p => p.label);
    // Should contain the real inputs
    expect(labels).toContain('inputImage');
    expect(labels).toContain('angle');

    // Must NOT contain helper fn names, locals, keywords, or attribute names
    const forbidden = ['rgb2hsv', 'K', 'p', 'q', 'd', 'e', 'h', 'hsv', 'col', 'fragment', 'location'];
    for (const name of forbidden) {
      expect(labels).not.toContain(name);
    }

    const map = new Map(result.inputs.map(p => [p.label, p.dataType]));
    expect(map.get('inputImage')).toBe('sampler2D');
    expect(map.get('angle')).toBe('float');
    expect(result.outputs).toHaveLength(1);
  });

  it('2f. feedback shader: previousFrame is builtin, yields 0 inputs', () => {
    const code = `\
@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let c = textureSample(previousFrame, previousFrameSampler, v_uv);
  return vec4f(c.rgb * 0.99, 1.0);
}`;
    const result = parseWgslShader(code);

    expect(result.parseError).toBeUndefined();
    expect(result.inputs).toHaveLength(0);
    expect(result.outputs).toHaveLength(1);
  });

  it('2g. REGRESSION: @doraemon must not appear as a port', () => {
    const code = `@doraemon fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  return vec4f(1.0);
}`;
    const result = parseWgslShader(code);
    const labels = result.inputs.map((p) => p.label);
    expect(labels).not.toContain('doraemon');
  });
});

// =============================================================================
// 3. Error handling
// =============================================================================

describe('parseWgslShader — error handling', () => {
  it('3a. invalid WGSL syntax produces a non-empty parseError', () => {
    const code = `@fragment fn main(broken {{{`;
    const result = parseWgslShader(code);

    expect(result.parseError).toBeDefined();
    expect(typeof result.parseError).toBe('string');
    expect(result.parseError!.length).toBeGreaterThan(0);
  });
});

// =============================================================================
// 4. Port ID preservation from existingInputs/existingOutputs
// =============================================================================

describe('parseWgslShader — port ID preservation', () => {
  it('reuses IDs from existingInputs for matching labels', () => {
    const code = `\
@group(0) @binding(0) var inputImage: texture_2d<f32>;
@group(0) @binding(1) var inputImageSampler: sampler;
@group(0) @binding(2) var<uniform> intensity: f32;

@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  var color = textureSample(inputImage, inputImageSampler, v_uv);
  return vec4f(color.rgb * intensity, color.a);
}`;
    const existingInputs = [
      { id: 'preserved_tex_42', label: 'inputImage', dataType: 'sampler2D' as const, direction: 'input' as const },
      { id: 'preserved_uni_99', label: 'intensity', dataType: 'float' as const, direction: 'input' as const },
    ];
    const existingOutputs = [
      { id: 'preserved_out_7', label: 'fragColor', dataType: 'vec4' as const, direction: 'output' as const },
    ];

    const result = parseWgslShader(code, existingInputs, existingOutputs);

    expect(result.inputs).toHaveLength(2);
    expect(result.inputs[0].id).toBe('preserved_tex_42');
    expect(result.inputs[0].label).toBe('inputImage');
    expect(result.inputs[1].id).toBe('preserved_uni_99');
    expect(result.inputs[1].label).toBe('intensity');
    expect(result.outputs).toHaveLength(1);
    expect(result.outputs[0].id).toBe('preserved_out_7');
    expect(result.outputs[0].label).toBe('fragColor');
  });

  it('generates new IDs for ports without matching existingInputs', () => {
    const code = `\
@group(0) @binding(0) var inputImage: texture_2d<f32>;
@group(0) @binding(1) var inputImageSampler: sampler;

@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  return textureSample(inputImage, inputImageSampler, v_uv);
}`;
    const existingInputs = [
      { id: 'old_id_for_something_else', label: 'unrelatedPort', dataType: 'float' as const, direction: 'input' as const },
    ];

    const result = parseWgslShader(code, existingInputs);

    expect(result.inputs).toHaveLength(1);
    expect(result.inputs[0].label).toBe('inputImage');
    // The ID should NOT be the unrelated port's ID
    expect(result.inputs[0].id).not.toBe('old_id_for_something_else');
    // Should have a generated ID
    expect(result.inputs[0].id).toMatch(/^port_\d+_\d+$/);
  });
});

// =============================================================================
// 5. Output type mapping
// =============================================================================

describe('parseWgslShader — type mapping', () => {
  it('vec4f output maps to dataType vec4', () => {
    const code = `\
@group(0) @binding(0) var<uniform> x: f32;
@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f { return vec4f(x); }`;
    const result = parseWgslShader(code);

    expect(result.outputs).toHaveLength(1);
    expect(result.outputs[0].dataType).toBe('vec4');
  });

  it('vec3f uniform maps to dataType vec3', () => {
    const code = `\
@group(0) @binding(0) var<uniform> x: vec3f;
@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f { return vec4f(x, 1.0); }`;
    const result = parseWgslShader(code);

    expect(result.inputs).toHaveLength(1);
    expect(result.inputs[0].label).toBe('x');
    expect(result.inputs[0].dataType).toBe('vec3');
    expect(result.outputs).toHaveLength(1);
    expect(result.outputs[0].dataType).toBe('vec4');
  });
});
