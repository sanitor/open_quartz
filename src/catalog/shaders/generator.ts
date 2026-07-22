import type { ShaderEntry } from './filter';

export const generatorShaders: ShaderEntry[] = [
  {
    label: 'Solid Color',
    code: `@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  return color;
}`,
  },
  {
    label: 'Gradient',
    code: `@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  return mix(colorA, colorB, v_uv.x);
}`,
  },
  {
    label: 'Checkerboard',
    code: `@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let cell = floor(v_uv * max(gridSize, vec2f(1.0)));
  let checker = (cell.x + cell.y) % 2.0;
  return mix(color1, color2, checker);
}`,
  },
  {
    label: 'Noise',
    code: `fn hash(p: vec2f) -> f32 {
  return fract(sin(dot(p, vec2f(127.1, 311.7))) * 43758.5453123);
}

fn valueNoise(p: vec2f) -> f32 {
  let i = floor(p);
  let f = fract(p);
  let u = f * f * (vec2f(3.0) - 2.0 * f);
  let a = hash(i);
  let b = hash(i + vec2f(1.0, 0.0));
  let c = hash(i + vec2f(0.0, 1.0));
  let d = hash(i + vec2f(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let n = valueNoise(v_uv * max(scale, 1.0));
  return vec4f(vec3f(n), 1.0);
}`,
  },
  {
    label: 'Circle',
    code: `@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let center = circle.xy;
  let radius = circle.z;
  let dist = length(v_uv - center);
  let mask = 1.0 - step(radius, dist);
  return vec4f(vec3f(mask), 1.0);
}`,
  },
];
