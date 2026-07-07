import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { FramebufferFormat } from '../../src/types';

// We need to capture putImageData calls to inspect pixel output
let capturedImageData: ImageData | null = null;
let mockToDataURL: ReturnType<typeof vi.fn>;

beforeEach(() => {
  capturedImageData = null;
  mockToDataURL = vi.fn(() => 'data:image/png;base64,MOCK');

  const mockCtx = {
    createImageData: (w: number, h: number) => {
      const data = new Uint8ClampedArray(w * h * 4);
      return { data, width: w, height: h } as ImageData;
    },
    putImageData: (imageData: ImageData) => {
      capturedImageData = imageData;
    },
  };

  vi.spyOn(document, 'createElement').mockImplementation((tag: string) => {
    if (tag === 'canvas') {
      return {
        width: 0,
        height: 0,
        getContext: () => mockCtx,
        toDataURL: mockToDataURL,
      } as unknown as HTMLCanvasElement;
    }
    return document.createElementNS('http://www.w3.org/1999/xhtml', tag) as HTMLElement;
  });
});

/** Encode raw bytes as a base64 data URL. */
function encodeDataUrl(bytes: Uint8Array): string {
  let binary = '';
  for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
  return 'data:application/octet-stream;base64,' + btoa(binary);
}

/** Encode a Float32Array as a base64 data URL. */
function encodeFloatDataUrl(floats: Float32Array): string {
  return encodeDataUrl(new Uint8Array(floats.buffer));
}

// Import after mocks are set up (module is stateless so order doesn't matter here)
import { generateRawPreview } from '../../src/utils/rawPreview';

