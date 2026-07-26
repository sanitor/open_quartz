import { describe, it, expect } from 'vitest';
import {
  predefinedShaders,
  CUSTOM_SHADER_CODE,
  CUSTOM_2IN1_SHADER,
} from '../../src/catalog/predefinedShaders';
import { generatorShaders } from '../../src/catalog/shaders/generator';
import { feedbackShaders } from '../../src/catalog/shaders/feedback';
import { parseWgslShader } from '../../src/engine/gpu/wgslParser';
import { createDefaultShaderCode, createInputShader, makeNode } from '../../src/store/helpers';

// ---------------------------------------------------------------------------
// Expected parse results per shader — exact labels, types, and counts
// ---------------------------------------------------------------------------

const EXPECTED: Record<string, { inputs: [string, string][]; outputs: [string, string][] }> = {
  // Filters
  'Resample':              { inputs: [['inputImage', 'sampler2D']], outputs: [['fragColor', 'vec4']] },
  'Sobel Edge Detection':  { inputs: [['inputImage', 'sampler2D'], ['intensity', 'float']], outputs: [['fragColor', 'vec4']] },
  'Gaussian Blur 3x3':     { inputs: [['inputImage', 'sampler2D']], outputs: [['fragColor', 'vec4']] },
  'Box Blur':              { inputs: [['inputImage', 'sampler2D']], outputs: [['fragColor', 'vec4']] },
  'Sharpen':               { inputs: [['inputImage', 'sampler2D'], ['strength', 'float']], outputs: [['fragColor', 'vec4']] },
  'Emboss':                { inputs: [['inputImage', 'sampler2D']], outputs: [['fragColor', 'vec4']] },
  'Pixelate':              { inputs: [['inputImage', 'sampler2D'], ['blockSize', 'float']], outputs: [['fragColor', 'vec4']] },
  // Color
  'Invert':                { inputs: [['inputImage', 'sampler2D']], outputs: [['fragColor', 'vec4']] },
  'Grayscale':             { inputs: [['inputImage', 'sampler2D']], outputs: [['fragColor', 'vec4']] },
  'Brightness/Contrast':   { inputs: [['inputImage', 'sampler2D'], ['contrast', 'float'], ['brightness', 'float']], outputs: [['fragColor', 'vec4']] },
  'Hue Rotate':            { inputs: [['inputImage', 'sampler2D'], ['angle', 'float']], outputs: [['fragColor', 'vec4']] },
  'Threshold':             { inputs: [['inputImage', 'sampler2D'], ['threshold', 'float']], outputs: [['fragColor', 'vec4']] },
  'Sepia':                 { inputs: [['inputImage', 'sampler2D']], outputs: [['fragColor', 'vec4']] },
  'Field Color Map':       { inputs: [['inputImage', 'sampler2D']], outputs: [['fragColor', 'vec4']] },
  // Generators
  'Solid Color':           { inputs: [['color', 'float']], outputs: [['fragColor', 'vec4']] },
  'Gradient':              { inputs: [['colorA', 'float'], ['colorB', 'float']], outputs: [['fragColor', 'vec4']] },
  'Checkerboard':          { inputs: [['gridSize', 'float'], ['color1', 'float'], ['color2', 'float']], outputs: [['fragColor', 'vec4']] },
  'Noise':                 { inputs: [['scale', 'float']], outputs: [['fragColor', 'vec4']] },
  'Circle':                { inputs: [['circle', 'float']], outputs: [['fragColor', 'vec4']] },
  // Blend
  'Add':                   { inputs: [['inputA', 'sampler2D'], ['inputB', 'sampler2D']], outputs: [['fragColor', 'vec4']] },
  'Multiply':              { inputs: [['inputA', 'sampler2D'], ['inputB', 'sampler2D']], outputs: [['fragColor', 'vec4']] },
  'Screen':                { inputs: [['inputA', 'sampler2D'], ['inputB', 'sampler2D']], outputs: [['fragColor', 'vec4']] },
  'Overlay':               { inputs: [['inputA', 'sampler2D'], ['inputB', 'sampler2D']], outputs: [['fragColor', 'vec4']] },
  'Difference':            { inputs: [['inputA', 'sampler2D'], ['inputB', 'sampler2D']], outputs: [['fragColor', 'vec4']] },
  'Exclusion':             { inputs: [['inputA', 'sampler2D'], ['inputB', 'sampler2D']], outputs: [['fragColor', 'vec4']] },
  'Soft Light':            { inputs: [['inputA', 'sampler2D'], ['inputB', 'sampler2D']], outputs: [['fragColor', 'vec4']] },
  // Distortion
  'Twirl':                 { inputs: [['inputImage', 'sampler2D'], ['radius', 'float'], ['angle', 'float']], outputs: [['fragColor', 'vec4']] },
  'Ripple':                { inputs: [['inputImage', 'sampler2D'], ['frequency', 'float'], ['amplitude', 'float']], outputs: [['fragColor', 'vec4']] },
  'Displacement':          { inputs: [['displaceMap', 'sampler2D'], ['inputImage', 'sampler2D'], ['strength', 'float']], outputs: [['fragColor', 'vec4']] },
  'Barrel':                { inputs: [['inputImage', 'sampler2D'], ['k1', 'float'], ['k2', 'float']], outputs: [['fragColor', 'vec4']] },
  'Pinch':                 { inputs: [['inputImage', 'sampler2D'], ['radius', 'float'], ['strength', 'float']], outputs: [['fragColor', 'vec4']] },
  // Feedback
  'Gray-Scott Reaction-Diffusion': { inputs: [['dA', 'float'], ['feedRate', 'float'], ['timestep', 'float'], ['dB', 'float'], ['killRate', 'float']], outputs: [['fragColor', 'vec4']] },
};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('predefinedShaders', () => {
  it('is a non-empty array with unique labels', () => {
    expect(predefinedShaders.length).toBeGreaterThan(0);
    const labels = predefinedShaders.map(s => s.label);
    expect(new Set(labels).size).toBe(labels.length);
  });

  it('every predefined shader has an expected parse spec', () => {
    for (const shader of predefinedShaders) {
      expect(EXPECTED).toHaveProperty(shader.label);
    }
  });

  for (const shader of predefinedShaders) {
    it(`${shader.label}: exact inputs and outputs`, () => {
      const result = parseWgslShader(shader.code);
      const spec = EXPECTED[shader.label];

      expect(result.parseError).toBeUndefined();
      expect(result.raw).toBe(shader.code);

      // Exact input count, labels, and types
      const actualInputs = result.inputs.map(p => [p.label, p.dataType]);
      expect(actualInputs).toEqual(spec.inputs);

      // Exact output count, labels, and types
      const actualOutputs = result.outputs.map(p => [p.label, p.dataType]);
      expect(actualOutputs).toEqual(spec.outputs);
    });
  }
});

