import { describe, it, expect } from 'vitest';
import {
  predefinedShaders,
  CUSTOM_SHADER_CODE,
  CUSTOM_2IN1_SHADER,
} from '../../src/catalog/predefinedShaders';
import { generatorShaders } from '../../src/catalog/shaders/generator';
import { feedbackShaders } from '../../src/catalog/shaders/feedback';
import { parseWgslShader } from '../../src/engine/gpu/wgslParser';

describe('predefinedShaders', () => {
  it('is a non-empty array', () => {
    expect(Array.isArray(predefinedShaders)).toBe(true);
    expect(predefinedShaders.length).toBeGreaterThan(0);
  });

  it('each entry has a label (string) and code (string)', () => {
    for (const shader of predefinedShaders) {
      expect(typeof shader.label).toBe('string');
      expect(shader.label.length).toBeGreaterThan(0);
      expect(typeof shader.code).toBe('string');
      expect(shader.code.length).toBeGreaterThan(0);
    }
  });

  it('has unique labels', () => {
    const labels = predefinedShaders.map(s => s.label);
    const unique = new Set(labels);
    expect(unique.size).toBe(labels.length);
  });

  it('each shader template can be parsed by parseWgslShader', () => {
    for (const shader of predefinedShaders) {
      const result = parseWgslShader(shader.code);
      // Every predefined shader should have at least one output
      expect(result.outputs.length).toBeGreaterThanOrEqual(1);
      expect(result.raw).toBe(shader.code);
    }
  });

  it('each non-generator shader has at least one texture_2d input', () => {
    const generatorLabels = new Set(generatorShaders.map((s) => s.label));
    const feedbackLabels = new Set(feedbackShaders.map((s) => s.label));
    for (const shader of predefinedShaders) {
      if (generatorLabels.has(shader.label) || feedbackLabels.has(shader.label)) continue;
      const result = parseWgslShader(shader.code);
      const hasSampler = result.inputs.some(p => p.dataType === 'sampler2D');
      expect(hasSampler).toBe(true);
    }
  });

  it('generator shaders have no texture_2d input', () => {
    for (const shader of generatorShaders) {
      const result = parseWgslShader(shader.code);
      const hasSampler = result.inputs.some(p => p.dataType === 'sampler2D');
      expect(hasSampler).toBe(false);
    }
  });
});

describe('CUSTOM_SHADER_CODE', () => {
  it('is a non-empty string', () => {
    expect(typeof CUSTOM_SHADER_CODE).toBe('string');
    expect(CUSTOM_SHADER_CODE.length).toBeGreaterThan(0);
  });

  it('contains a textureSample call', () => {
    expect(CUSTOM_SHADER_CODE).toMatch(/textureSample/);
  });

  it('contains an @fragment fn main', () => {
    expect(CUSTOM_SHADER_CODE).toMatch(/@fragment/);
  });

  it('can be parsed by parseWgslShader', () => {
    const result = parseWgslShader(CUSTOM_SHADER_CODE);
    expect(result.inputs.length).toBeGreaterThanOrEqual(1);
    expect(result.outputs).toHaveLength(1);
    expect(result.outputs[0].label).toBe('fragColor');
  });

  it('has a sampler2D input and a float input', () => {
    const result = parseWgslShader(CUSTOM_SHADER_CODE);
    const types = new Map(result.inputs.map(p => [p.label, p.dataType]));
    expect(types.get('inputImage')).toBe('sampler2D');
    expect(types.get('intensity')).toBe('float');
  });
});

describe('CUSTOM_2IN1_SHADER', () => {
  it('is a non-empty string', () => {
    expect(typeof CUSTOM_2IN1_SHADER).toBe('string');
    expect(CUSTOM_2IN1_SHADER.length).toBeGreaterThan(0);
  });

  it('contains two textureSample calls (inputA + inputB)', () => {
    const matches = CUSTOM_2IN1_SHADER.match(/textureSample\s*\(/g);
    expect(matches).not.toBeNull();
    expect(matches!.length).toBe(2);
  });

  it('can be parsed by parseWgslShader', () => {
    const result = parseWgslShader(CUSTOM_2IN1_SHADER);
    const samplers = result.inputs.filter(p => p.dataType === 'sampler2D');
    expect(samplers).toHaveLength(2);
    expect(samplers.map(s => s.label)).toContain('inputA');
    expect(samplers.map(s => s.label)).toContain('inputB');
  });

  it('has a mixFactor float uniform', () => {
    const result = parseWgslShader(CUSTOM_2IN1_SHADER);
    const mixPort = result.inputs.find(p => p.label === 'mixFactor');
    expect(mixPort).toBeDefined();
    expect(mixPort!.dataType).toBe('float');
  });

  it('has a fragColor output', () => {
    const result = parseWgslShader(CUSTOM_2IN1_SHADER);
    expect(result.outputs).toHaveLength(1);
    expect(result.outputs[0].label).toBe('fragColor');
    expect(result.outputs[0].dataType).toBe('vec4');
  });
});
