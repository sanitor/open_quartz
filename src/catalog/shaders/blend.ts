import type { ShaderEntry } from './filter';

export const blendShaders: ShaderEntry[] = [
  {
    label: 'Add',
    code: `uniform sampler2D inputA;
uniform sampler2D inputB;

out vec4 fragColor;

void main() {
  vec4 a = texture(inputA, v_uv);
  vec4 b = texture(inputB, v_uv);
  fragColor = min(a + b, vec4(1.0));
}`,
  },
  {
    label: 'Multiply',
    code: `uniform sampler2D inputA;
uniform sampler2D inputB;

out vec4 fragColor;

void main() {
  vec4 a = texture(inputA, v_uv);
  vec4 b = texture(inputB, v_uv);
  fragColor = a * b;
}`,
  },
  {
    label: 'Screen',
    code: `uniform sampler2D inputA;
uniform sampler2D inputB;

out vec4 fragColor;

void main() {
  vec4 a = texture(inputA, v_uv);
  vec4 b = texture(inputB, v_uv);
  fragColor = vec4(1.0) - (vec4(1.0) - a) * (vec4(1.0) - b);
}`,
  },
  {
    label: 'Overlay',
    code: `uniform sampler2D inputA;
uniform sampler2D inputB;

out vec4 fragColor;

vec3 overlay(vec3 base, vec3 blend) {
  return mix(
    2.0 * base * blend,
    1.0 - 2.0 * (1.0 - base) * (1.0 - blend),
    step(0.5, base)
  );
}

void main() {
  vec4 a = texture(inputA, v_uv);
  vec4 b = texture(inputB, v_uv);
  fragColor = vec4(overlay(a.rgb, b.rgb), max(a.a, b.a));
}`,
  },
  {
    label: 'Difference',
    code: `uniform sampler2D inputA;
uniform sampler2D inputB;

out vec4 fragColor;

void main() {
  vec4 a = texture(inputA, v_uv);
  vec4 b = texture(inputB, v_uv);
  fragColor = vec4(abs(a.rgb - b.rgb), max(a.a, b.a));
}`,
  },
  {
    label: 'Exclusion',
    code: `uniform sampler2D inputA;
uniform sampler2D inputB;

out vec4 fragColor;

void main() {
  vec4 a = texture(inputA, v_uv);
  vec4 b = texture(inputB, v_uv);
  fragColor = vec4(a.rgb + b.rgb - 2.0 * a.rgb * b.rgb, max(a.a, b.a));
}`,
  },
  {
    label: 'Soft Light',
    code: `uniform sampler2D inputA;
uniform sampler2D inputB;

out vec4 fragColor;

vec3 softLight(vec3 base, vec3 blend) {
  vec3 lo = 2.0 * base * blend + base * base * (1.0 - 2.0 * blend);
  vec3 hi = 2.0 * base * (1.0 - blend) + sqrt(base) * (2.0 * blend - 1.0);
  return mix(lo, hi, step(0.5, blend));
}

void main() {
  vec4 a = texture(inputA, v_uv);
  vec4 b = texture(inputB, v_uv);
  fragColor = vec4(softLight(a.rgb, b.rgb), max(a.a, b.a));
}`,
  },
];
