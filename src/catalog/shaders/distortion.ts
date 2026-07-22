import type { ShaderEntry } from './filter';

export const distortionShaders: ShaderEntry[] = [
  {
    label: 'Twirl',
    code: `@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let center = vec2f(0.5);
  var uv = v_uv - center;
  let dist = length(uv);
  let factor = max(1.0 - dist / max(radius, 0.001), 0.0);
  let a = angle * factor * factor;
  let s = sin(a);
  let c = cos(a);
  uv = vec2f(uv.x * c - uv.y * s, uv.x * s + uv.y * c);
  return textureSample(inputImage, inputImageSampler, uv + center);
}`,
  },
  {
    label: 'Ripple',
    code: `@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  var uv = v_uv;
  uv.x += sin(uv.y * frequency) * amplitude;
  uv.y += sin(uv.x * frequency) * amplitude;
  return textureSample(inputImage, inputImageSampler, uv);
}`,
  },
  {
    label: 'Displacement',
    code: `@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let disp = textureSample(displaceMap, displaceMapSampler, v_uv);
  let offset = (disp.rg - vec2f(0.5)) * 2.0 * strength;
  return textureSample(inputImage, inputImageSampler, v_uv + offset);
}`,
  },
  {
    label: 'Barrel',
    code: `@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let uv = v_uv * 2.0 - vec2f(1.0);
  let r2 = dot(uv, uv);
  let distortion = 1.0 + k1 * r2 + k2 * r2 * r2;
  let distorted = uv * distortion * 0.5 + vec2f(0.5);
  if (distorted.x < 0.0 || distorted.x > 1.0 || distorted.y < 0.0 || distorted.y > 1.0) {
    return vec4f(0.0);
  }
  return textureSample(inputImage, inputImageSampler, distorted);
}`,
  },
  {
    label: 'Pinch',
    code: `@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let center = vec2f(0.5);
  var uv = v_uv - center;
  let dist = length(uv);
  let r = max(radius, 0.001);
  if (dist < r) {
    let factor = dist / r;
    let pinch = pow(factor, 1.0 + strength) * r;
    uv = normalize(uv) * pinch;
  }
  return textureSample(inputImage, inputImageSampler, uv + center);
}`,
  },
];
