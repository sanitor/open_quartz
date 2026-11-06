import { readFileSync } from 'node:fs';
import { deflateSync, inflateSync } from 'node:zlib';
import assert from 'node:assert/strict';
const catalog = JSON.parse(readFileSync(new URL('../crates/open_quartz/src/project/shader_catalog.json', import.meta.url), 'utf8'));
export const CATALOG_SHADERS = catalog.groups.flatMap((group) => group.items.map((item) => ({ ...item, category: group.category })));
export const CATALOG_SHADER_LABELS = CATALOG_SHADERS.map((item) => item.label);

const WIDTH = 64;
const HEIGHT = 64;
const CHANNELS = 4;
const TAU = Math.PI * 2;
const clamp01 = (value) => Math.min(1, Math.max(0, value));
const fract = (value) => value - Math.floor(value);
const mix = (a, b, t) => a * (1 - t) + b * t;
const vec = (value, size) => Array.isArray(value) ? value : Array(size).fill(Number(value));
const scalar = (uniforms, name, fallback = 0) => {
  const value = uniforms?.[name];
  return Array.isArray(value) ? Number(value[0] ?? fallback) : Number(value ?? fallback);
};
const vector = (uniforms, name, size, fallback) => {
  const value = uniforms?.[name];
  return (Array.isArray(value) ? value : vec(Number(value ?? fallback), size)).slice(0, size).map(Number);
};

function normaliseImage(image) {
  if (!image) return null;
  if (image.pixels && image.width && image.height) return image;
  if (Buffer.isBuffer(image) || image instanceof Uint8Array) return decodePng(Buffer.from(image));
  throw new TypeError('Reference images must be decoded RGBA images or PNG buffers');
}

function sample(image, u, v) {
  const x = Math.min(image.width - 1, Math.max(0, Math.floor(u * image.width)));
  const y = Math.min(image.height - 1, Math.max(0, Math.floor(v * image.height)));
  const offset = (y * image.width + x) * 4;
  return [image.pixels[offset] / 255, image.pixels[offset + 1] / 255, image.pixels[offset + 2] / 255, image.pixels[offset + 3] / 255];
}

function sampleTex(image, u, v) {
  return sample(image, clamp01(u), clamp01(v));
}

function luma(color) {
  return color[0] * 0.299 + color[1] * 0.587 + color[2] * 0.114;
}

function hash2(x, y) {
  return fract(Math.sin(x * 127.1 + y * 311.7) * 43758.5453123);
}

function valueNoise(x, y) {
  const ix = Math.floor(x), iy = Math.floor(y);
  const fx = fract(x), fy = fract(y);
  const ux = fx * fx * (3 - 2 * fx), uy = fy * fy * (3 - 2 * fy);
  const a = hash2(ix, iy), b = hash2(ix + 1, iy), c = hash2(ix, iy + 1), d = hash2(ix + 1, iy + 1);
  return mix(mix(a, b, ux), mix(c, d, ux), uy);
}

function hsvToRgb(h, s, v) {
  const i = Math.floor(h * 6);
  const f = h * 6 - i;
  const p = v * (1 - s), q = v * (1 - f * s), t = v * (1 - (1 - f) * s);
  switch (i % 6) {
    case 0: return [v, t, p];
    case 1: return [q, v, p];
    case 2: return [p, v, t];
    case 3: return [p, q, v];
    case 4: return [t, p, v];
    default: return [v, p, q];
  }
}

function rgbToHsv([r, g, b]) {
  const max = Math.max(r, g, b), min = Math.min(r, g, b), d = max - min;
  let h = 0;
  if (d) {
    if (max === r) h = ((g - b) / d) % 6;
    else if (max === g) h = (b - r) / d + 2;
    else h = (r - g) / d + 4;
    h /= 6;
    if (h < 0) h += 1;
  }
  return [h, max ? d / max : 0, max];
}

function turbo(t) {
  const stops = [[0.114, 0.056, 0.566], [0.376, 0.763, 0.843], [0.267, 0.472, 0.090], [0.905, 0.811, 0.011], [0.740, 0.080, 0.055]];
  const position = clamp01(t) * 4;
  const index = Math.min(3, Math.floor(position));
  const amount = position - index;
  return stops[index].map((value, i) => mix(value, stops[index + 1][i], amount));
}

