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
  {
    label: 'Field Color Map',
    code: `uniform sampler2D inputImage;

out vec4 fragColor;

vec3 turbo(float t) {
    vec3 a = vec3(0.114, 0.056, 0.566);
    vec3 b = vec3(0.376, 0.763, 0.843);
    vec3 c = vec3(0.267, 0.472, 0.090);
    vec3 d = vec3(0.905, 0.811, 0.011);
    vec3 e = vec3(0.740, 0.080, 0.055);

    vec3 r;
    if (t < 0.25) r = mix(a, b, t / 0.25);
    else if (t < 0.5) r = mix(b, c, (t - 0.25) / 0.25);
    else if (t < 0.75) r = mix(c, d, (t - 0.5) / 0.25);
    else r = mix(d, e, (t - 0.75) / 0.25);
    return r;
}

void main() {
    vec4 raw = texture(inputImage, v_uv);
    float v = raw.g;  // Read chemical B from G channel
    vec3 color = turbo(clamp(v, 0.0, 1.0));
    fragColor = vec4(color, 1.0);
}`,
  },
];
