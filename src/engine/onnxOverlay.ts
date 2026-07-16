import * as THREE from 'three';
import type { OnnxDetection } from './onnxSession';

const CLASS_HUE_STEP = 47; // co-prime with 360, gives good visual separation

function classColor(classId: number): string {
  const hue = (classId * CLASS_HUE_STEP) % 360;
  return `hsl(${hue}, 82%, 55%)`;
}

/**
 * Draw detection bounding boxes over an RGBA pixel buffer. The buffer is
 * expected in row-major top-left origin (as produced by
 * `WebGLRenderer.readTargetToOffscreenCanvas`).
 *
 * Returns:
 *   - `dataUrl` — PNG for the SidePanel preview.
 *   - `texture` — `THREE.CanvasTexture` for downstream sampler2D consumers.
 *   - `canvas`  — the underlying offscreen canvas, so callers can dispose it.
 */
export function drawDetectionOverlay(
  sourceCanvas: HTMLCanvasElement | OffscreenCanvas,
  width: number,
  height: number,
  detections: OnnxDetection[],
): { dataUrl: string; texture: THREE.CanvasTexture; canvas: HTMLCanvasElement } {
  const out = document.createElement('canvas');
  out.width = width;
  out.height = height;
  const ctx = out.getContext('2d');
  if (!ctx) throw new Error('drawDetectionOverlay: 2d context unavailable');

  ctx.drawImage(sourceCanvas, 0, 0, width, height);

  ctx.lineWidth = Math.max(1, Math.round(Math.min(width, height) / 320));
  ctx.font = `${Math.max(10, Math.round(height / 40))}px system-ui, sans-serif`;
  ctx.textBaseline = 'top';

  for (const det of detections) {
    const [x1, y1, x2, y2] = det.bbox;
    const px = Math.round(x1 * width);
    const py = Math.round(y1 * height);
    const pw = Math.round((x2 - x1) * width);
    const ph = Math.round((y2 - y1) * height);
    if (pw <= 0 || ph <= 0) continue;

    const color = classColor(det.class_id);
    ctx.strokeStyle = color;
    ctx.strokeRect(px, py, pw, ph);

    const label = `${det.class_name} ${Math.round(det.score * 100)}%`;
    const metrics = ctx.measureText(label);
    const th = Math.round(parseInt(ctx.font, 10) * 1.2);
    ctx.fillStyle = color;
    ctx.fillRect(px, Math.max(0, py - th), metrics.width + 6, th);
    ctx.fillStyle = '#fff';
    ctx.fillText(label, px + 3, Math.max(0, py - th) + 1);
  }

  const texture = new THREE.CanvasTexture(out);
  texture.needsUpdate = true;
  texture.flipY = true;
  return { dataUrl: out.toDataURL('image/png'), texture, canvas: out };
}

export function drawSegmentationOverlay(
  sourceCanvas: HTMLCanvasElement | OffscreenCanvas,
  width: number,
  height: number,
  maskRgba: Uint8Array,
  maskW: number,
  maskH: number,
): { dataUrl: string; texture: THREE.CanvasTexture; canvas: HTMLCanvasElement } {
  const out = document.createElement('canvas');
  out.width = width;
  out.height = height;
  const ctx = out.getContext('2d');
  if (!ctx) throw new Error('drawSegmentationOverlay: 2d context unavailable');

  ctx.drawImage(sourceCanvas, 0, 0, width, height);

  const maskCanvas = document.createElement('canvas');
  maskCanvas.width = maskW;
  maskCanvas.height = maskH;
  const maskCtx = maskCanvas.getContext('2d');
  if (maskCtx && maskW > 0 && maskH > 0 && typeof ImageData !== 'undefined') {
    const clamped = new Uint8ClampedArray(maskRgba.length);
    clamped.set(maskRgba);
    const imgData = new ImageData(clamped, maskW, maskH);
    maskCtx.putImageData(imgData, 0, 0);
    ctx.drawImage(maskCanvas, 0, 0, width, height);
  }

  const texture = new THREE.CanvasTexture(out);
  texture.needsUpdate = true;
  texture.flipY = true;
  return { dataUrl: out.toDataURL('image/png'), texture, canvas: out };
}