const pixelOps = {
  'CUSTOM SHADER': ({ a, uniforms }) => {
    const intensity = scalar(uniforms, 'intensity', 1);
    return [a[0] * intensity, a[1] * intensity, a[2] * intensity, a[3]];
  },
  'CUSTOM 2IN-1': ({ a, b, uniforms }) => {
    const t = scalar(uniforms, 'mixFactor', 0.5);
    return a.map((value, i) => mix(value, b[i], t));
  },
  Resample: ({ a }) => a,
  'Sobel Edge Detection': ({ image, uv, uniforms }) => {
    const px = 1 / image.width, py = 1 / image.height;
    const tl = luma(sampleTex(image, uv[0] - px, uv[1] - py));
    const t = luma(sampleTex(image, uv[0], uv[1] - py));
    const tr = luma(sampleTex(image, uv[0] + px, uv[1] - py));
    const left = luma(sampleTex(image, uv[0] - px, uv[1]));
    const right = luma(sampleTex(image, uv[0] + px, uv[1]));
    const bl = luma(sampleTex(image, uv[0] - px, uv[1] + py));
    const bottom = luma(sampleTex(image, uv[0], uv[1] + py));
    const br = luma(sampleTex(image, uv[0] + px, uv[1] + py));
    const gx = -tl - 2 * left - bl + tr + 2 * right + br;
    const gy = -tl - 2 * t - tr + bl + 2 * bottom + br;
    const edge = Math.hypot(gx, gy) * scalar(uniforms, 'intensity', 1);
    return [edge, edge, edge, 1];
  },
  'Gaussian Blur 3x3': ({ image, uv }) => {
    const px = 1 / image.width, py = 1 / image.height;
    const weights = [0.0625, 0.125, 0.0625, 0.125, 0.25, 0.125, 0.0625, 0.125, 0.0625];
    const offsets = [[-px, -py], [0, -py], [px, -py], [-px, 0], [0, 0], [px, 0], [-px, py], [0, py], [px, py]];
    const out = [0, 0, 0, 0];
    offsets.forEach(([dx, dy], i) => sampleTex(image, uv[0] + dx, uv[1] + dy).forEach((value, channel) => { out[channel] += value * weights[i]; }));
    return out;
  },
  'Box Blur': ({ image, uv }) => {
    const px = 1 / image.width, py = 1 / image.height;
    const out = [0, 0, 0, 0];
    for (let y = -1; y <= 1; y++) for (let x = -1; x <= 1; x++) sampleTex(image, uv[0] + x * px, uv[1] + y * py).forEach((value, channel) => { out[channel] += value / 9; });
    return out;
  },
  Sharpen: ({ image, uv, a, uniforms }) => {
    const px = 1 / image.width, py = 1 / image.height;
    const blur = [0, 0, 0, 0];
    for (let y = -1; y <= 1; y++) for (let x = -1; x <= 1; x++) if (x || y) sampleTex(image, uv[0] + x * px, uv[1] + y * py).forEach((value, channel) => { blur[channel] += value / 8; });
    const strength = scalar(uniforms, 'strength', 1);
    return a.map((value, i) => value + (value - blur[i]) * strength);
  },
  Emboss: ({ image, uv }) => {
    const px = 1 / image.width, py = 1 / image.height;
    const hi = sampleTex(image, uv[0] - px, uv[1] - py), lo = sampleTex(image, uv[0] + px, uv[1] + py);
    const gray = luma([hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]]) + 0.5;
    return [gray, gray, gray, 1];
  },
  Pixelate: ({ image, uv, uniforms }) => {
    const size = vector(uniforms, 'blockSize', 2, 8);
    const blocksX = image.width / Math.max(size[0], 1), blocksY = image.height / Math.max(size[1], 1);
    return sampleTex(image, Math.floor(uv[0] * blocksX) / blocksX, Math.floor(uv[1] * blocksY) / blocksY);
  },
  Invert: ({ a }) => [1 - a[0], 1 - a[1], 1 - a[2], a[3]],
  Grayscale: ({ a }) => { const g = luma(a); return [g, g, g, a[3]]; },
  'Brightness/Contrast': ({ a, uniforms }) => {
    const brightness = scalar(uniforms, 'brightness', 0), contrast = Math.max(scalar(uniforms, 'contrast', 1), 0);
    return [0, 1, 2].map((i) => clamp01((a[i] - 0.5) * contrast + 0.5 + brightness)).concat(a[3]);
  },
  'Hue Rotate': ({ a, uniforms }) => {
    const hsv = rgbToHsv(a), h = fract(hsv[0] + scalar(uniforms, 'angle', 0) / TAU);
    return hsvToRgb(h, hsv[1], hsv[2]).concat(a[3]);
  },
  Threshold: ({ a, uniforms }) => { const v = luma(a) >= scalar(uniforms, 'threshold', 0.5) ? 1 : 0; return [v, v, v, a[3]]; },
  Sepia: ({ a }) => [Math.min(1, a[0] * 0.393 + a[1] * 0.769 + a[2] * 0.189), Math.min(1, a[0] * 0.349 + a[1] * 0.686 + a[2] * 0.168), Math.min(1, a[0] * 0.272 + a[1] * 0.534 + a[2] * 0.131), a[3]],
  'Field Color Map': ({ a }) => turbo(a[1]).concat(1),
  'Solid Color': ({ uniforms }) => vector(uniforms, 'color', 4, 0),
  Gradient: ({ uv, uniforms }) => {
    const left = vector(uniforms, 'colorA', 4, 0), right = vector(uniforms, 'colorB', 4, 1);
    return left.map((value, i) => mix(value, right[i], uv[0]));
  },
  Checkerboard: ({ uv, uniforms }) => {
    const grid = vector(uniforms, 'gridSize', 2, 8), c1 = vector(uniforms, 'color1', 4, 0), c2 = vector(uniforms, 'color2', 4, 1);
    const checker = (Math.floor(uv[0] * Math.max(grid[0], 1)) + Math.floor(uv[1] * Math.max(grid[1], 1))) % 2;
    return c1.map((value, i) => mix(value, c2[i], checker));
  },
  Noise: ({ uv, uniforms }) => { const n = valueNoise(uv[0] * Math.max(scalar(uniforms, 'scale', 10), 1), uv[1] * Math.max(scalar(uniforms, 'scale', 10), 1)); return [n, n, n, 1]; },
  Circle: ({ uv, uniforms }) => {
    const circle = vector(uniforms, 'circle', 4, 0.5), distance = Math.hypot(uv[0] - circle[0], uv[1] - circle[1]);
    const mask = distance < circle[2] ? 1 : 0;
    return [mask, mask, mask, 1];
  },
  Add: ({ a, b }) => a.map((value, i) => Math.min(1, value + b[i])),
  Multiply: ({ a, b }) => a.map((value, i) => value * b[i]),
  Screen: ({ a, b }) => a.map((value, i) => 1 - (1 - value) * (1 - b[i])),
  Overlay: ({ a, b }) => {
    const rgb = [0, 1, 2].map((i) => a[i] < 0.5 ? 2 * a[i] * b[i] : 1 - 2 * (1 - a[i]) * (1 - b[i]));
    return rgb.concat(Math.max(a[3], b[3]));
  },
  Difference: ({ a, b }) => [0, 1, 2].map((i) => Math.abs(a[i] - b[i])).concat(Math.max(a[3], b[3])),
  Exclusion: ({ a, b }) => [0, 1, 2].map((i) => a[i] + b[i] - 2 * a[i] * b[i]).concat(Math.max(a[3], b[3])),
  'Soft Light': ({ a, b }) => [0, 1, 2].map((i) => b[i] < 0.5 ? 2 * a[i] * b[i] + a[i] * a[i] * (1 - 2 * b[i]) : 2 * a[i] * (1 - b[i]) + Math.sqrt(Math.max(0, a[i])) * (2 * b[i] - 1)).concat(Math.max(a[3], b[3])),
  Twirl: ({ image, uv, uniforms }) => {
    const radius = Math.max(scalar(uniforms, 'radius', 0.5), 0.001), angle = scalar(uniforms, 'angle', 3);
    let x = uv[0] - 0.5, y = uv[1] - 0.5; const dist = Math.hypot(x, y), factor = Math.max(1 - dist / radius, 0); const a = angle * factor * factor;
    const s = Math.sin(a), c = Math.cos(a); [x, y] = [x * c - y * s, x * s + y * c];
    return sampleTex(image, x + 0.5, y + 0.5);
  },
  Ripple: ({ image, uv, uniforms }) => {
    const frequency = scalar(uniforms, 'frequency', 10), amplitude = scalar(uniforms, 'amplitude', 0.02);
    let x = uv[0] + Math.sin(uv[1] * frequency) * amplitude;
    let y = uv[1] + Math.sin(x * frequency) * amplitude;
    return sampleTex(image, x, y);
  },
  Displacement: ({ image, displacement, uv, uniforms }) => {
    const strength = scalar(uniforms, 'strength', 0.1), disp = sampleTex(displacement, uv[0], uv[1]);
    return sampleTex(image, uv[0] + (disp[0] - 0.5) * 2 * strength, uv[1] + (disp[1] - 0.5) * 2 * strength);
  },
  Barrel: ({ image, uv, uniforms }) => {
    const k1 = scalar(uniforms, 'k1', 0), k2 = scalar(uniforms, 'k2', 0), x = uv[0] * 2 - 1, y = uv[1] * 2 - 1, r2 = x * x + y * y, distortion = 1 + k1 * r2 + k2 * r2 * r2, u = x * distortion * 0.5 + 0.5, v = y * distortion * 0.5 + 0.5;
    return u < 0 || u > 1 || v < 0 || v > 1 ? [0, 0, 0, 0] : sampleTex(image, u, v);
  },
  Pinch: ({ image, uv, uniforms }) => {
    const radius = Math.max(scalar(uniforms, 'radius', 0.5), 0.001), strength = scalar(uniforms, 'strength', 0.5);
    let x = uv[0] - 0.5, y = uv[1] - 0.5; const dist = Math.hypot(x, y);
    if (dist < radius && dist > 1e-9) { const pinch = Math.pow(dist / radius, 1 + strength) * radius; x *= pinch / dist; y *= pinch / dist; }
    return sampleTex(image, x + 0.5, y + 0.5);
  },
  'Gray-Scott Reaction-Diffusion': ({ uv }) => {
    const inside = Math.abs(uv[0] - 0.5) < 0.06 && Math.abs(uv[1] - 0.5) < 0.06;
    return inside ? [0.5, 0.25, 0, 1] : [1, 0, 0, 1];
  },
};

