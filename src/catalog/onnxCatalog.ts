import type { Port } from '../types';

// ---------------------------------------------------------------------------
// Task taxonomy
// ---------------------------------------------------------------------------

export type OnnxTask =
  | 'super-resolution'
  | 'background-removal'
  | 'detection'
  | 'segmentation'
  | 'style-transfer'
  | 'denoising'
  | 'depth-estimation'
  | 'generic';

// ---------------------------------------------------------------------------
// Parameter descriptors (drive auto-generated UI knobs for a catalog entry)
// ---------------------------------------------------------------------------

export interface ParamDescriptor {
  type: 'float' | 'int' | 'boolean';
  default: number | boolean;
  min?: number;
  max?: number;
  step?: number;
  label: string;
}

// ---------------------------------------------------------------------------
// Catalog entry — everything we know about a model before downloading it
// ---------------------------------------------------------------------------

export interface CatalogEntry {
  id: string;
  label: string;
  task: OnnxTask;
  category: string;          // menu grouping: 'Detection', 'Super-Resolution', etc.
  downloadUrl: string;       // model download URL (network)
  fileSize: number;          // bytes, for progress display
  sha256: string;            // integrity check after download
  expectedIO: {
    inputs: Port[];
    outputs: Port[];
  };
  defaultParams?: Record<string, ParamDescriptor>;
}

// ---------------------------------------------------------------------------
// Built-in catalog
// ---------------------------------------------------------------------------

export const ONNX_CATALOG: Record<string, CatalogEntry> = {
  yolov8n: {
    id: 'yolov8n',
    label: 'YOLOv8n Detector',
    task: 'detection',
    category: 'Detection',
    downloadUrl: 'https://raw.githubusercontent.com/caozisheng/rimeflow-yolov8n/main/models/yolov8n.onnx',
    fileSize: 12_851_098,
    sha256: '', // TODO: fill in actual sha256 hash after verifying the release artifact
    expectedIO: {
      inputs: [
        { id: 'onnx_in_image', label: 'image', dataType: 'sampler2D', direction: 'input' },
      ],
      outputs: [
        { id: 'onnx_out_detections', label: 'detections', dataType: 'roi', direction: 'output' },
        { id: 'onnx_out_overlay', label: 'overlay', dataType: 'sampler2D', direction: 'output' },
      ],
    },
    defaultParams: {
      scoreThreshold: { type: 'float', default: 0.25, min: 0, max: 1, step: 0.05, label: 'Score Threshold' },
      iouThreshold: { type: 'float', default: 0.45, min: 0, max: 1, step: 0.05, label: 'IoU Threshold' },
    },
  },
  'super-resolution-3x': {
    id: 'super-resolution-3x',
    label: 'Super Resolution 3×',
    task: 'super-resolution',
    category: 'Super-Resolution',
    downloadUrl: 'https://media.githubusercontent.com/media/onnx/models/main/validated/vision/super_resolution/sub_pixel_cnn_2016/model/super-resolution-10.onnx',
    fileSize: 240_078,
    sha256: '',
    expectedIO: {
      inputs: [
        { id: 'onnx_in_image', label: 'image', dataType: 'sampler2D', direction: 'input' },
      ],
      outputs: [
        { id: 'onnx_out_upscaled', label: 'upscaled', dataType: 'sampler2D', direction: 'output' },
      ],
    },
  },
  'realesrgan-x4': {
    id: 'realesrgan-x4',
    label: 'Real-ESRGAN 4×',
    task: 'super-resolution',
    category: 'Super-Resolution',
    downloadUrl: 'https://huggingface.co/Samo629/real-esrgan-onnx/resolve/main/realesr-general-x4v3.onnx',
    fileSize: 4_866_421,
    sha256: '',
    expectedIO: {
      inputs: [
        { id: 'onnx_in_image', label: 'image', dataType: 'sampler2D', direction: 'input' },
      ],
      outputs: [
        { id: 'onnx_out_upscaled', label: 'upscaled', dataType: 'sampler2D', direction: 'output' },
      ],
    },
  },
  u2netp: {
    id: 'u2netp',
    label: 'U²Net-P (Background)',
    task: 'background-removal',
    category: 'Background Removal',
    downloadUrl: 'https://github.com/danielgatis/rembg/releases/download/v0.0.0/u2netp.onnx',
    fileSize: 4_574_861,
    sha256: '',
    expectedIO: {
      inputs: [
        { id: 'onnx_in_image', label: 'image', dataType: 'sampler2D', direction: 'input' },
      ],
      outputs: [
        { id: 'onnx_out_foreground', label: 'foreground', dataType: 'sampler2D', direction: 'output' },
      ],
    },
  },
  modnet: {
    id: 'modnet',
    label: 'MODNet (Portrait)',
    task: 'background-removal',
    category: 'Background Removal',
    downloadUrl: 'https://huggingface.co/onnx-community/modnet-webnn/resolve/main/onnx/model.onnx',
    fileSize: 25_888_640,
    sha256: '',
    expectedIO: {
      inputs: [
        { id: 'onnx_in_image', label: 'image', dataType: 'sampler2D', direction: 'input' },
      ],
      outputs: [
        { id: 'onnx_out_foreground', label: 'foreground', dataType: 'sampler2D', direction: 'output' },
      ],
    },
  },
  'midas-small': {
    id: 'midas-small',
    label: 'MiDaS v2.1 Small (Depth)',
    task: 'depth-estimation',
    category: 'Depth Estimation',
    downloadUrl: 'https://huggingface.co/Heliosoph/midas-small-onnx/resolve/main/midas_v21_small_256.onnx',
    fileSize: 66_389_153,
    sha256: '',
    expectedIO: {
      inputs: [
        { id: 'onnx_in_image', label: 'image', dataType: 'sampler2D', direction: 'input' },
      ],
      outputs: [
        { id: 'onnx_out_depth', label: 'depth', dataType: 'sampler2D', direction: 'output' },
      ],
    },
  },
  'yolo26n-sem': {
    id: 'yolo26n-sem',
    label: 'YOLO26n Semantic Seg',
    task: 'segmentation',
    category: 'Segmentation',
    downloadUrl: 'https://github.com/caozisheng/rimeflow-yolo26n-sem/raw/refs/heads/master/models/yolo26n-sem.onnx',
    fileSize: 6_284_385,
    sha256: '',
    expectedIO: {
      inputs: [
        { id: 'onnx_in_image', label: 'image', dataType: 'sampler2D', direction: 'input' },
      ],
      outputs: [
        { id: 'onnx_out_overlay', label: 'overlay', dataType: 'sampler2D', direction: 'output' },
      ],
    },
  },
};
// ---------------------------------------------------------------------------
// Derived helpers
// ---------------------------------------------------------------------------

/** Sorted unique category list derived from the catalog. */
export const CATALOG_CATEGORIES: string[] = [
  ...new Set(Object.values(ONNX_CATALOG).map((e) => e.category)),
].sort();
