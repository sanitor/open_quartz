import type { ShaderEntry } from './filter';

export const generatorShaders: ShaderEntry[] = [
  {
    label: 'Solid Color',
    code: `uniform vec4 color;

out vec4 fragColor;

void main() {
  fragColor = color;
}`,
  },
  {
    label: 'Gradient',
    code: `uniform vec4 colorA;
uniform vec4 colorB;

out vec4 fragColor;

void main() {
  fragColor = mix(colorA, colorB, v_uv.x);
}`,
  },
  {
    label: 'Checkerboard',
    code: `uniform vec2 gridSize;
uniform vec4 color1;
uniform vec4 color2;

out vec4 fragColor;

void main() {
  vec2 cell = floor(v_uv * max(gridSize, vec2(1.0)));
  float checker = mod(cell.x + cell.y, 2.0);
  fragColor = mix(color1, color2, checker);
}`,
  },
  {
    label: 'Noise',
    code: `uniform float scale;

out vec4 fragColor;

float hash(vec2 p) {
  return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453123);
}

float valueNoise(vec2 p) {
  vec2 i = floor(p);
  vec2 f = fract(p);
  vec2 u = f * f * (3.0 - 2.0 * f);
  float a = hash(i);
  float b = hash(i + vec2(1.0, 0.0));
  float c = hash(i + vec2(0.0, 1.0));
  float d = hash(i + vec2(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

void main() {
  float n = valueNoise(v_uv * max(scale, 1.0));
  fragColor = vec4(vec3(n), 1.0);
}`,
  },
  {
    label: 'Circle',
    code: `uniform vec3 circle;

out vec4 fragColor;

void main() {
  vec2 center = circle.xy;
  float radius = circle.z;
  float dist = length(v_uv - center);
  float mask = 1.0 - step(radius, dist);
  fragColor = vec4(vec3(mask), 1.0);
}`,
  },
];
