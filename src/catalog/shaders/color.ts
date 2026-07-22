import type { ShaderEntry } from './filter';

export const colorShaders: ShaderEntry[] = [
  {
    label: 'Invert',
    code: `@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let col = textureSample(inputImage, inputImageSampler, v_uv);
  return vec4f(1.0 - col.rgb, col.a);
}`,
  },
  {
    label: 'Grayscale',
    code: `@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let col = textureSample(inputImage, inputImageSampler, v_uv);
  let gray = dot(col.rgb, vec3f(0.299, 0.587, 0.114));
  return vec4f(vec3f(gray), col.a);
}`,
  },
  {
    label: 'Brightness/Contrast',
    code: `@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let c = textureSample(inputImage, inputImageSampler, v_uv);
  let adjusted = (c.rgb - vec3f(0.5)) * max(contrast, 0.0) + vec3f(0.5) + vec3f(brightness);
  return vec4f(clamp(adjusted, vec3f(0.0), vec3f(1.0)), c.a);
}`,
  },
  {
    label: 'Hue Rotate',
    code: `fn rgb2hsv(c: vec3f) -> vec3f {
  let K = vec4f(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
  let p = mix(vec4f(c.bg, K.wz), vec4f(c.gb, K.xy), step(c.b, c.g));
  let q = mix(vec4f(p.xyw, c.r), vec4f(c.r, p.yzx), step(p.x, c.r));
  let d = q.x - min(q.w, q.y);
  let e = 1.0e-10;
  return vec3f(abs(q.z + (q.w - q.y) / (6.0 * d + e)), d / (q.x + e), q.x);
}

fn hsv2rgb(c: vec3f) -> vec3f {
  let K = vec4f(1.0, 2.0 / 3.0, 1.0 / 3.0, 3.0);
  let p = abs(fract(c.xxx + K.xyz) * 6.0 - K.www);
  return c.z * mix(K.xxx, clamp(p - K.xxx, vec3f(0.0), vec3f(1.0)), c.y);
}

@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let col = textureSample(inputImage, inputImageSampler, v_uv);
  let hsv = rgb2hsv(col.rgb);
  let h = fract(hsv.x + angle / 6.28318530718);
  return vec4f(hsv2rgb(vec3f(h, hsv.y, hsv.z)), col.a);
}`,
  },
  {
    label: 'Threshold',
    code: `@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let col = textureSample(inputImage, inputImageSampler, v_uv);
  let lum = dot(col.rgb, vec3f(0.299, 0.587, 0.114));
  let bw = step(threshold, lum);
  return vec4f(vec3f(bw), col.a);
}`,
  },
  {
    label: 'Sepia',
    code: `@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let col = textureSample(inputImage, inputImageSampler, v_uv);
  let r = dot(col.rgb, vec3f(0.393, 0.769, 0.189));
  let g = dot(col.rgb, vec3f(0.349, 0.686, 0.168));
  let b = dot(col.rgb, vec3f(0.272, 0.534, 0.131));
  return vec4f(min(r, 1.0), min(g, 1.0), min(b, 1.0), col.a);
}`,
  },
  {
    label: 'Field Color Map',
    code: `fn turbo(t: f32) -> vec3f {
  let a = vec3f(0.114, 0.056, 0.566);
  let b = vec3f(0.376, 0.763, 0.843);
  let c = vec3f(0.267, 0.472, 0.090);
  let d = vec3f(0.905, 0.811, 0.011);
  let e = vec3f(0.740, 0.080, 0.055);

  var r: vec3f;
  if (t < 0.25) { r = mix(a, b, t / 0.25); }
  else if (t < 0.5) { r = mix(b, c, (t - 0.25) / 0.25); }
  else if (t < 0.75) { r = mix(c, d, (t - 0.5) / 0.25); }
  else { r = mix(d, e, (t - 0.75) / 0.25); }
  return r;
}

@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let raw = textureSample(inputImage, inputImageSampler, v_uv);
  let v = raw.g;
  let color = turbo(clamp(v, 0.0, 1.0));
  return vec4f(color, 1.0);
}`,
  },
];