export const REFERENCE_LABELS = Object.freeze(Object.keys(pixelOps));

function renderPixel(label, uv, images, uniforms) {
  const imageA = images[0] ?? null, imageB = images[1] ?? null;
  const operation = pixelOps[label];
  assert.equal(typeof operation, 'function', `No independent reference oracle registered for catalog shader ${label}`);
  const a = imageA ? sampleTex(imageA, uv[0], uv[1]) : [0, 0, 0, 1];
  const b = imageB ? sampleTex(imageB, uv[0], uv[1]) : [0, 0, 0, 1];
  return operation({ label, uv, image: label === 'Displacement' ? imageB : imageA, displacement: imageA, a, b, uniforms });
}

/**
 * CPU oracle. `images` are ordered by the shader's texture declarations. It
 * deliberately never evaluates or parses the WGSL source.
 */
function feedbackSeed() {
  const a = new Float64Array(WIDTH * HEIGHT);
  const b = new Float64Array(WIDTH * HEIGHT);
  for (let y = 0; y < HEIGHT; y++) for (let x = 0; x < WIDTH; x++) {
    const index = y * WIDTH + x;
    const inside = Math.abs((x + 0.5) / WIDTH - 0.5) < 0.06 && Math.abs((y + 0.5) / HEIGHT - 0.5) < 0.06;
    // The runtime stores each feedback frame in rgba8unorm, including the seed.
    a[index] = inside ? 128 / 255 : 1;
    b[index] = inside ? 64 / 255 : 0;
  }
  return { a, b };
}

