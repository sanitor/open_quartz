import type { DataType, Port } from '../types';

export type OnnxTask =
  | 'super-resolution'
  | 'background-removal'
  | 'detection'
  | 'segmentation'
  | 'style-transfer'
  | 'denoising'
  | 'depth-estimation'
  | 'generic';

export interface MathDescriptor {
  id: string;
  label: string;
  category: string;
  inputCount: number;
  formula: string;
}

export interface MathCategory {
  category: string;
  ops: string[];
}

export interface ParamDescriptor {
  type: 'float' | 'int' | 'boolean';
  default: number | boolean;
  min?: number;
  max?: number;
  step?: number;
  label: string;
}

export interface OnnxModelDescriptor {
  id: string;
  label: string;
  task: OnnxTask;
  category: string;
  downloadUrl: string;
  fileSize: number;
  sha256: string;
  expectedIO: {
    inputs: Port[];
    outputs: Port[];
  };
  defaultParams?: Record<string, ParamDescriptor>;
}

export interface ShaderPortDescriptor {
  label: string;
  dataType: DataType;
}

export interface ShaderTemplateDescriptor {
  id: string;
  label: string;
  inputs: ShaderPortDescriptor[];
  outputs: ShaderPortDescriptor[];
}

export interface ShaderGroupDescriptor {
  category: string;
  items: ShaderTemplateDescriptor[];
}

export interface CatalogSnapshot {
  mathCategories: MathCategory[];
  mathOps: MathDescriptor[];
  onnxCategories: string[];
  onnxModels: OnnxModelDescriptor[];
  shaderGroups: ShaderGroupDescriptor[];
}

