/**
 * Pure TypeScript YOLO semantic segmentation post-processing.
 *
 * Replaces the Rust WASM `rimeflow-yolo26n-sem::postprocess` module.
 * Decodes raw segmentation output tensor, resizes the mask, and
 * colorizes with the Cityscapes palette.
 */

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface SegmentationMask {
  /** Class index per pixel (row-major, top-left origin). */
  classMap: Uint8Array;
  maskW: number;
  maskH: number;
  numClasses: number;
}

export interface SegmentationResult {
  maskRgba: Uint8Array;
  maskW: number;
  maskH: number;
  classCounts: number[];
  numClasses: number;
}

// ---------------------------------------------------------------------------
// Cityscapes 19-class palette (standard colors)
// ---------------------------------------------------------------------------

export const CITYSCAPES_CLASSES: readonly string[] = [
  'road', 'sidewalk', 'building', 'wall', 'fence',
  'pole', 'traffic light', 'traffic sign', 'vegetation', 'terrain',
  'sky', 'person', 'rider', 'car', 'truck',
  'bus', 'train', 'motorcycle', 'bicycle',
];

/** RGBA colors for each Cityscapes class (19 classes). */
export const CITYSCAPES_PALETTE: readonly [number, number, number, number][] = [
  [128, 64, 128, 255],   // road
  [244, 35, 232, 255],   // sidewalk
  [70, 70, 70, 255],     // building
  [102, 102, 156, 255],  // wall
  [190, 153, 153, 255],  // fence
  [153, 153, 153, 255],  // pole
  [250, 170, 30, 255],   // traffic light
  [220, 220, 0, 255],    // traffic sign
  [107, 142, 35, 255],   // vegetation
  [152, 251, 152, 255],  // terrain
  [70, 130, 180, 255],   // sky
  [220, 20, 60, 255],    // person
  [255, 0, 0, 255],      // rider
  [0, 0, 142, 255],      // car
  [0, 0, 70, 255],       // truck
  [0, 60, 100, 255],     // bus
  [0, 80, 100, 255],     // train
  [0, 0, 230, 255],      // motorcycle
  [119, 11, 32, 255],    // bicycle
];

// ---------------------------------------------------------------------------
// Decode
// ---------------------------------------------------------------------------

/**
 * Decode raw segmentation output tensor into a class map.
 *
 * @param raw     Float32 tensor, shape [1, numClasses, H, W] flattened (NCHW).
 * @param srcW    Original source image width (before letterbox).
 * @param srcH    Original source image height (before letterbox).
 * @param scale   Letterbox scale factor.
 * @param padX    Letterbox horizontal padding (pixels in model space).
 * @param padY    Letterbox vertical padding (pixels in model space).
 */
export function decodeSegmentationOutput(
  raw: Float32Array,
  srcW: number,
  srcH: number,
  scale: number,
  padX: number,
  padY: number,
): SegmentationMask {
  // Infer spatial dims from tensor: [1, C, H, W]
  // Total = C * H * W; we know C (try 19 for Cityscapes)
  const numClasses = 19;
  const total = raw.length;
  const spatial = total / numClasses;
  // H and W from the model output — for letterboxed 640×640 input,
  // output is typically 640×640 or downsampled.
  // We need to find H×W = spatial. Assume square if we can't determine.
  const side = Math.round(Math.sqrt(spatial));
  const outH = side;
  const outW = Math.round(spatial / side);

  // Compute the content region within the letterboxed output
  // (excluding padding). Map back to model output coords.
  const contentW = Math.round(srcW * scale);
  const contentH = Math.round(srcH * scale);
  const cropX = Math.round(padX * outW / (srcW * scale + 2 * padX));
  const cropY = Math.round(padY * outH / (srcH * scale + 2 * padY));
  const cropW = Math.round(contentW * outW / (srcW * scale + 2 * padX));
  const cropH = Math.round(contentH * outH / (srcH * scale + 2 * padY));

  // Argmax per pixel within the content region
  const maskW = cropW > 0 ? cropW : outW;
  const maskH = cropH > 0 ? cropH : outH;
  const classMap = new Uint8Array(maskW * maskH);

  for (let y = 0; y < maskH; y++) {
    for (let x = 0; x < maskW; x++) {
      const srcX = x + cropX;
      const srcY = y + cropY;
      let maxVal = -Infinity;
      let maxClass = 0;
      for (let c = 0; c < numClasses; c++) {
        const val = raw[c * outH * outW + srcY * outW + srcX];
        if (val > maxVal) {
          maxVal = val;
          maxClass = c;
        }
      }
      classMap[y * maskW + x] = maxClass;
    }
  }

  return { classMap, maskW, maskH, numClasses };
}

// ---------------------------------------------------------------------------
// Resize (nearest neighbor)
// ---------------------------------------------------------------------------

/**
 * Resize a class map using nearest-neighbor interpolation.
 */
export function resizeMaskNearest(
  mask: Uint8Array,
  srcW: number,
  srcH: number,
  dstW: number,
  dstH: number,
): Uint8Array {
  const out = new Uint8Array(dstW * dstH);
  const xRatio = srcW / dstW;
  const yRatio = srcH / dstH;

  for (let y = 0; y < dstH; y++) {
    const srcY = Math.min(Math.floor(y * yRatio), srcH - 1);
    for (let x = 0; x < dstW; x++) {
      const srcX = Math.min(Math.floor(x * xRatio), srcW - 1);
      out[y * dstW + x] = mask[srcY * srcW + srcX];
    }
  }

  return out;
}

// ---------------------------------------------------------------------------
// Colorize
// ---------------------------------------------------------------------------

/**
 * Convert a class map to RGBA pixels using a palette.
 */
export function maskToRgba(
  classMap: Uint8Array,
  width: number,
  height: number,
  palette: readonly (readonly [number, number, number, number])[] = CITYSCAPES_PALETTE,
): Uint8Array {
  const rgba = new Uint8Array(width * height * 4);
  for (let i = 0; i < classMap.length; i++) {
    const c = classMap[i];
    const color = c < palette.length ? palette[c] : [0, 0, 0, 255];
    const j = i * 4;
    rgba[j] = color[0];
    rgba[j + 1] = color[1];
    rgba[j + 2] = color[2];
    rgba[j + 3] = color[3];
  }
  return rgba;
}

// ---------------------------------------------------------------------------
// Full pipeline
// ---------------------------------------------------------------------------

/**
 * Full segmentation post-processing: decode → resize → colorize → class counts.
 */
export function segmentPostprocess(
  raw: Float32Array,
  srcW: number,
  srcH: number,
  scale: number,
  padX: number,
  padY: number,
): SegmentationResult {
  const decoded = decodeSegmentationOutput(raw, srcW, srcH, scale, padX, padY);
  const resized = resizeMaskNearest(decoded.classMap, decoded.maskW, decoded.maskH, srcW, srcH);

  const classCounts = new Array<number>(decoded.numClasses).fill(0);
  for (let i = 0; i < resized.length; i++) {
    const c = resized[i];
    if (c < decoded.numClasses) classCounts[c]++;
  }

  const maskRgba = maskToRgba(resized, srcW, srcH);

  return {
    maskRgba,
    maskW: srcW,
    maskH: srcH,
    classCounts,
    numClasses: decoded.numClasses,
  };
}
