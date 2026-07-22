import type { ShaderEntry } from './filter';

export const blendShaders: ShaderEntry[] = [
  {
    label: 'Add',
    code: `@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let a = textureSample(inputA, inputASampler, v_uv);
  let b = textureSample(inputB, inputBSampler, v_uv);
  return min(a + b, vec4f(1.0));
}`,
  },
  {
    label: 'Multiply',
    code: `@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let a = textureSample(inputA, inputASampler, v_uv);
  let b = textureSample(inputB, inputBSampler, v_uv);
  return a * b;
}`,
  },
  {
    label: 'Screen',
    code: `@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let a = textureSample(inputA, inputASampler, v_uv);
  let b = textureSample(inputB, inputBSampler, v_uv);
  return vec4f(1.0) - (vec4f(1.0) - a) * (vec4f(1.0) - b);
}`,
  },
  {
    label: 'Overlay',
    code: `fn overlay(base: vec3f, blend: vec3f) -> vec3f {
  return mix(
    2.0 * base * blend,
    vec3f(1.0) - 2.0 * (vec3f(1.0) - base) * (vec3f(1.0) - blend),
    step(vec3f(0.5), base)
  );
}

@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let a = textureSample(inputA, inputASampler, v_uv);
  let b = textureSample(inputB, inputBSampler, v_uv);
  return vec4f(overlay(a.rgb, b.rgb), max(a.a, b.a));
}`,
  },
  {
    label: 'Difference',
    code: `@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let a = textureSample(inputA, inputASampler, v_uv);
  let b = textureSample(inputB, inputBSampler, v_uv);
  return vec4f(abs(a.rgb - b.rgb), max(a.a, b.a));
}`,
  },
  {
    label: 'Exclusion',
    code: `@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let a = textureSample(inputA, inputASampler, v_uv);
  let b = textureSample(inputB, inputBSampler, v_uv);
  return vec4f(a.rgb + b.rgb - 2.0 * a.rgb * b.rgb, max(a.a, b.a));
}`,
  },
  {
    label: 'Soft Light',
    code: `fn softLight(base: vec3f, blend: vec3f) -> vec3f {
  let lo = 2.0 * base * blend + base * base * (vec3f(1.0) - 2.0 * blend);
  let hi = 2.0 * base * (vec3f(1.0) - blend) + sqrt(base) * (2.0 * blend - vec3f(1.0));
  return mix(lo, hi, step(vec3f(0.5), blend));
}

@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let a = textureSample(inputA, inputASampler, v_uv);
  let b = textureSample(inputB, inputBSampler, v_uv);
  return vec4f(softLight(a.rgb, b.rgb), max(a.a, b.a));
}`,
  },
];
