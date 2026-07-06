import type { FramebufferFormat } from '../types';

export function generateRawPreview(
  rawDataUrl: string,
  format: FramebufferFormat,
  width: number,
  height: number,
  stride?: number,
): string | null {
  try {
    const b64 = rawDataUrl.split(',')[1];
    const binary = atob(b64);
    const src = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) src[i] = binary.charCodeAt(i);

    const canvas = document.createElement('canvas');
    canvas.width = width;
    canvas.height = height;
    const ctx = canvas.getContext('2d')!;
    const imageData = ctx.createImageData(width, height);
    const out = imageData.data;

    if (format === 'nv12') {
      const ySize = width * height;
      for (let j = 0; j < height; j++) {
        for (let i = 0; i < width; i++) {
          const yIdx = j * width + i;
          const uvIdx = ySize + Math.floor(j / 2) * width + (i & ~1);
          const y = src[yIdx];
          const u = src[uvIdx] - 128;
          const v = src[uvIdx + 1] - 128;
          const o = yIdx * 4;
          out[o]     = clamp(y + 1.402 * v);
          out[o + 1] = clamp(y - 0.344 * u - 0.714 * v);
          out[o + 2] = clamp(y + 1.772 * u);
          out[o + 3] = 255;
        }
      }
    } else {
      const bpp = BPP[format];
      const rowBytes = width * bpp;
      const strideBytes = stride ?? rowBytes;
      const isFloat = format === 'rgba32f' || format === 'rg32f' || format === 'r32f';
      const channels = CHANNELS[format];

      for (let j = 0; j < height; j++) {
        for (let i = 0; i < width; i++) {
          const srcOff = j * strideBytes + i * bpp;
          const dstOff = (j * width + i) * 4;

          if (isFloat) {
            const floats = new Float32Array(src.buffer, srcOff, channels);
            if (channels >= 3) {
              out[dstOff]     = clamp(floats[0] * 255);
              out[dstOff + 1] = clamp(floats[1] * 255);
              out[dstOff + 2] = clamp(floats[2] * 255);
              out[dstOff + 3] = channels >= 4 ? clamp(floats[3] * 255) : 255;
            } else if (channels === 2) {
              out[dstOff]     = clamp(floats[0] * 255);
              out[dstOff + 1] = clamp(floats[1] * 255);
              out[dstOff + 2] = 0;
              out[dstOff + 3] = 255;
            } else {
              const g = clamp(floats[0] * 255);
              out[dstOff] = out[dstOff + 1] = out[dstOff + 2] = g;
              out[dstOff + 3] = 255;
            }
          } else {
            if (channels >= 3) {
              out[dstOff]     = src[srcOff];
              out[dstOff + 1] = src[srcOff + 1];
              out[dstOff + 2] = src[srcOff + 2];
              out[dstOff + 3] = channels >= 4 ? src[srcOff + 3] : 255;
            } else if (channels === 2) {
              out[dstOff]     = src[srcOff];
              out[dstOff + 1] = src[srcOff + 1];
              out[dstOff + 2] = 0;
              out[dstOff + 3] = 255;
            } else {
              out[dstOff] = out[dstOff + 1] = out[dstOff + 2] = src[srcOff];
              out[dstOff + 3] = 255;
            }
          }
        }
      }
    }

    ctx.putImageData(imageData, 0, 0);
    return canvas.toDataURL('image/png');
  } catch {
    return null;
  }
}

function clamp(v: number): number {
  return v < 0 ? 0 : v > 255 ? 255 : Math.round(v);
}

const BPP: Record<string, number> = {
  rgba8: 4, rgba32f: 16, rg8: 2, rg32f: 8, r8: 1, r32f: 4,
};

const CHANNELS: Record<string, number> = {
  rgba8: 4, rgba32f: 4, rg8: 2, rg32f: 2, r8: 1, r32f: 1,
};