describe('generateRawPreview', () => {
  describe('format rgba8 (4 channels, byte)', () => {
    it('maps RGBA bytes directly to output pixels', () => {
      // 1x1 pixel: R=10, G=20, B=30, A=40
      const raw = new Uint8Array([10, 20, 30, 40]);
      const dataUrl = encodeDataUrl(raw);
      const result = generateRawPreview(dataUrl, 'rgba8', 1, 1);

      expect(result).toBe('data:image/png;base64,MOCK');
      expect(capturedImageData).not.toBeNull();
      const d = capturedImageData!.data;
      expect(d[0]).toBe(10);
      expect(d[1]).toBe(20);
      expect(d[2]).toBe(30);
      expect(d[3]).toBe(40);
    });

    it('handles 2x1 image', () => {
      const raw = new Uint8Array([
        255, 0, 0, 255,   // pixel (0,0): red
        0, 255, 0, 128,   // pixel (1,0): green
      ]);
      const dataUrl = encodeDataUrl(raw);
      generateRawPreview(dataUrl, 'rgba8', 2, 1);

      const d = capturedImageData!.data;
      // pixel 0
      expect(d[0]).toBe(255);
      expect(d[1]).toBe(0);
      expect(d[2]).toBe(0);
      expect(d[3]).toBe(255);
      // pixel 1
      expect(d[4]).toBe(0);
      expect(d[5]).toBe(255);
      expect(d[6]).toBe(0);
      expect(d[7]).toBe(128);
    });
  });

  describe('format r8 (1 channel → grayscale)', () => {
    it('repeats single channel across RGB with alpha=255', () => {
      const raw = new Uint8Array([100]);
      const dataUrl = encodeDataUrl(raw);
      generateRawPreview(dataUrl, 'r8', 1, 1);

      const d = capturedImageData!.data;
      expect(d[0]).toBe(100); // R
      expect(d[1]).toBe(100); // G
      expect(d[2]).toBe(100); // B
      expect(d[3]).toBe(255); // A
    });

    it('handles multiple pixels', () => {
      const raw = new Uint8Array([50, 200]);
      const dataUrl = encodeDataUrl(raw);
      generateRawPreview(dataUrl, 'r8', 2, 1);

      const d = capturedImageData!.data;
      expect(d[0]).toBe(50);
      expect(d[4]).toBe(200);
    });
  });

  describe('format rg8 (2 channels)', () => {
    it('maps R and G channels, sets B=0 and A=255', () => {
      const raw = new Uint8Array([100, 200]);
      const dataUrl = encodeDataUrl(raw);
      generateRawPreview(dataUrl, 'rg8', 1, 1);

      const d = capturedImageData!.data;
      expect(d[0]).toBe(100); // R
      expect(d[1]).toBe(200); // G
      expect(d[2]).toBe(0);   // B
      expect(d[3]).toBe(255); // A
    });
  });

  describe('format rgba32f (float → clamped byte)', () => {
    it('converts float [0..1] to byte [0..255]', () => {
      const floats = new Float32Array([0.5, 0.25, 1.0, 0.75]);
      const dataUrl = encodeFloatDataUrl(floats);
      generateRawPreview(dataUrl, 'rgba32f', 1, 1);

      const d = capturedImageData!.data;
      expect(d[0]).toBe(128);  // 0.5 * 255 = 127.5, rounded
      expect(d[1]).toBe(64);   // 0.25 * 255 = 63.75, rounded
      expect(d[2]).toBe(255);  // 1.0 * 255
      expect(d[3]).toBe(191);  // 0.75 * 255 = 191.25, rounded
    });

    it('clamps values above 1.0 to 255', () => {
      const floats = new Float32Array([2.0, 0.0, 0.0, 1.0]);
      const dataUrl = encodeFloatDataUrl(floats);
      generateRawPreview(dataUrl, 'rgba32f', 1, 1);

      expect(capturedImageData!.data[0]).toBe(255);
    });

    it('clamps negative values to 0', () => {
      const floats = new Float32Array([-1.0, 0.0, 0.0, 1.0]);
      const dataUrl = encodeFloatDataUrl(floats);
      generateRawPreview(dataUrl, 'rgba32f', 1, 1);

      expect(capturedImageData!.data[0]).toBe(0);
    });
  });

  describe('format r32f (1-channel float)', () => {
    it('converts single float channel to grayscale', () => {
      const floats = new Float32Array([0.5]);
      const dataUrl = encodeFloatDataUrl(floats);
      generateRawPreview(dataUrl, 'r32f', 1, 1);

      const d = capturedImageData!.data;
      const expected = Math.round(0.5 * 255);
      expect(d[0]).toBe(expected);
      expect(d[1]).toBe(expected);
      expect(d[2]).toBe(expected);
      expect(d[3]).toBe(255);
    });
  });

  describe('format rg32f (2-channel float)', () => {
    it('maps R and G float channels, B=0, A=255', () => {
      const floats = new Float32Array([0.3, 0.7]);
      const dataUrl = encodeFloatDataUrl(floats);
      generateRawPreview(dataUrl, 'rg32f', 1, 1);

      const d = capturedImageData!.data;
      // clamp uses Math.round, Float32 may lose precision
      const clamp = (v: number) => v < 0 ? 0 : v > 255 ? 255 : Math.round(v);
      expect(d[0]).toBe(clamp(floats[0] * 255));
      expect(d[1]).toBe(clamp(floats[1] * 255));
      expect(d[2]).toBe(0);
      expect(d[3]).toBe(255);
    });
  });

  describe('format nv12 (YUV → RGB conversion)', () => {
    it('converts NV12 YUV data to RGB pixels', () => {
      // 2x2 image in NV12: Y plane = 4 bytes, UV plane = 4 bytes (2x1 interleaved U,V)
      const width = 2;
      const height = 2;
      const ySize = width * height;
      // Y values: 128 for all pixels
      // UV values: U=128 (neutral), V=128 (neutral) → should produce gray
      const raw = new Uint8Array(ySize + width * Math.ceil(height / 2));
      // Y plane
      raw[0] = 128; raw[1] = 128; raw[2] = 128; raw[3] = 128;
      // UV plane (interleaved U, V)
      raw[4] = 128; raw[5] = 128; // row 0 UV
      raw[6] = 128; raw[7] = 128; // row 1 UV (same block)

      const dataUrl = encodeDataUrl(raw);
      generateRawPreview(dataUrl, 'nv12', width, height);

      const d = capturedImageData!.data;
      // With Y=128, U=0 (128-128), V=0 → R≈128, G≈128, B≈128
      expect(d[0]).toBe(128); // R
      expect(d[1]).toBe(128); // G
      expect(d[2]).toBe(128); // B
      expect(d[3]).toBe(255); // A always 255 for nv12
    });

    it('produces colored output for non-neutral UV', () => {
      const width = 1;
      const height = 2;
      const raw = new Uint8Array([
        200, 200,       // Y plane: 2 pixels
        100, 200,       // UV plane: U=100, V=200
      ]);
      const dataUrl = encodeDataUrl(raw);
      generateRawPreview(dataUrl, 'nv12', width, height);

      const d = capturedImageData!.data;
      // Y=200, U=100-128=-28, V=200-128=72
      // R = clamp(200 + 1.402*72) ≈ 301 → 255
      // G = clamp(200 - 0.344*(-28) - 0.714*72) ≈ 200 + 9.632 - 51.408 ≈ 158
      // B = clamp(200 + 1.772*(-28)) ≈ 200 - 49.616 ≈ 150
      expect(d[3]).toBe(255); // alpha always 255
      // Verify pixel is not pure gray (UV affects color)
      expect(d[0]).not.toBe(d[1]);
    });
  });

  describe('stride parameter', () => {
    it('skips padding bytes between rows with stride > row width', () => {
      // 2x2 image, rgba8 (bpp=4), rowBytes=8, stride=16 (8 bytes padding per row)
      const stride = 16;
      const raw = new Uint8Array(stride * 2);
      // Row 0: pixel(0,0)=red, pixel(1,0)=green, then 8 bytes padding
      raw.set([255, 0, 0, 255, 0, 255, 0, 255], 0);
      // Row 1 at offset 16: pixel(0,1)=blue, pixel(1,1)=white
      raw.set([0, 0, 255, 255, 255, 255, 255, 255], stride);

      const dataUrl = encodeDataUrl(raw);
      generateRawPreview(dataUrl, 'rgba8', 2, 2, stride);

      const d = capturedImageData!.data;
      // pixel (0,0): red
      expect(d[0]).toBe(255);
      expect(d[1]).toBe(0);
      expect(d[2]).toBe(0);
      // pixel (1,0): green
      expect(d[4]).toBe(0);
      expect(d[5]).toBe(255);
      expect(d[6]).toBe(0);
      // pixel (0,1): blue — at output offset (1*2+0)*4 = 8
      expect(d[8]).toBe(0);
      expect(d[9]).toBe(0);
      expect(d[10]).toBe(255);
      // pixel (1,1): white
      expect(d[12]).toBe(255);
      expect(d[13]).toBe(255);
      expect(d[14]).toBe(255);
    });

    it('works with float formats and stride', () => {
      // 1x2 image, r32f (bpp=4), rowBytes=4, stride=8 (4 bytes padding)
      const stride = 8;
      const buf = new ArrayBuffer(stride * 2);
      const view = new DataView(buf);
      view.setFloat32(0, 0.5, true);   // row 0
      view.setFloat32(stride, 1.0, true); // row 1

      const raw = new Uint8Array(buf);
      const dataUrl = encodeDataUrl(raw);
      generateRawPreview(dataUrl, 'r32f', 1, 2, stride);

      const d = capturedImageData!.data;
      // pixel (0,0): gray from 0.5
      expect(d[0]).toBe(Math.round(0.5 * 255));
      // pixel (0,1): white from 1.0 — at output offset 4
      expect(d[4]).toBe(255);
    });
  });

  describe('error handling', () => {
    it('returns null for invalid data URL (no comma)', () => {
      const result = generateRawPreview('invalid-data-url', 'rgba8', 1, 1);
      expect(result).toBeNull();
    });

    it('returns null for malformed base64', () => {
      const result = generateRawPreview('data:;base64,!!!invalid!!!', 'rgba8', 1, 1);
      expect(result).toBeNull();
    });

    it('returns null when atob throws', () => {
      // The data URL has a comma but the base64 part is invalid
      const result = generateRawPreview('data:application/octet-stream;base64,@@@', 'rgba8', 1, 1);
      expect(result).toBeNull();
    });
  });

  describe('canvas interaction', () => {
    it('calls canvas.toDataURL', () => {
      const raw = new Uint8Array([10, 20, 30, 40]);
      const dataUrl = encodeDataUrl(raw);
      generateRawPreview(dataUrl, 'rgba8', 1, 1);

      expect(mockToDataURL).toHaveBeenCalledWith('image/png');
    });

    it('calls putImageData with the computed image data', () => {
      const raw = new Uint8Array([10, 20, 30, 40]);
      const dataUrl = encodeDataUrl(raw);
      generateRawPreview(dataUrl, 'rgba8', 1, 1);

      expect(capturedImageData).not.toBeNull();
      expect(capturedImageData!.width).toBe(1);
      expect(capturedImageData!.height).toBe(1);
    });
  });
});
