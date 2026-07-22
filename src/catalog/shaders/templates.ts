export const CUSTOM_SHADER_CODE = [
  '@fragment',
  'fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {',
  '  var color = textureSample(inputImage, inputImageSampler, v_uv);',
  '  color = vec4f(color.rgb * intensity, color.a);',
  '  return color;',
  '}',
].join('\n');

export const CUSTOM_2IN1_SHADER = `@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let a = textureSample(inputA, inputASampler, v_uv);
  let b = textureSample(inputB, inputBSampler, v_uv);
  return mix(a, b, mixFactor);
}`;
