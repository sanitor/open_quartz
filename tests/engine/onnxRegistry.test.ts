import { describe, it, expect } from 'vitest';
import { ONNX_MODELS, DEFAULT_ONNX_MODEL_ID } from '../../src/catalog/onnxRegistry';

describe('onnxRegistry', () => {
  it('DEFAULT_ONNX_MODEL_ID points at yolov8n', () => {
    expect(DEFAULT_ONNX_MODEL_ID).toBe('yolov8n');
  });

  it('ONNX_MODELS registry is non-empty and keyed by descriptor id', () => {
    const keys = Object.keys(ONNX_MODELS);
    expect(keys.length).toBeGreaterThan(0);
    for (const key of keys) {
      expect(ONNX_MODELS[key].id).toBe(key);
    }
  });

  it('default model resolves in the registry', () => {
    expect(ONNX_MODELS[DEFAULT_ONNX_MODEL_ID]).toBeDefined();
  });

  describe('yolov8n descriptor', () => {
    const desc = ONNX_MODELS.yolov8n;

    it('has stable identity fields', () => {
      expect(desc.id).toBe('yolov8n');
      expect(desc.label).toBe('YOLOv8n Detector');
      expect(desc.modelUrl).toBe('/models/yolov8n.onnx');
      expect(desc.targetSize).toBe(640);
    });

    it('ships default score / IoU thresholds', () => {
      expect(desc.scoreThreshold).toBe(0.25);
      expect(desc.iouThreshold).toBe(0.45);
    });

    it('has a human-readable description', () => {
      expect(typeof desc.description).toBe('string');
      expect(desc.description.length).toBeGreaterThan(0);
    });

    it('has exactly one sampler2D input port', () => {
      expect(desc.inputs).toHaveLength(1);
      const [inPort] = desc.inputs;
      expect(inPort.dataType).toBe('sampler2D');
      expect(inPort.direction).toBe('input');
      expect(inPort.label).toBe('image');
      expect(inPort.id).toBe('onnx_in_image');
    });

    it('has exactly two output ports (detections roi + overlay sampler2D)', () => {
      expect(desc.outputs).toHaveLength(2);

      const detections = desc.outputs.find((p) => p.label === 'detections');
      const overlay = desc.outputs.find((p) => p.label === 'overlay');
      expect(detections).toBeDefined();
      expect(overlay).toBeDefined();
      if (!detections || !overlay) throw new Error('missing port');

      expect(detections.dataType).toBe('roi');
      expect(detections.direction).toBe('output');
      expect(detections.id).toBe('onnx_out_detections');

      expect(overlay.dataType).toBe('sampler2D');
      expect(overlay.direction).toBe('output');
      expect(overlay.id).toBe('onnx_out_overlay');
    });
  });
});
