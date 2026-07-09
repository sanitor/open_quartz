import type { ShaderEntry } from './filter';

export const colorShaders: ShaderEntry[] = [
  {
    label: 'Invert',
    code: `uniform sampler2D inputImage;

out vec4 fragColor;

void main() {
  vec4 col = texture(inputImage, v_uv);
  fragColor = vec4(1.0 - col.rgb, col.a);
}`,
  },
  {
    label: 'Grayscale',
    code: `uniform sampler2D inputImage;

out vec4 fragColor;

void main() {
  vec4 col = texture(inputImage, v_uv);
  float gray = dot(col.rgb, vec3(0.299, 0.587, 0.114));
  fragColor = vec4(vec3(gray), col.a);
}`,
  },
  {
    label: 'Brightness/Contrast',
    code: `uniform sampler2D inputImage;
uniform float brightness;
uniform float contrast;

out vec4 fragColor;

void main() {
  vec4 c = texture(inputImage, v_uv);
  c.rgb = (c.rgb - 0.5) * max(contrast, 0.0) + 0.5 + brightness;
  fragColor = vec4(clamp(c.rgb, 0.0, 1.0), c.a);
}`,
  },
  {
    label: 'Hue Rotate',
    code: `uniform sampler2D inputImage;
uniform float angle;

out vec4 fragColor;

vec3 rgb2hsv(vec3 c) {
  vec4 K = vec4(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
  vec4 p = mix(vec4(c.bg, K.wz), vec4(c.gb, K.xy), step(c.b, c.g));
  vec4 q = mix(vec4(p.xyw, c.r), vec4(c.r, p.yzx), step(p.x, c.r));
  float d = q.x - min(q.w, q.y);
  float e = 1.0e-10;
  return vec3(abs(q.z + (q.w - q.y) / (6.0 * d + e)), d / (q.x + e), q.x);
}

vec3 hsv2rgb(vec3 c) {
  vec4 K = vec4(1.0, 2.0 / 3.0, 1.0 / 3.0, 3.0);
  vec3 p = abs(fract(c.xxx + K.xyz) * 6.0 - K.www);
  return c.z * mix(K.xxx, clamp(p - K.xxx, 0.0, 1.0), c.y);
}

void main() {
  vec4 col = texture(inputImage, v_uv);
  vec3 hsv = rgb2hsv(col.rgb);
  hsv.x = fract(hsv.x + angle / 6.28318530718);
  fragColor = vec4(hsv2rgb(hsv), col.a);
}`,
  },
  {
    label: 'Threshold',
    code: `uniform sampler2D inputImage;
uniform float threshold;

out vec4 fragColor;

void main() {
  vec4 col = texture(inputImage, v_uv);
  float lum = dot(col.rgb, vec3(0.299, 0.587, 0.114));
  float bw = step(threshold, lum);
  fragColor = vec4(vec3(bw), col.a);
}`,
  },
  {
    label: 'Sepia',
    code: `uniform sampler2D inputImage;

out vec4 fragColor;

void main() {
  vec4 col = texture(inputImage, v_uv);
  float r = dot(col.rgb, vec3(0.393, 0.769, 0.189));
  float g = dot(col.rgb, vec3(0.349, 0.686, 0.168));
  float b = dot(col.rgb, vec3(0.272, 0.534, 0.131));
  fragColor = vec4(min(r, 1.0), min(g, 1.0), min(b, 1.0), col.a);
}`,
  },
];
