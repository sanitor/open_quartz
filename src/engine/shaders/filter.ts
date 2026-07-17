export interface ShaderEntry {
  label: string;
  code: string;
}

export const filterShaders: ShaderEntry[] = [
  {
    label: 'Resample',
    code: `uniform sampler2D inputImage;

out vec4 fragColor;

void main() {
  fragColor = texture(inputImage, v_uv);
}`,
  },
  {
    label: 'Sobel Edge Detection',
    code: `uniform sampler2D inputImage;
uniform float intensity;

out vec4 fragColor;

void main() {
  vec2 size = vec2(textureSize(inputImage, 0));
  vec2 px = 1.0 / size;

  float tl = dot(texture(inputImage, v_uv + vec2(-px.x, -px.y)).rgb, vec3(0.299, 0.587, 0.114));
  float t  = dot(texture(inputImage, v_uv + vec2(  0.0, -px.y)).rgb, vec3(0.299, 0.587, 0.114));
  float tr = dot(texture(inputImage, v_uv + vec2( px.x, -px.y)).rgb, vec3(0.299, 0.587, 0.114));
  float l  = dot(texture(inputImage, v_uv + vec2(-px.x,   0.0)).rgb, vec3(0.299, 0.587, 0.114));
  float r  = dot(texture(inputImage, v_uv + vec2( px.x,   0.0)).rgb, vec3(0.299, 0.587, 0.114));
  float bl = dot(texture(inputImage, v_uv + vec2(-px.x,  px.y)).rgb, vec3(0.299, 0.587, 0.114));
  float b  = dot(texture(inputImage, v_uv + vec2(  0.0,  px.y)).rgb, vec3(0.299, 0.587, 0.114));
  float br = dot(texture(inputImage, v_uv + vec2( px.x,  px.y)).rgb, vec3(0.299, 0.587, 0.114));

  float gx = -tl - 2.0 * l - bl + tr + 2.0 * r + br;
  float gy = -tl - 2.0 * t - tr + bl + 2.0 * b + br;
  float edge = sqrt(gx * gx + gy * gy) * intensity;

  fragColor = vec4(vec3(edge), 1.0);
}`,
  },
  {
    label: 'Gaussian Blur 3x3',
    code: `uniform sampler2D inputImage;

out vec4 fragColor;

void main() {
  vec2 size = vec2(textureSize(inputImage, 0));
  vec2 px = 1.0 / size;

  vec4 col = vec4(0.0);
  col += texture(inputImage, v_uv + vec2(-px.x, -px.y)) * 0.0625;
  col += texture(inputImage, v_uv + vec2( 0.0,  -px.y)) * 0.125;
  col += texture(inputImage, v_uv + vec2( px.x, -px.y)) * 0.0625;
  col += texture(inputImage, v_uv + vec2(-px.x,  0.0))  * 0.125;
  col += texture(inputImage, v_uv)                       * 0.25;
  col += texture(inputImage, v_uv + vec2( px.x,  0.0))  * 0.125;
  col += texture(inputImage, v_uv + vec2(-px.x,  px.y))  * 0.0625;
  col += texture(inputImage, v_uv + vec2( 0.0,   px.y))  * 0.125;
  col += texture(inputImage, v_uv + vec2( px.x,  px.y))  * 0.0625;

  fragColor = col;
}`,
  },
  {
    label: 'Box Blur',
    code: `uniform sampler2D inputImage;

out vec4 fragColor;

void main() {
  vec2 size = vec2(textureSize(inputImage, 0));
  vec2 px = 1.0 / size;

  vec4 col = vec4(0.0);
  for (float y = -1.0; y <= 1.0; y += 1.0) {
    for (float x = -1.0; x <= 1.0; x += 1.0) {
      col += texture(inputImage, v_uv + vec2(x, y) * px);
    }
  }
  fragColor = col / 9.0;
}`,
  },
  {
    label: 'Sharpen',
    code: `uniform sampler2D inputImage;
uniform float strength;

out vec4 fragColor;

void main() {
  vec2 size = vec2(textureSize(inputImage, 0));
  vec2 px = 1.0 / size;

  vec4 center = texture(inputImage, v_uv);
  vec4 blur =
    texture(inputImage, v_uv + vec2(-px.x, -px.y)) +
    texture(inputImage, v_uv + vec2( 0.0,  -px.y)) +
    texture(inputImage, v_uv + vec2( px.x, -px.y)) +
    texture(inputImage, v_uv + vec2(-px.x,  0.0))  +
    texture(inputImage, v_uv + vec2( px.x,  0.0))  +
    texture(inputImage, v_uv + vec2(-px.x,  px.y)) +
    texture(inputImage, v_uv + vec2( 0.0,   px.y)) +
    texture(inputImage, v_uv + vec2( px.x,  px.y));
  blur /= 8.0;

  fragColor = center + (center - blur) * strength;
}`,
  },
  {
    label: 'Emboss',
    code: `uniform sampler2D inputImage;

out vec4 fragColor;

void main() {
  vec2 size = vec2(textureSize(inputImage, 0));
  vec2 px = 1.0 / size;

  vec4 col =
    texture(inputImage, v_uv + vec2(-px.x, -px.y)) -
    texture(inputImage, v_uv + vec2( px.x,  px.y));
  float gray = dot(col.rgb, vec3(0.299, 0.587, 0.114)) + 0.5;

  fragColor = vec4(vec3(gray), 1.0);
}`,
  },
  {
    label: 'Pixelate',
    code: `uniform sampler2D inputImage;
uniform vec2 blockSize;

out vec4 fragColor;

void main() {
  vec2 size = vec2(textureSize(inputImage, 0));
  vec2 blocks = size / max(blockSize, vec2(1.0));
  vec2 uv = floor(v_uv * blocks) / blocks;
  fragColor = texture(inputImage, uv);
}`,
  },
];
