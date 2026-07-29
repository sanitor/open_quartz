export const CUSTOM_SHADER_CODE = [
  '@group(0) @binding(0) var inputImage: texture_2d<f32>;',
  '@group(0) @binding(1) var inputImageSampler: sampler;',
  '@group(0) @binding(2) var<uniform> intensity: f32;',
  '@fragment',
  'fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {',
  '  var color = textureSample(inputImage, inputImageSampler, v_uv);',
  '  color = vec4f(color.rgb * intensity, color.a);',
  '  return color;',
  '}',
].join('\n');

export const CUSTOM_2IN1_SHADER = `@group(0) @binding(0) var inputA: texture_2d<f32>;
@group(0) @binding(1) var inputASampler: sampler;
@group(0) @binding(2) var inputB: texture_2d<f32>;
@group(0) @binding(3) var inputBSampler: sampler;
@group(0) @binding(4) var<uniform> mixFactor: f32;
@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let a = textureSample(inputA, inputASampler, v_uv);
  let b = textureSample(inputB, inputBSampler, v_uv);
  return mix(a, b, mixFactor);
}`;