function feedbackFrame(uniforms = {}, frame = 0) {
  let { a, b } = feedbackSeed();
  const dA = scalar(uniforms, 'dA', 0.21);
  const dB = scalar(uniforms, 'dB', 0.105);
  const feed = scalar(uniforms, 'feedRate', 0.055);
  const kill = scalar(uniforms, 'killRate', 0.062);
  const timestep = scalar(uniforms, 'timestep', 1);
  for (let step = 0; step < frame; step++) {
    const nextA = new Float64Array(a.length);
    const nextB = new Float64Array(b.length);
    for (let y = 0; y < HEIGHT; y++) for (let x = 0; x < WIDTH; x++) {
      const index = y * WIDTH + x;
      const up = ((y === 0 ? y : y - 1) * WIDTH) + x;
      const down = ((y === HEIGHT - 1 ? y : y + 1) * WIDTH) + x;
      const left = y * WIDTH + (x === 0 ? x : x - 1);
      const right = y * WIDTH + (x === WIDTH - 1 ? x : x + 1);
      const laplaceA = a[up] + a[down] + a[left] + a[right] - 4 * a[index];
      const laplaceB = b[up] + b[down] + b[left] + b[right] - 4 * b[index];
      const reaction = a[index] * b[index] * b[index];
      nextA[index] = Math.round(clamp01(a[index] + (dA * laplaceA - reaction + feed * (1 - a[index])) * timestep) * 255) / 255;
      nextB[index] = Math.round(clamp01(b[index] + (dB * laplaceB + reaction - (feed + kill) * b[index]) * timestep) * 255) / 255;
    }
    a = nextA;
    b = nextB;
  }
  const pixels = Buffer.alloc(WIDTH * HEIGHT * CHANNELS);
  for (let index = 0; index < a.length; index++) {
    const offset = index * CHANNELS;
    pixels[offset] = Math.round(a[index] * 255);
    pixels[offset + 1] = Math.round(b[index] * 255);
    pixels[offset + 2] = 0;
    pixels[offset + 3] = 255;
  }
  return { width: WIDTH, height: HEIGHT, pixels };
}

