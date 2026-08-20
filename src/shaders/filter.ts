export interface ShaderEntry {
  label: string;
  code: string;
}

export const filterShaders: ShaderEntry[] = [
  {
    label: 'Resample',
    code: `
// Passes the input image through unchanged (identity/resample).
@group(0) @binding(0) var inputImage: texture_2d<f32>;
@group(0) @binding(1) var inputImageSampler: sampler;

@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  return textureSample(inputImage, inputImageSampler, v_uv);
}`,
  },
  {
    label: 'Sobel Edge Detection',
    code: `
// Detects edges using the Sobel operator.
@group(0) @binding(0) var inputImage: texture_2d<f32>;
@group(0) @binding(1) var inputImageSampler: sampler;
@group(0) @binding(2) var<uniform> intensity: f32; // Edge strength multiplier. 0–5, default 1

@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let px = 1.0 / vec2f(textureDimensions(inputImage));

  let tl = dot(textureSample(inputImage, inputImageSampler, v_uv + vec2f(-px.x, -px.y)).rgb, vec3f(0.299, 0.587, 0.114));
  let t  = dot(textureSample(inputImage, inputImageSampler, v_uv + vec2f(  0.0, -px.y)).rgb, vec3f(0.299, 0.587, 0.114));
  let tr = dot(textureSample(inputImage, inputImageSampler, v_uv + vec2f( px.x, -px.y)).rgb, vec3f(0.299, 0.587, 0.114));
  let l  = dot(textureSample(inputImage, inputImageSampler, v_uv + vec2f(-px.x,   0.0)).rgb, vec3f(0.299, 0.587, 0.114));
  let r  = dot(textureSample(inputImage, inputImageSampler, v_uv + vec2f( px.x,   0.0)).rgb, vec3f(0.299, 0.587, 0.114));
  let bl = dot(textureSample(inputImage, inputImageSampler, v_uv + vec2f(-px.x,  px.y)).rgb, vec3f(0.299, 0.587, 0.114));
  let b  = dot(textureSample(inputImage, inputImageSampler, v_uv + vec2f(  0.0,  px.y)).rgb, vec3f(0.299, 0.587, 0.114));
  let br = dot(textureSample(inputImage, inputImageSampler, v_uv + vec2f( px.x,  px.y)).rgb, vec3f(0.299, 0.587, 0.114));

  let gx = -tl - 2.0 * l - bl + tr + 2.0 * r + br;
  let gy = -tl - 2.0 * t - tr + bl + 2.0 * b + br;
  let edge = sqrt(gx * gx + gy * gy) * intensity;

  return vec4f(vec3f(edge), 1.0);
}`,
  },
  {
    label: 'Gaussian Blur 3x3',
    code: `
// Applies a 3×3 Gaussian blur kernel.
@group(0) @binding(0) var inputImage: texture_2d<f32>;
@group(0) @binding(1) var inputImageSampler: sampler;

@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let px = 1.0 / vec2f(textureDimensions(inputImage));

  var col = vec4f(0.0);
  col += textureSample(inputImage, inputImageSampler, v_uv + vec2f(-px.x, -px.y)) * 0.0625;
  col += textureSample(inputImage, inputImageSampler, v_uv + vec2f( 0.0,  -px.y)) * 0.125;
  col += textureSample(inputImage, inputImageSampler, v_uv + vec2f( px.x, -px.y)) * 0.0625;
  col += textureSample(inputImage, inputImageSampler, v_uv + vec2f(-px.x,  0.0))  * 0.125;
  col += textureSample(inputImage, inputImageSampler, v_uv)                        * 0.25;
  col += textureSample(inputImage, inputImageSampler, v_uv + vec2f( px.x,  0.0))  * 0.125;
  col += textureSample(inputImage, inputImageSampler, v_uv + vec2f(-px.x,  px.y)) * 0.0625;
  col += textureSample(inputImage, inputImageSampler, v_uv + vec2f( 0.0,   px.y)) * 0.125;
  col += textureSample(inputImage, inputImageSampler, v_uv + vec2f( px.x,  px.y)) * 0.0625;

  return col;
}`,
  },
  {
    label: 'Box Blur',
    code: `
// Applies a 3×3 box (mean) blur.
@group(0) @binding(0) var inputImage: texture_2d<f32>;
@group(0) @binding(1) var inputImageSampler: sampler;

@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let px = 1.0 / vec2f(textureDimensions(inputImage));

  var col = vec4f(0.0);
  col += textureSample(inputImage, inputImageSampler, v_uv + vec2f(-1.0, -1.0) * px);
  col += textureSample(inputImage, inputImageSampler, v_uv + vec2f( 0.0, -1.0) * px);
  col += textureSample(inputImage, inputImageSampler, v_uv + vec2f( 1.0, -1.0) * px);
  col += textureSample(inputImage, inputImageSampler, v_uv + vec2f(-1.0,  0.0) * px);
  col += textureSample(inputImage, inputImageSampler, v_uv);
  col += textureSample(inputImage, inputImageSampler, v_uv + vec2f( 1.0,  0.0) * px);
  col += textureSample(inputImage, inputImageSampler, v_uv + vec2f(-1.0,  1.0) * px);
  col += textureSample(inputImage, inputImageSampler, v_uv + vec2f( 0.0,  1.0) * px);
  col += textureSample(inputImage, inputImageSampler, v_uv + vec2f( 1.0,  1.0) * px);
  return col / 9.0;
}`,
  },
  {
    label: 'Sharpen',
    code: `
// Sharpens the image using unsharp masking.
@group(0) @binding(0) var inputImage: texture_2d<f32>;
@group(0) @binding(1) var inputImageSampler: sampler;
@group(0) @binding(2) var<uniform> strength: f32; // Sharpening intensity. 0–10, default 1

@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let px = 1.0 / vec2f(textureDimensions(inputImage));

  let center = textureSample(inputImage, inputImageSampler, v_uv);
  var blur =
    textureSample(inputImage, inputImageSampler, v_uv + vec2f(-px.x, -px.y)) +
    textureSample(inputImage, inputImageSampler, v_uv + vec2f( 0.0,  -px.y)) +
    textureSample(inputImage, inputImageSampler, v_uv + vec2f( px.x, -px.y)) +
    textureSample(inputImage, inputImageSampler, v_uv + vec2f(-px.x,  0.0))  +
    textureSample(inputImage, inputImageSampler, v_uv + vec2f( px.x,  0.0))  +
    textureSample(inputImage, inputImageSampler, v_uv + vec2f(-px.x,  px.y)) +
    textureSample(inputImage, inputImageSampler, v_uv + vec2f( 0.0,   px.y)) +
    textureSample(inputImage, inputImageSampler, v_uv + vec2f( px.x,  px.y));
  blur /= 8.0;

  return center + (center - blur) * strength;
}`,
  },
  {
    label: 'Emboss',
    code: `
// Applies an emboss (relief) effect.
@group(0) @binding(0) var inputImage: texture_2d<f32>;
@group(0) @binding(1) var inputImageSampler: sampler;

@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let px = 1.0 / vec2f(textureDimensions(inputImage));

  let col =
    textureSample(inputImage, inputImageSampler, v_uv + vec2f(-px.x, -px.y)) -
    textureSample(inputImage, inputImageSampler, v_uv + vec2f( px.x,  px.y));
  let gray = dot(col.rgb, vec3f(0.299, 0.587, 0.114)) + 0.5;

  return vec4f(vec3f(gray), 1.0);
}`,
  },
  {
    label: 'Pixelate',
    code: `
// Reduces resolution into blocky pixels.
@group(0) @binding(0) var inputImage: texture_2d<f32>;
@group(0) @binding(1) var inputImageSampler: sampler;
@group(0) @binding(2) var<uniform> blockSize: vec2f; // Block size in pixels (width, height). Default (8, 8)

@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let size = vec2f(textureDimensions(inputImage));
  let blocks = size / max(blockSize, vec2f(1.0));
  let uv = floor(v_uv * blocks) / blocks;
  return textureSample(inputImage, inputImageSampler, uv);
}`,
  },
];
