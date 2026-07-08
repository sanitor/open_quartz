// Static descriptors for the ONNX models bundled with OpenQuartz.
// Adding a new model = adding a registry entry + shipping the .onnx file.

import type { Port } from '../types';

export interface OnnxModelDescriptor {
  id: string;
  label: string;
  modelUrl: string;      // fetched at runtime by ORT
  targetSize: number;    // model input side length (e.g. 640)
  scoreThreshold: number;
  iouThreshold: number;
  inputs: Port[];
  outputs: Port[];
  description: string;
}

export const ONNX_MODELS: Record<string, OnnxModelDescriptor> = {
  yolov8n: {
    id: 'yolov8n',
    label: 'YOLOv8n Detector',
    modelUrl: '/models/yolov8n.onnx',
    targetSize: 640,
    scoreThreshold: 0.25,
    iouThreshold: 0.45,
    description: 'Ultralytics YOLOv8n, 80 COCO classes, ~6MB',
    inputs: [
      {
        id: 'onnx_in_image',
        label: 'image',
        dataType: 'sampler2D',
        direction: 'input',
      },
    ],
    outputs: [
      {
        id: 'onnx_out_detections',
        label: 'detections',
        dataType: 'roi',
        direction: 'output',
      },
      {
        id: 'onnx_out_overlay',
        label: 'overlay',
        dataType: 'sampler2D',
        direction: 'output',
      },
    ],
  },
};

export const DEFAULT_ONNX_MODEL_ID = 'yolov8n';
