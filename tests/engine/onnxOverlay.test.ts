import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import type { OnnxDetection } from '../../src/engine/onnx/overlay';

// Mock three's CanvasTexture — mirror the executionEngine.test.ts style.
vi.mock('three', () => ({
  CanvasTexture: class {
    image: unknown;
    needsUpdate = false;
    flipY = true;
    dispose = vi.fn();
    constructor(img?: unknown) {
      this.image = img;
    }
  },
}));

import { drawDetectionOverlay, drawSegmentationOverlay } from '../../src/engine/onnx/overlay';

interface MeasureTextResult { width: number }

interface FakeContext {
  drawImage: ReturnType<typeof vi.fn>;
  strokeRect: ReturnType<typeof vi.fn>;
  fillRect: ReturnType<typeof vi.fn>;
  fillText: ReturnType<typeof vi.fn>;
  measureText: (text: string) => MeasureTextResult;
  lineWidth: number;
  font: string;
  textBaseline: CanvasTextBaseline;
  strokeStyle: string;
  fillStyle: string;
  strokeStyleHistory: string[];
  fillStyleHistory: string[];
}

function makeFakeContext(): FakeContext {
  const strokeStyleHistory: string[] = [];
  const fillStyleHistory: string[] = [];
  const ctx: FakeContext = {
    drawImage: vi.fn(),
    strokeRect: vi.fn(),
    fillRect: vi.fn(),
    fillText: vi.fn(),
    measureText: (text: string) => ({ width: text.length * 6 }),
    lineWidth: 1,
    font: '10px sans-serif',
    textBaseline: 'alphabetic',
    strokeStyle: '',
    fillStyle: '',
    strokeStyleHistory,
    fillStyleHistory,
  };
  // Property setters record history so tests can prove class colors were used.
  Object.defineProperty(ctx, 'strokeStyle', {
    get: () => strokeStyleHistory.at(-1) ?? '',
    set: (v: string) => { strokeStyleHistory.push(v); },
  });
  Object.defineProperty(ctx, 'fillStyle', {
    get: () => fillStyleHistory.at(-1) ?? '',
    set: (v: string) => { fillStyleHistory.push(v); },
  });
  return ctx;
}

// Stub `HTMLCanvasElement.prototype.getContext` per-test so we can control it.
// (jsdom returns null for 2d without the `canvas` optional dep — installed?
//  No, it's not; we assert that behavior directly below.)
let originalGetContext: typeof HTMLCanvasElement.prototype.getContext;
let originalToDataURL: typeof HTMLCanvasElement.prototype.toDataURL;
let contexts: FakeContext[] = [];

beforeEach(() => {
  contexts = [];
  originalGetContext = HTMLCanvasElement.prototype.getContext;
  originalToDataURL = HTMLCanvasElement.prototype.toDataURL;
  HTMLCanvasElement.prototype.getContext = vi.fn(function (this: HTMLCanvasElement, type: string) {
    if (type !== '2d') return null;
    const ctx = makeFakeContext();
    contexts.push(ctx);
    return ctx as unknown as CanvasRenderingContext2D;
  }) as typeof HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.toDataURL = vi.fn(() => 'data:image/png;base64,MOCK') as typeof HTMLCanvasElement.prototype.toDataURL;
});

afterEach(() => {
  HTMLCanvasElement.prototype.getContext = originalGetContext;
  HTMLCanvasElement.prototype.toDataURL = originalToDataURL;
  vi.restoreAllMocks();
});

function det(overrides: Partial<OnnxDetection> = {}): OnnxDetection {
  return {
    bbox: [0.1, 0.1, 0.4, 0.5],
    score: 0.9,
    class_id: 1,
    class_name: 'person',
    ...overrides,
  };
}