export function makeFeedbackReference(uniforms, frame = 0) {
  assert.ok(Number.isInteger(frame) && frame >= 0, 'feedback frame must be a non-negative integer');
  return feedbackFrame(uniforms, frame);
}

export function makeReference(label, images = [], uniforms = {}, options = {}) {
  assert.ok(REFERENCE_LABELS.includes(label), `Unknown shader reference: ${label}`);
  const normalized = images.map(normaliseImage);
  const pixels = Buffer.alloc(WIDTH * HEIGHT * CHANNELS);
  for (let y = 0; y < HEIGHT; y++) for (let x = 0; x < WIDTH; x++) {
    const uv = [(x + 0.5) / WIDTH, (y + 0.5) / HEIGHT];
    const value = renderPixel(label, uv, normalized, uniforms).map((channel) => Number.isFinite(channel) ? channel : 0);
    const offset = (y * WIDTH + x) * CHANNELS;
    for (let i = 0; i < CHANNELS; i++) pixels[offset + i] = Math.round(clamp01(value[i] ?? 0) * 255);
  }
  const noise = label === 'Noise';
  const feedback = label === 'Gray-Scott Reaction-Diffusion';
  const oracle = noise ? {
    kind: 'statistical-noise',
    exact: false,
    reason: 'GPU sin/hash evaluation is not bit-identical to CPU JavaScript; require grayscale, non-constant output, matching mean/variance and adjacent-pixel RMS gradient at the configured frequency. Per-pixel errors are diagnostic, not an equality oracle.',
    tolerance: 14,
    requireGrayscale: true,
    minStandardDeviation: 0.04,
  } : feedback ? {
    kind: 'feedback-dynamic',
    exact: false,
    seedFrame: 0,
    minFrame: 1,
    maxFrame: 200,
    reason: 'The first rendered frame is the deterministic iFrame-zero seed; paused output after Play is matched against independently simulated nonzero Gray-Scott frames.',
    tolerance: 8,
  } : { kind: 'exact', exact: true, tolerance: 6 };
  return { width: WIDTH, height: HEIGHT, pixels, label, uniforms: structuredClone(uniforms), oracle, frame: options.frame ?? 0 };
}

function crc32(buffer) {
  let crc = 0xffffffff;
  for (const byte of buffer) { crc ^= byte; for (let i = 0; i < 8; i++) crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1)); }
  return (crc ^ 0xffffffff) >>> 0;
}
function chunk(type, body) {
  const typeBuffer = Buffer.from(type);
  const output = Buffer.alloc(12 + body.length);
  output.writeUInt32BE(body.length, 0); typeBuffer.copy(output, 4); body.copy(output, 8); output.writeUInt32BE(crc32(Buffer.concat([typeBuffer, body])), 8 + body.length);
  return output;
}

