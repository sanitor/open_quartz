export const CUSTOM_SHADER_CODE = [
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

export const CUSTOM_2IN1_SHADER = `uniform sampler2D inputA;
uniform sampler2D inputB;
uniform float mixFactor;

out vec4 fragColor;

void main() {
  vec4 a = texture(inputA, v_uv);
  vec4 b = texture(inputB, v_uv);
  fragColor = mix(a, b, mixFactor);
}`;