describe('drawDetectionOverlay', () => {
  it('returns { dataUrl, texture, canvas } shaped result at the requested size', () => {
    const source = document.createElement('canvas');
    source.width = 640;
    source.height = 480;

    const result = drawDetectionOverlay(source, 320, 240, []);

    expect(result.canvas.width).toBe(320);
    expect(result.canvas.height).toBe(240);
    expect(result.dataUrl).toBe('data:image/png;base64,MOCK');
    expect(result.texture).toBeDefined();
  });

  it('marks the CanvasTexture as needsUpdate=true and flipY=true', () => {
    const source = document.createElement('canvas');
    const result = drawDetectionOverlay(source, 100, 100, []);
    expect(result.texture.needsUpdate).toBe(true);
    expect(result.texture.flipY).toBe(true);
  });

  it('throws when 2d context is unavailable', () => {
    HTMLCanvasElement.prototype.getContext = vi.fn(() => null) as typeof HTMLCanvasElement.prototype.getContext;
    const source = document.createElement('canvas');
    expect(() => drawDetectionOverlay(source, 100, 100, [])).toThrow('2d context unavailable');
  });

  it('draws the source image once even with zero detections', () => {
    const source = document.createElement('canvas');
    drawDetectionOverlay(source, 100, 100, []);

    expect(contexts).toHaveLength(1);
    const [ctx] = contexts;
    expect(ctx.drawImage).toHaveBeenCalledTimes(1);
    expect(ctx.drawImage).toHaveBeenCalledWith(source, 0, 0, 100, 100);
    // No detections → no strokes / label fills.
    expect(ctx.strokeRect).not.toHaveBeenCalled();
    expect(ctx.fillText).not.toHaveBeenCalled();
  });

  it('draws stroke + label fill + label text for a valid detection', () => {
    const source = document.createElement('canvas');
    drawDetectionOverlay(source, 640, 480, [det({ bbox: [0.1, 0.2, 0.5, 0.6], score: 0.87, class_name: 'car' })]);

    const [ctx] = contexts;
    expect(ctx.strokeRect).toHaveBeenCalledTimes(1);
    // fillRect draws the label background; fillText writes the label.
    expect(ctx.fillRect).toHaveBeenCalledTimes(1);
    expect(ctx.fillText).toHaveBeenCalledTimes(1);
    const fillTextArgs = ctx.fillText.mock.calls[0];
    expect(fillTextArgs[0]).toBe('car 87%');
  });

  it('positions the bbox stroke using the target width/height', () => {
    const source = document.createElement('canvas');
    // bbox [0, 0, 1, 1] normalized → full canvas at 100×100.
    drawDetectionOverlay(source, 100, 100, [det({ bbox: [0, 0, 1, 1] })]);

    const [ctx] = contexts;
    expect(ctx.strokeRect).toHaveBeenCalledWith(0, 0, 100, 100);
  });

  it('skips detections with zero width or zero height', () => {
    const source = document.createElement('canvas');
    drawDetectionOverlay(source, 100, 100, [
      det({ bbox: [0.5, 0.5, 0.5, 0.9], class_name: 'zero_w' }),
      det({ bbox: [0.5, 0.5, 0.9, 0.5], class_name: 'zero_h' }),
      det({ bbox: [0.9, 0.5, 0.5, 0.9], class_name: 'neg_w' }),
      det({ bbox: [0.5, 0.9, 0.9, 0.5], class_name: 'neg_h' }),
    ]);

    const [ctx] = contexts;
    // Not one of these detections should produce a stroke.
    expect(ctx.strokeRect).not.toHaveBeenCalled();
    expect(ctx.fillText).not.toHaveBeenCalled();
  });

  it('mixes valid and invalid detections — only valid ones stroke', () => {
    const source = document.createElement('canvas');
    drawDetectionOverlay(source, 100, 100, [
      det({ bbox: [0.5, 0.5, 0.5, 0.9], class_name: 'skipped' }),
      det({ bbox: [0.1, 0.1, 0.4, 0.4], class_name: 'kept' }),
    ]);

    const [ctx] = contexts;
    expect(ctx.strokeRect).toHaveBeenCalledTimes(1);
    expect(ctx.fillText).toHaveBeenCalledTimes(1);
    expect(ctx.fillText.mock.calls[0][0]).toContain('kept');
  });

  it('gives the same class_id the same color across calls (deterministic)', () => {
    const source = document.createElement('canvas');
    drawDetectionOverlay(source, 100, 100, [det({ class_id: 7 })]);
    drawDetectionOverlay(source, 100, 100, [det({ class_id: 7 })]);

    expect(contexts).toHaveLength(2);
    const firstStroke = contexts[0].strokeStyleHistory.at(-1);
    const secondStroke = contexts[1].strokeStyleHistory.at(-1);
    expect(firstStroke).toBeDefined();
    expect(firstStroke).toBe(secondStroke);
    expect(firstStroke).toMatch(/^hsl\(/);
  });

  it('gives different class_id values different colors', () => {
    const source = document.createElement('canvas');
    drawDetectionOverlay(source, 100, 100, [det({ class_id: 1 })]);
    drawDetectionOverlay(source, 100, 100, [det({ class_id: 2 })]);

    const firstStroke = contexts[0].strokeStyleHistory.at(-1);
    const secondStroke = contexts[1].strokeStyleHistory.at(-1);
    expect(firstStroke).not.toBe(secondStroke);
  });

  it('scales stroke width and label font by canvas dimensions', () => {
    const source = document.createElement('canvas');
    drawDetectionOverlay(source, 100, 100, []);
    const [smallCtx] = contexts;
    const smallLineWidth = smallCtx.lineWidth;
    const smallFont = smallCtx.font;

    contexts = [];
    drawDetectionOverlay(source, 1600, 1200, []);
    const [bigCtx] = contexts;

    // Bigger canvas → equal or thicker stroke, larger font.
    expect(bigCtx.lineWidth).toBeGreaterThanOrEqual(smallLineWidth);
    expect(parseInt(bigCtx.font, 10)).toBeGreaterThan(parseInt(smallFont, 10));
  });

  it('draws a label even when the box hugs the top edge (py=0 clamp)', () => {
    const source = document.createElement('canvas');
    drawDetectionOverlay(source, 100, 100, [det({ bbox: [0, 0, 0.5, 0.5] })]);

    const [ctx] = contexts;
    expect(ctx.fillText).toHaveBeenCalledTimes(1);
    // fillRect y is Math.max(0, py - th) → 0 when py=0.
    const fillRectArgs = ctx.fillRect.mock.calls[0];
    expect(fillRectArgs[1]).toBe(0);
  });
});

describe('drawSegmentationOverlay', () => {
  it('returns { dataUrl, texture, canvas } at the requested size', () => {
    const source = document.createElement('canvas');
    source.width = 640;
    source.height = 480;
    const mask = new Uint8Array(4 * 2 * 2); // 2×2 RGBA

    const result = drawSegmentationOverlay(source, 320, 240, mask, 2, 2);

    expect(result.canvas.width).toBe(320);
    expect(result.canvas.height).toBe(240);
    expect(result.dataUrl).toBe('data:image/png;base64,MOCK');
    expect(result.texture).toBeDefined();
  });

  it('marks CanvasTexture as needsUpdate=true and flipY=true', () => {
    const source = document.createElement('canvas');
    const mask = new Uint8Array(4);
    const result = drawSegmentationOverlay(source, 100, 100, mask, 1, 1);
    expect(result.texture.needsUpdate).toBe(true);
    expect(result.texture.flipY).toBe(true);
  });

  it('throws when 2d context is unavailable', () => {
    HTMLCanvasElement.prototype.getContext = vi.fn(() => null) as typeof HTMLCanvasElement.prototype.getContext;
    const source = document.createElement('canvas');
    expect(() => drawSegmentationOverlay(source, 100, 100, new Uint8Array(4), 1, 1)).toThrow(
      '2d context unavailable',
    );
  });

  it('draws the source image onto the output canvas', () => {
    const source = document.createElement('canvas');
    const mask = new Uint8Array(4);
    drawSegmentationOverlay(source, 200, 150, mask, 1, 1);

    expect(contexts).toHaveLength(2); // output ctx + mask ctx
    const [outCtx] = contexts;
    expect(outCtx.drawImage).toHaveBeenCalled();
  });

  it('handles a zero-size mask without error', () => {
    const source = document.createElement('canvas');
    expect(() =>
      drawSegmentationOverlay(source, 100, 100, new Uint8Array(0), 0, 0),
    ).not.toThrow();
  });
});

