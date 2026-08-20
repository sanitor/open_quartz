import type { ShaderEntry } from './filter';

export const generatorShaders: ShaderEntry[] = [
  {
    label: 'Solid Color',
    code: `// Outputs a solid color.
@group(0) @binding(0) var<uniform> color: vec4f; // RGBA color value
@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  return color;
}`,
  },
  {
    label: 'Gradient',
    code: `// Horizontal linear gradient between two colors.
@group(0) @binding(0) var<uniform> colorA: vec4f; // Left edge color
@group(0) @binding(1) var<uniform> colorB: vec4f; // Right edge color
@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  return mix(colorA, colorB, v_uv.x);
}`,
  },
  {
    label: 'Checkerboard',
    code: `// Generates a checkerboard pattern.
@group(0) @binding(0) var<uniform> gridSize: vec2f; // Number of cells (columns, rows). Default (8, 8)
@group(0) @binding(1) var<uniform> color1: vec4f; // First square color
@group(0) @binding(2) var<uniform> color2: vec4f; // Second square color
@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let cell = floor(v_uv * max(gridSize, vec2f(1.0)));
  let checker = (cell.x + cell.y) % 2.0;
  return mix(color1, color2, checker);
}`,
  },
  {
    label: 'Noise',
    code: `// Generates value noise.
@group(0) @binding(0) var<uniform> scale: f32; // Noise frequency. Higher = finer detail. Default 10
fn hash(p: vec2f) -> f32 {
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
    code: `// Draws a filled circle.
@group(0) @binding(0) var<uniform> circle: vec4f; // Circle parameters (centerX, centerY, radius, unused). Default (0.5, 0.5, 0.25, 0)
@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let center = circle.xy;
  let radius = circle.z;
  let dist = length(v_uv - center);
  let mask = 1.0 - step(radius, dist);
  return vec4f(vec3f(mask), 1.0);
}`,
  },
];