const SNAPSHOT: CatalogSnapshot = {
  mathCategories: [
    { category: 'Arithmetic', ops: ['add', 'subtract', 'multiply', 'divide', 'negate', 'modulo'] },
    { category: 'Range', ops: ['min', 'max', 'clamp', 'saturate', 'step', 'smoothstep', 'abs', 'sign'] },
    { category: 'Trigonometry', ops: ['sin', 'cos', 'tan', 'asin', 'acos', 'atan'] },
    { category: 'Exponential', ops: ['pow', 'sqrt', 'exp', 'log'] },
    { category: 'Interpolation', ops: ['mix'] },
    { category: 'Rounding', ops: ['floor', 'ceil', 'round', 'fract'] },
  ],
  mathOps: [
    { id: 'add', label: 'Add', category: 'Arithmetic', inputCount: 2, formula: 'a + b' },
    { id: 'subtract', label: 'Subtract', category: 'Arithmetic', inputCount: 2, formula: 'a - b' },
    { id: 'multiply', label: 'Multiply', category: 'Arithmetic', inputCount: 2, formula: 'a * b' },
    { id: 'divide', label: 'Divide', category: 'Arithmetic', inputCount: 2, formula: 'b == 0 ? 0 : a / b' },
    { id: 'negate', label: 'Negate', category: 'Arithmetic', inputCount: 1, formula: '-a' },
    { id: 'modulo', label: 'Modulo', category: 'Arithmetic', inputCount: 2, formula: 'b == 0 ? 0 : a % b' },
    { id: 'min', label: 'Min', category: 'Range', inputCount: 2, formula: 'min(a, b)' },
    { id: 'max', label: 'Max', category: 'Range', inputCount: 2, formula: 'max(a, b)' },
    { id: 'clamp', label: 'Clamp', category: 'Range', inputCount: 3, formula: 'min(max(a, b), c)' },
    { id: 'saturate', label: 'Saturate', category: 'Range', inputCount: 1, formula: 'clamp(a, 0, 1)' },
    { id: 'step', label: 'Step', category: 'Range', inputCount: 2, formula: 'b >= a ? 1 : 0' },
    { id: 'smoothstep', label: 'Smoothstep', category: 'Range', inputCount: 3, formula: 't*t*(3-2*t), t=clamp((c-a)/(b-a),0,1)' },
    { id: 'abs', label: 'Abs', category: 'Range', inputCount: 1, formula: 'abs(a)' },
    { id: 'sign', label: 'Sign', category: 'Range', inputCount: 1, formula: 'a == 0 ? a : sign(a)' },
    { id: 'sin', label: 'Sin', category: 'Trigonometry', inputCount: 1, formula: 'sin(a)' },
    { id: 'cos', label: 'Cos', category: 'Trigonometry', inputCount: 1, formula: 'cos(a)' },
    { id: 'tan', label: 'Tan', category: 'Trigonometry', inputCount: 1, formula: 'tan(a)' },
    { id: 'asin', label: 'Asin', category: 'Trigonometry', inputCount: 1, formula: 'asin(a)' },
    { id: 'acos', label: 'Acos', category: 'Trigonometry', inputCount: 1, formula: 'acos(a)' },
    { id: 'atan', label: 'Atan', category: 'Trigonometry', inputCount: 1, formula: 'atan(a)' },
    { id: 'pow', label: 'Pow', category: 'Exponential', inputCount: 2, formula: 'pow(a, b)' },
    { id: 'sqrt', label: 'Sqrt', category: 'Exponential', inputCount: 1, formula: 'sqrt(a)' },
    { id: 'exp', label: 'Exp', category: 'Exponential', inputCount: 1, formula: 'exp(a)' },
    { id: 'log', label: 'Log', category: 'Exponential', inputCount: 1, formula: 'ln(a)' },
    { id: 'mix', label: 'Mix', category: 'Interpolation', inputCount: 3, formula: 'a * (1 - c) + b * c' },
    { id: 'floor', label: 'Floor', category: 'Rounding', inputCount: 1, formula: 'floor(a)' },
    { id: 'ceil', label: 'Ceil', category: 'Rounding', inputCount: 1, formula: 'ceil(a)' },
    { id: 'round', label: 'Round', category: 'Rounding', inputCount: 1, formula: 'a.fract() == -0.5 ? ceil(a) : floor(a + 0.5)' },
    { id: 'fract', label: 'Fract', category: 'Rounding', inputCount: 1, formula: 'a - floor(a)' },
  ],
  onnxCategories: ['Background Removal', 'Depth Estimation', 'Detection', 'Segmentation', 'Super-Resolution'],
  onnxModels: [
    {
      id: 'yolov8n',
      label: 'YOLOv8n Detector',
      task: 'detection',
      category: 'Detection',
      downloadUrl: 'https://raw.githubusercontent.com/caozisheng/rimeflow-yolov8n/main/models/yolov8n.onnx',
      fileSize: 12_851_098,
      sha256: '',
      expectedIO: {
        inputs: [{ id: 'onnx_in_image', label: 'image', dataType: 'sampler2D', direction: 'input' }],
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
    model('super-resolution-3x', 'Super Resolution 3×', 'super-resolution', 'Super-Resolution', 'https://media.githubusercontent.com/media/onnx/models/main/validated/vision/super_resolution/sub_pixel_cnn_2016/model/super-resolution-10.onnx', 240_078, 'onnx_out_upscaled', 'upscaled'),
    model('realesrgan-x4', 'Real-ESRGAN 4×', 'super-resolution', 'Super-Resolution', 'https://huggingface.co/Samo629/real-esrgan-onnx/resolve/main/realesr-general-x4v3.onnx', 4_866_421, 'onnx_out_upscaled', 'upscaled'),
    model('u2netp', 'U²Net-P (Background)', 'background-removal', 'Background Removal', 'https://github.com/danielgatis/rembg/releases/download/v0.0.0/u2netp.onnx', 4_574_861, 'onnx_out_foreground', 'foreground'),
    model('modnet', 'MODNet (Portrait)', 'background-removal', 'Background Removal', 'https://huggingface.co/onnx-community/modnet-webnn/resolve/main/onnx/model.onnx', 25_888_640, 'onnx_out_foreground', 'foreground'),
    model('midas-small', 'MiDaS v2.1 Small (Depth)', 'depth-estimation', 'Depth Estimation', 'https://huggingface.co/Heliosoph/midas-small-onnx/resolve/main/midas_v21_small_256.onnx', 66_389_153, 'onnx_out_depth', 'depth'),
    model('yolo26n-sem', 'YOLO26n Semantic Seg', 'segmentation', 'Segmentation', 'https://github.com/caozisheng/rimeflow-yolo26n-sem/raw/refs/heads/master/models/yolo26n-sem.onnx', 6_284_385, 'onnx_out_overlay', 'overlay'),
  ],
  shaderGroups: [
    shaderGroup('FILTER', [
      shader('Resample', [['inputImage', 'sampler2D']], [['fragColor', 'vec4']]),
      shader('Sobel Edge Detection', [['inputImage', 'sampler2D'], ['intensity', 'float']], [['fragColor', 'vec4']]),
      shader('Gaussian Blur 3x3', [['inputImage', 'sampler2D']], [['fragColor', 'vec4']]),
      shader('Box Blur', [['inputImage', 'sampler2D']], [['fragColor', 'vec4']]),
      shader('Sharpen', [['inputImage', 'sampler2D'], ['strength', 'float']], [['fragColor', 'vec4']]),
      shader('Emboss', [['inputImage', 'sampler2D']], [['fragColor', 'vec4']]),
      shader('Pixelate', [['inputImage', 'sampler2D'], ['blockSize', 'vec2']], [['fragColor', 'vec4']]),
    ]),
    shaderGroup('COLOR', [
      shader('Invert', [['inputImage', 'sampler2D']], [['fragColor', 'vec4']]),
      shader('Grayscale', [['inputImage', 'sampler2D']], [['fragColor', 'vec4']]),
      shader('Brightness/Contrast', [['inputImage', 'sampler2D'], ['brightness', 'float'], ['contrast', 'float']], [['fragColor', 'vec4']]),
      shader('Hue Rotate', [['inputImage', 'sampler2D'], ['angle', 'float']], [['fragColor', 'vec4']]),
      shader('Threshold', [['inputImage', 'sampler2D'], ['threshold', 'float']], [['fragColor', 'vec4']]),
      shader('Sepia', [['inputImage', 'sampler2D']], [['fragColor', 'vec4']]),
      shader('Field Color Map', [['inputImage', 'sampler2D']], [['fragColor', 'vec4']]),
    ]),
    shaderGroup('GENERATOR', [
      shader('Solid Color', [['color', 'vec4']], [['fragColor', 'vec4']]),
      shader('Gradient', [['colorA', 'vec4'], ['colorB', 'vec4']], [['fragColor', 'vec4']]),
      shader('Checkerboard', [['gridSize', 'vec2'], ['color1', 'vec4'], ['color2', 'vec4']], [['fragColor', 'vec4']]),
      shader('Noise', [['scale', 'float']], [['fragColor', 'vec4']]),
      shader('Circle', [['circle', 'vec4']], [['fragColor', 'vec4']]),
    ]),
    shaderGroup('BLEND', ['Add', 'Multiply', 'Screen', 'Overlay', 'Difference', 'Exclusion', 'Soft Light'].map((label) => (
      shader(label, [['inputA', 'sampler2D'], ['inputB', 'sampler2D']], [['fragColor', 'vec4']])
    ))),
    shaderGroup('DISTORTION', [
      shader('Twirl', [['inputImage', 'sampler2D'], ['radius', 'float'], ['angle', 'float']], [['fragColor', 'vec4']]),
      shader('Ripple', [['inputImage', 'sampler2D'], ['frequency', 'float'], ['amplitude', 'float']], [['fragColor', 'vec4']]),
      shader('Displacement', [['displaceMap', 'sampler2D'], ['inputImage', 'sampler2D'], ['strength', 'float']], [['fragColor', 'vec4']]),
      shader('Barrel', [['inputImage', 'sampler2D'], ['k1', 'float'], ['k2', 'float']], [['fragColor', 'vec4']]),
      shader('Pinch', [['inputImage', 'sampler2D'], ['radius', 'float'], ['strength', 'float']], [['fragColor', 'vec4']]),
    ]),
    shaderGroup('FEEDBACK', [
      shader('Gray-Scott Reaction-Diffusion', [['dA', 'float'], ['dB', 'float'], ['feedRate', 'float'], ['killRate', 'float'], ['timestep', 'float']], [['fragColor', 'vec4']]),
    ]),
  ],
};

const byOnnxId = new Map(SNAPSHOT.onnxModels.map((entry) => [entry.id, entry]));
const byMathId = new Map(SNAPSHOT.mathOps.map((entry) => [entry.id, entry]));

export class Catalog {
  static snapshot(): CatalogSnapshot {
    return structuredClone(SNAPSHOT);
  }

  static onnxModel(id: string): OnnxModelDescriptor | undefined {
    const entry = byOnnxId.get(id);
    return entry ? structuredClone(entry) : undefined;
  }

  static mathOp(id: string): MathDescriptor | undefined {
    const entry = byMathId.get(id);
    return entry ? structuredClone(entry) : undefined;
  }
}

export function catalogSnapshot(): CatalogSnapshot {
  return Catalog.snapshot();
}

export function getOnnxModelDescriptor(id: string): OnnxModelDescriptor | undefined {
  return Catalog.onnxModel(id);
}

export const ONNX_MODEL_DESCRIPTORS: Readonly<Record<string, OnnxModelDescriptor>> = Object.freeze(
  Object.fromEntries(SNAPSHOT.onnxModels.map((entry) => [entry.id, entry])),
);

function model(
  id: string,
  label: string,
  task: OnnxTask,
  category: string,
  downloadUrl: string,
  fileSize: number,
  outputId: string,
  outputLabel: string,
): OnnxModelDescriptor {
  return {
    id,
    label,
    task,
    category,
    downloadUrl,
    fileSize,
    sha256: '',
    expectedIO: {
      inputs: [{ id: 'onnx_in_image', label: 'image', dataType: 'sampler2D', direction: 'input' }],
      outputs: [{ id: outputId, label: outputLabel, dataType: 'sampler2D', direction: 'output' }],
    },
  };
}

function shaderGroup(category: string, items: ShaderTemplateDescriptor[]): ShaderGroupDescriptor {
  return { category, items };
}

function shader(
  label: string,
  inputs: Array<[string, DataType]>,
  outputs: Array<[string, DataType]>,
): ShaderTemplateDescriptor {
  return {
    id: label.toLowerCase().replaceAll(' ', '-'),
    label,
    inputs: inputs.map(([portLabel, dataType]) => ({ label: portLabel, dataType })),
    outputs: outputs.map(([portLabel, dataType]) => ({ label: portLabel, dataType })),
  };
}
