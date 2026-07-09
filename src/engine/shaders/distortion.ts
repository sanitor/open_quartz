import type { ShaderEntry } from './filter';

export const distortionShaders: ShaderEntry[] = [
  {
    label: 'Twirl',
    code: `uniform sampler2D inputImage;
uniform float angle;
uniform float radius;

out vec4 fragColor;

void main() {
  vec2 center = vec2(0.5);
  vec2 uv = v_uv - center;
  float dist = length(uv);
  float factor = max(1.0 - dist / max(radius, 0.001), 0.0);
  float a = angle * factor * factor;
  float s = sin(a);
  float c = cos(a);
  uv = vec2(uv.x * c - uv.y * s, uv.x * s + uv.y * c);
  fragColor = texture(inputImage, uv + center);
}`,
  },
  {
    label: 'Ripple',
    code: `uniform sampler2D inputImage;
uniform float amplitude;
uniform float frequency;

out vec4 fragColor;

void main() {
  vec2 uv = v_uv;
  uv.x += sin(uv.y * frequency) * amplitude;
  uv.y += sin(uv.x * frequency) * amplitude;
  fragColor = texture(inputImage, uv);
}`,
  },
  {
    label: 'Displacement',
    code: `uniform sampler2D inputImage;
uniform sampler2D displaceMap;
uniform float strength;

out vec4 fragColor;

void main() {
  vec4 disp = texture(displaceMap, v_uv);
  vec2 offset = (disp.rg - 0.5) * 2.0 * strength;
  fragColor = texture(inputImage, v_uv + offset);
}`,
  },
  {
    label: 'Barrel',
    code: `uniform sampler2D inputImage;
uniform float k1;
uniform float k2;

out vec4 fragColor;

void main() {
  vec2 uv = v_uv * 2.0 - 1.0;
  float r2 = dot(uv, uv);
  float distortion = 1.0 + k1 * r2 + k2 * r2 * r2;
  vec2 distorted = uv * distortion * 0.5 + 0.5;
  if (distorted.x < 0.0 || distorted.x > 1.0 || distorted.y < 0.0 || distorted.y > 1.0) {
    fragColor = vec4(0.0);
  } else {
    fragColor = texture(inputImage, distorted);
  }
}`,
  },
  {
    label: 'Pinch',
    code: `uniform sampler2D inputImage;
uniform float strength;
uniform float radius;

out vec4 fragColor;

void main() {
  vec2 center = vec2(0.5);
  vec2 uv = v_uv - center;
  float dist = length(uv);
  float r = max(radius, 0.001);
  if (dist < r) {
    float factor = dist / r;
    float pinch = pow(factor, 1.0 + strength) * r;
    uv = normalize(uv) * pinch;
  }
  fragColor = texture(inputImage, uv + center);
}`,
  },
];