/** Encode a non-interlaced RGBA8 PNG using only Node built-ins. */
export function encodePng({ width, height, pixels }) {
  assert.equal(width * height * 4, pixels.length, 'RGBA pixel buffer dimensions do not match');
  const rows = Buffer.alloc(height * (width * 4 + 1));
  for (let y = 0; y < height; y++) { rows[y * (width * 4 + 1)] = 0; Buffer.from(pixels).copy(rows, y * (width * 4 + 1) + 1, y * width * 4, (y + 1) * width * 4); }
  const header = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(width, 0); ihdr.writeUInt32BE(height, 4); ihdr[8] = 8; ihdr[9] = 6;
  return Buffer.concat([header, chunk('IHDR', ihdr), chunk('IDAT', deflateSync(rows, { level: 9 })), chunk('IEND', Buffer.alloc(0))]);
}

/** Decode the exact PNG form emitted above and ordinary 8-bit PNG screenshots. */
export function decodePng(png) {
  const bytes = Buffer.from(png); assert.ok(bytes.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10])), 'Invalid PNG signature');
  let width = 0, height = 0, bitDepth = 0, colorType = 0, interlace = 0; const idat = []; let offset = 8;
  while (offset + 12 <= bytes.length) { const length = bytes.readUInt32BE(offset); const type = bytes.toString('ascii', offset + 4, offset + 8); const body = bytes.subarray(offset + 8, offset + 8 + length); offset += length + 12; if (type === 'IHDR') { width = body.readUInt32BE(0); height = body.readUInt32BE(4); bitDepth = body[8]; colorType = body[9]; interlace = body[12]; } else if (type === 'IDAT') idat.push(body); else if (type === 'IEND') break; }
  assert.equal(bitDepth, 8, 'Only 8-bit PNG is supported'); assert.equal(interlace, 0, 'Interlaced PNG is not supported'); assert.equal(colorType, 6, 'Expected RGBA PNG');
  const raw = inflateSync(Buffer.concat(idat)); const stride = width * 4; assert.equal(raw.length, height * (stride + 1), 'Invalid PNG scanline length'); const pixels = Buffer.alloc(width * height * 4);
  const paeth = (a, b, c) => { const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c); return pa <= pb && pa <= pc ? a : pb <= pc ? b : c; };
  for (let y = 0; y < height; y++) { const filter = raw[y * (stride + 1)]; const row = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1)); const out = pixels.subarray(y * stride, (y + 1) * stride); const prior = y ? pixels.subarray((y - 1) * stride, y * stride) : null; for (let x = 0; x < stride; x++) { const left = x >= 4 ? out[x - 4] : 0, up = prior?.[x] ?? 0, upperLeft = prior?.[x - 4] ?? 0; out[x] = (row[x] + (filter === 1 ? left : filter === 2 ? up : filter === 3 ? Math.floor((left + up) / 2) : filter === 4 ? paeth(left, up, upperLeft) : 0)) & 255; } }
  return { width, height, pixels };
}


// The window scales the 64x64 texture with linear filtering. Its physical pixel
// centers need not coincide with texel centers, even when sampling each texel.
function displayedReferencePixel(expected, x, y, roi) {
  const sourceX = roi ? (Math.floor(roi.left + (x + 0.5) * roi.width / WIDTH) + 0.5 - roi.left) / roi.width * WIDTH - 0.5 : x;
  const sourceY = roi ? (Math.floor(roi.top + (y + 0.5) * roi.height / HEIGHT) + 0.5 - roi.top) / roi.height * HEIGHT - 0.5 : y;
  const left = Math.floor(sourceX), top = Math.floor(sourceY);
  const fx = sourceX - left, fy = sourceY - top;
  const premultiplied = [0, 0, 0];
  let alpha = 0;
  for (let dy = 0; dy < 2; dy++) for (let dx = 0; dx < 2; dx++) {
    const offset = (Math.max(0, Math.min(HEIGHT - 1, top + dy)) * WIDTH + Math.max(0, Math.min(WIDTH - 1, left + dx))) * CHANNELS;
    const weight = (dx ? fx : 1 - fx) * (dy ? fy : 1 - fy);
    const a = expected.pixels[offset + 3] / 255;
    alpha += a * weight;
    for (let channel = 0; channel < 3; channel++) premultiplied[channel] += expected.pixels[offset + channel] * a * weight;
  }
  return premultiplied.map((value, channel) => value + [245, 245, 247][channel] * (1 - alpha));
}