describe('CUSTOM_SHADER_CODE', () => {
  it('exact parse: inputImage (sampler2D) + intensity (float), 1 output', () => {
    const result = parseWgslShader(CUSTOM_SHADER_CODE);
    expect(result.parseError).toBeUndefined();
    expect(result.inputs.map(p => [p.label, p.dataType])).toEqual([
      ['inputImage', 'sampler2D'],
      ['intensity', 'float'],
    ]);
    expect(result.outputs).toHaveLength(1);
    expect(result.outputs[0].label).toBe('fragColor');
    expect(result.outputs[0].dataType).toBe('vec4');
  });
});

describe('CUSTOM_2IN1_SHADER', () => {
  it('exact parse: inputA + inputB (sampler2D) + mixFactor (float), 1 output', () => {
    const result = parseWgslShader(CUSTOM_2IN1_SHADER);
    expect(result.parseError).toBeUndefined();
    expect(result.inputs.map(p => [p.label, p.dataType])).toEqual([
      ['inputA', 'sampler2D'],
      ['inputB', 'sampler2D'],
      ['mixFactor', 'float'],
    ]);
    expect(result.outputs).toHaveLength(1);
    expect(result.outputs[0].label).toBe('fragColor');
    expect(result.outputs[0].dataType).toBe('vec4');
  });
});

describe('createDefaultShaderCode / createInputShader', () => {
  it('default shader: inputImage (sampler2D) + intensity (float)', () => {
    const result = parseWgslShader(createDefaultShaderCode('shader'));
    expect(result.parseError).toBeUndefined();
    expect(result.inputs.map(p => [p.label, p.dataType])).toEqual([
      ['inputImage', 'sampler2D'],
      ['intensity', 'float'],
    ]);
    expect(result.outputs).toHaveLength(1);
  });

  it('input shader sampler2D: value (sampler2D)', () => {
    const result = parseWgslShader(createInputShader('sampler2D'));
    expect(result.parseError).toBeUndefined();
    expect(result.inputs.map(p => [p.label, p.dataType])).toEqual([
      ['value', 'sampler2D'],
    ]);
    expect(result.outputs).toHaveLength(1);
  });

  it('input shader float: value (float)', () => {
    const result = parseWgslShader(createInputShader('float'));
    expect(result.parseError).toBeUndefined();
    expect(result.inputs.map(p => [p.label, p.dataType])).toEqual([
      ['value', 'float'],
    ]);
    expect(result.outputs).toHaveLength(1);
  });

  it('constant shader: color (vec4)', () => {
    const result = parseWgslShader(createDefaultShaderCode('constant'));
    expect(result.parseError).toBeUndefined();
    expect(result.inputs.map(p => [p.label, p.dataType])).toEqual([
      ['color', 'vec4'],
    ]);
    expect(result.outputs).toHaveLength(1);
  });
});

describe('makeNode output port types', () => {
  it('input node (float): output port is float, not vec4', () => {
    const node = makeNode('input', undefined, 'float');
    expect(node.data.outputs).toHaveLength(1);
    expect(node.data.outputs[0].dataType).toBe('float');
  });

  it('input node (sampler2D): output port is sampler2D', () => {
    const node = makeNode('input', undefined, 'sampler2D');
    expect(node.data.outputs).toHaveLength(1);
    expect(node.data.outputs[0].dataType).toBe('sampler2D');
  });

  it('input node (vec3): output port is vec3', () => {
    const node = makeNode('input', undefined, 'vec3');
    expect(node.data.outputs).toHaveLength(1);
    expect(node.data.outputs[0].dataType).toBe('vec3');
  });

  it('constant node: output port matches uniform type (vec4)', () => {
    const node = makeNode('constant');
    expect(node.data.outputs).toHaveLength(1);
    expect(node.data.outputs[0].dataType).toBe('vec4');
  });

  it('shader node: output port stays vec4 (fragment color)', () => {
    const node = makeNode('shader');
    expect(node.data.outputs).toHaveLength(1);
    expect(node.data.outputs[0].dataType).toBe('vec4');
  });
});