/** Compare an already sampled 64×64 renderer image with one prepared case. */
export function compareShaderOutput(entry, actual) {
  assert.ok(actual && actual.width === WIDTH && actual.height === HEIGHT, 'actual shader output must be 64×64');
  const pixels = Buffer.from(actual.pixels);
  assert.equal(pixels.length, WIDTH * HEIGHT * CHANNELS, 'actual shader output must be RGBA');
  const oracle = entry.oracle ?? { kind: 'exact', exact: true, tolerance: 6 };
  if (oracle.kind === 'feedback-dynamic') {
    let best = null;
    for (let frame = oracle.minFrame ?? 1; frame <= (oracle.maxFrame ?? 200); frame++) {
      const candidate = makeFeedbackReference(entry.uniforms ?? {}, frame);
      const result = compareShaderOutput({ ...entry, expected: candidate, oracle: { kind: 'exact', exact: true, tolerance: oracle.tolerance ?? 8 } }, actual);
      if (!best || result.meanError < best.meanError) best = { ...result, matchedFrame: frame };
      if (result.verified) break;
    }
    assert.ok(best, 'feedback comparison did not evaluate any evolution frame');
    return { ...best, oracle: { ...oracle, matchedFrame: best.matchedFrame, seededSimulationMatched: best.verified, evolutionChecked: best.verified && best.matchedFrame > 0 } };
  }
  const expected = entry.expected ?? decodePng(readFileSync(entry.expectedPath));
  assert.equal(expected.width, WIDTH);
  assert.equal(expected.height, HEIGHT);
  const tolerance = oracle.tolerance ?? 6;
  let matchedPixels = 0;
  let maxError = 0;
  let sumError = 0;
  const actualValues = [];
  const expectedValues = [];
  for (let offset = 0; offset < pixels.length; offset += CHANNELS) {
    const wanted = displayedReferencePixel(expected, (offset / CHANNELS) % WIDTH, Math.floor(offset / CHANNELS / WIDTH), actual.roi);
    const observed = [pixels[offset], pixels[offset + 1], pixels[offset + 2]];
    const error = Math.max(...observed.map((value, channel) => Math.abs(value - wanted[channel])));
    maxError = Math.max(maxError, error);
    sumError += error;
    if (error <= tolerance) matchedPixels++;
    actualValues.push(observed);
    expectedValues.push(wanted);
  }
  const totalPixels = WIDTH * HEIGHT;
  const meanError = sumError / totalPixels;
  let verified = matchedPixels / totalPixels >= 0.985 && maxError <= tolerance;
  if (oracle.kind === 'statistical-noise') {
    let min = 1, max = 0, sum = 0, sum2 = 0, grayscale = true;
    for (const [r, g, b] of actualValues) {
      const value = r / 255;
      min = Math.min(min, value);
      max = Math.max(max, value);
      sum += value;
      sum2 += value * value;
      grayscale &&= Math.max(r, g, b) - Math.min(r, g, b) <= 4;
    }
    const mean = sum / totalPixels;
    const standardDeviation = Math.sqrt(Math.max(0, sum2 / totalPixels - mean * mean));
    const expectedMean = expectedValues.reduce((total, [r]) => total + r / 255, 0) / totalPixels;
    const expectedDeviation = Math.sqrt(expectedValues.reduce((total, [r]) => total + (r / 255 - expectedMean) ** 2, 0) / totalPixels);
    const gradient = (values) => {
      let squared = 0, count = 0;
      for (let y = 0; y < HEIGHT; y++) for (let x = 0; x < WIDTH; x++) {
        const index = y * WIDTH + x;
        for (const next of [x + 1 < WIDTH ? index + 1 : -1, y + 1 < HEIGHT ? index + WIDTH : -1]) {
          if (next < 0) continue;
          squared += ((values[index][0] - values[next][0]) / 255) ** 2;
          count++;
        }
      }
      return Math.sqrt(squared / count);
    };
    const rmsGradient = gradient(actualValues), expectedRmsGradient = gradient(expectedValues);
    verified = grayscale && max > min + 0.12 && standardDeviation >= (oracle.minStandardDeviation ?? 0.04)
      && Math.abs(mean - expectedMean) <= 0.08 && Math.abs(standardDeviation - expectedDeviation) <= 0.06
      && rmsGradient >= expectedRmsGradient * 0.7 && rmsGradient <= expectedRmsGradient * 1.3;
    return { matchedPixels, totalPixels, maxError, meanError, verified, oracle: { ...oracle, observed: { min, max, mean, standardDeviation, expectedMean, expectedDeviation, rmsGradient, expectedRmsGradient } } };
  }
  return { matchedPixels, totalPixels, maxError, meanError, verified, oracle };
}
