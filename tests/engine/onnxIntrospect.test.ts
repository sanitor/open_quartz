import { describe, it, expect } from 'vitest';
import type { OnnxModelMeta } from '../../src/engine/onnx/introspect';
import { inferTaskFromMeta, metaToDefaultPorts } from '../../src/engine/onnx/introspect';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Shorthand to build a minimal OnnxModelMeta. */
function makeMeta(
  overrides: Partial<OnnxModelMeta> = {},
): OnnxModelMeta {
  return {
    opset: 11,
    inputs: [{ name: 'input', shape: [], dtype: 'float32' }],
    outputs: [{ name: 'output', shape: [], dtype: 'float32' }],
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// inferTaskFromMeta
// ---------------------------------------------------------------------------

describe('inferTaskFromMeta', () => {
  describe('detection by shape', () => {
    it.each([
      { lastDim: 5, label: 'exactly 5 (bbox + score)' },
      { lastDim: 85, label: '85 (YOLOv5 80-class)' },
      { lastDim: 8400, label: 'large lastDim' },
    ])(
      'returns "detection" when single output lastDim=$lastDim ($label)',
      ({ lastDim }) => {
        const meta = makeMeta({
          outputs: [{ name: 'det', shape: [1, 25200, lastDim], dtype: 'float32' }],
        });
        expect(inferTaskFromMeta(meta)).toBe('detection');
      },
    );

    it('falls back to super-resolution for 1-in 1-out when lastDim < 5', () => {
      const meta = makeMeta({
        outputs: [{ name: 'out', shape: [1, 100, 4], dtype: 'float32' }],
      });
      // lastDim=4 < 5 → not detection; 1-in 1-out → super-resolution fallback
      expect(inferTaskFromMeta(meta)).toBe('super-resolution');
    });

    it('does NOT trigger shape-based detection when there are multiple outputs', () => {
      // lastDim >= 5 but outputs.length !== 1 → skips shape detection,
      // but >= 3 outputs still triggers count-based detection fallback
      const meta = makeMeta({
        outputs: [
          { name: 'boxes', shape: [1, 100, 6], dtype: 'float32' },
          { name: 'scores', shape: [1, 100], dtype: 'float32' },
          { name: 'classes', shape: [1, 100], dtype: 'float32' },
        ],
      });
      expect(inferTaskFromMeta(meta)).toBe('detection');
    });
  });

  describe('super-resolution by shape', () => {
    // NOTE: the detection heuristic (lastDim ≥ 5) runs before the SR check.
    // For 4-dim [N,C,H,W] shapes, lastDim = W. If W ≥ 5 the model is
    // classified as detection, so SR is only reachable when W < 5.

    it('returns "super-resolution" when output spatial > input spatial (2× upscale)', () => {
      const meta = makeMeta({
        inputs: [{ name: 'lr', shape: [1, 3, 2, 2], dtype: 'float32' }],
        outputs: [{ name: 'hr', shape: [1, 3, 4, 4], dtype: 'float32' }],
      });
      expect(inferTaskFromMeta(meta)).toBe('super-resolution');
    });

    it('returns "super-resolution" for asymmetric upscale', () => {
      const meta = makeMeta({
        inputs: [{ name: 'lr', shape: [1, 3, 1, 1], dtype: 'float32' }],
        outputs: [{ name: 'hr', shape: [1, 3, 4, 3], dtype: 'float32' }],
      });
      expect(inferTaskFromMeta(meta)).toBe('super-resolution');
    });

    it('4D image output with larger spatial dims is super-resolution, not detection', () => {
      // 4D output [N,C,H,W] is an image tensor — super-resolution fires first
      // even when lastDim >= 5, because the outShape.length <= 3 guard
      // prevents detection from matching 4D image tensors.
      const meta = makeMeta({
        inputs: [{ name: 'lr', shape: [1, 3, 64, 64], dtype: 'float32' }],
        outputs: [{ name: 'hr', shape: [1, 3, 128, 128], dtype: 'float32' }],
      });
      expect(inferTaskFromMeta(meta)).toBe('super-resolution');
    });

    it('falls back to image-to-image when output is same size as input (1-in 1-out)', () => {
      const meta = makeMeta({
        inputs: [{ name: 'in', shape: [1, 3, 2, 2], dtype: 'float32' }],
        outputs: [{ name: 'out', shape: [1, 3, 2, 2], dtype: 'float32' }],
      });
      // Shape-based SR fails (not bigger), but 1-in 1-out → super-resolution fallback
      expect(inferTaskFromMeta(meta)).toBe('super-resolution');
    });

    it('falls back to image-to-image when output is smaller than input (1-in 1-out)', () => {
      const meta = makeMeta({
        inputs: [{ name: 'in', shape: [1, 3, 4, 4], dtype: 'float32' }],
        outputs: [{ name: 'out', shape: [1, 3, 2, 2], dtype: 'float32' }],
      });
      expect(inferTaskFromMeta(meta)).toBe('super-resolution');
    });

    it('falls back to image-to-image when only height is larger (1-in 1-out)', () => {
      const meta = makeMeta({
        inputs: [{ name: 'in', shape: [1, 3, 2, 3], dtype: 'float32' }],
        outputs: [{ name: 'out', shape: [1, 3, 4, 3], dtype: 'float32' }],
      });
      expect(inferTaskFromMeta(meta)).toBe('super-resolution');
    });

    it('falls back to image-to-image with non-4-dim shapes (1-in 1-out)', () => {
      const meta = makeMeta({
        inputs: [{ name: 'in', shape: [1, 3, 2], dtype: 'float32' }],
        outputs: [{ name: 'out', shape: [1, 3, 4], dtype: 'float32' }],
      });
      expect(inferTaskFromMeta(meta)).toBe('super-resolution');
    });

    it('skips SR heuristic when multiple inputs', () => {
      const meta = makeMeta({
        inputs: [
          { name: 'a', shape: [1, 3, 2, 2], dtype: 'float32' },
          { name: 'b', shape: [1, 3, 2, 2], dtype: 'float32' },
        ],
        outputs: [{ name: 'hr', shape: [1, 3, 4, 4], dtype: 'float32' }],
      });
      expect(inferTaskFromMeta(meta)).toBe('generic');
    });

    it('falls back to image-to-image with symbolic dims (1-in 1-out)', () => {
      const meta = makeMeta({
        inputs: [{ name: 'in', shape: [1, 3, 'h', 'w'], dtype: 'float32' }],
        outputs: [{ name: 'out', shape: [1, 3, 'h2', 'w2'], dtype: 'float32' }],
      });
      expect(inferTaskFromMeta(meta)).toBe('super-resolution');
    });
  });

  describe('detection by output count', () => {
    it('returns "detection" when >= 3 outputs (boxes + scores + classes pattern)', () => {
      const meta = makeMeta({
        outputs: [
          { name: 'boxes', shape: [], dtype: 'float32' },
          { name: 'scores', shape: [], dtype: 'float32' },
          { name: 'labels', shape: [], dtype: 'int64' },
        ],
      });
      expect(inferTaskFromMeta(meta)).toBe('detection');
    });

    it('returns "detection" with more than 3 outputs', () => {
      const meta = makeMeta({
        outputs: [
          { name: 'a', shape: [], dtype: 'float32' },
          { name: 'b', shape: [], dtype: 'float32' },
          { name: 'c', shape: [], dtype: 'float32' },
          { name: 'd', shape: [], dtype: 'float32' },
        ],
      });
      expect(inferTaskFromMeta(meta)).toBe('detection');
    });

    it('does NOT trigger output-count detection with 2 outputs', () => {
      const meta = makeMeta({
        outputs: [
          { name: 'a', shape: [], dtype: 'float32' },
          { name: 'b', shape: [], dtype: 'float32' },
        ],
      });
      expect(inferTaskFromMeta(meta)).toBe('generic');
    });
  });

  describe('generic fallback', () => {
    it('returns "super-resolution" for single input/output with no shape info', () => {
      const meta = makeMeta({
        outputs: [{ name: 'output', shape: [], dtype: 'float32' }],
      });
      expect(inferTaskFromMeta(meta)).toBe('super-resolution');
    });

    it('returns "generic" with two outputs and no shape info', () => {
      const meta = makeMeta({
        outputs: [
          { name: 'a', shape: [], dtype: 'float32' },
          { name: 'b', shape: [], dtype: 'float32' },
        ],
      });
      expect(inferTaskFromMeta(meta)).toBe('generic');
    });
  });

  describe('edge cases', () => {
    it('returns "generic" when outputs array is empty', () => {
      const meta = makeMeta({ outputs: [] });
      expect(inferTaskFromMeta(meta)).toBe('generic');
    });

    it('returns "generic" when inputs array is empty', () => {
      const meta = makeMeta({ inputs: [], outputs: [{ name: 'o', shape: [], dtype: 'float32' }] });
      expect(inferTaskFromMeta(meta)).toBe('generic');
    });

    it('handles output shape with string lastDim (symbolic) — 1-in 1-out fallback', () => {
      const meta = makeMeta({
        outputs: [{ name: 'det', shape: [1, 'N', 'num_classes'], dtype: 'float32' }],
      });
      // lastDim is string → shape heuristic skipped → 1-in 1-out → super-resolution
      expect(inferTaskFromMeta(meta)).toBe('super-resolution');
    });
  });
});

// ---------------------------------------------------------------------------
// metaToDefaultPorts
// ---------------------------------------------------------------------------

describe('metaToDefaultPorts', () => {
  describe('input ports', () => {
    it('generates sampler2D input ports with correct id/label', () => {
      const meta = makeMeta({
        inputs: [
          { name: 'image', shape: [], dtype: 'float32' },
          { name: 'mask', shape: [], dtype: 'float32' },
        ],
      });
      const { inputs } = metaToDefaultPorts(meta);
      expect(inputs).toHaveLength(2);
      expect(inputs[0]).toEqual({
        id: 'onnx_in_image',
        label: 'image',
        dataType: 'sampler2D',
        direction: 'input',
      });
      expect(inputs[1]).toEqual({
        id: 'onnx_in_mask',
        label: 'mask',
        dataType: 'sampler2D',
        direction: 'input',
      });
    });

    it('falls back to index for unnamed inputs', () => {
      const meta = makeMeta({
        inputs: [
          { name: '', shape: [], dtype: 'float32' },
          { name: '', shape: [], dtype: 'float32' },
        ],
      });
      const { inputs } = metaToDefaultPorts(meta);
      expect(inputs[0].id).toBe('onnx_in_0');
      expect(inputs[0].label).toBe('input_0');
      expect(inputs[1].id).toBe('onnx_in_1');
      expect(inputs[1].label).toBe('input_1');
    });

    it('returns empty inputs array when meta has no inputs', () => {
      const meta = makeMeta({ inputs: [] });
      const { inputs } = metaToDefaultPorts(meta);
      expect(inputs).toEqual([]);
    });
  });

  describe('output ports — dataType per task', () => {
    it.each([
      { task: 'detection' as const, expectedType: 'roi' },
      { task: 'super-resolution' as const, expectedType: 'sampler2D' },
      { task: 'style-transfer' as const, expectedType: 'sampler2D' },
      { task: 'denoising' as const, expectedType: 'sampler2D' },
      { task: 'depth-estimation' as const, expectedType: 'sampler2D' },
      { task: 'segmentation' as const, expectedType: 'sampler2D' },
      { task: 'background-removal' as const, expectedType: 'sampler2D' },
      { task: 'generic' as const, expectedType: 'sampler2D' },
    ])(
      'outputs have dataType=$expectedType when inferredTask=$task',
      ({ task, expectedType }) => {
        const meta = makeMeta({ inferredTask: task });
        const { outputs } = metaToDefaultPorts(meta);
        expect(outputs).toHaveLength(1);
        expect(outputs[0].dataType).toBe(expectedType);
        expect(outputs[0].direction).toBe('output');
      },
    );

    it('defaults to "sampler2D" when inferredTask is undefined', () => {
      const meta = makeMeta();
      const { outputs } = metaToDefaultPorts(meta);
      expect(outputs[0].dataType).toBe('sampler2D');
    });
  });

  describe('output port id/label', () => {
    it('uses output name for id and label', () => {
      const meta = makeMeta({
        inferredTask: 'detection',
        outputs: [{ name: 'boxes', shape: [], dtype: 'float32' }],
      });
      const { outputs } = metaToDefaultPorts(meta);
      expect(outputs[0].id).toBe('onnx_out_boxes');
      expect(outputs[0].label).toBe('boxes');
    });

    it('falls back to index for unnamed outputs', () => {
      const meta = makeMeta({
        inferredTask: 'generic',
        outputs: [
          { name: '', shape: [], dtype: 'float32' },
          { name: '', shape: [], dtype: 'float32' },
        ],
      });
      const { outputs } = metaToDefaultPorts(meta);
      expect(outputs[0].id).toBe('onnx_out_0');
      expect(outputs[0].label).toBe('output_0');
      expect(outputs[1].id).toBe('onnx_out_1');
      expect(outputs[1].label).toBe('output_1');
    });

    it('returns empty outputs array when meta has no outputs', () => {
      const meta = makeMeta({ outputs: [] });
      const { outputs } = metaToDefaultPorts(meta);
      expect(outputs).toEqual([]);
    });
  });

  describe('integration: inferTaskFromMeta → metaToDefaultPorts', () => {
    it('detection-shaped model gets roi output ports', () => {
      const meta = makeMeta({
        outputs: [{ name: 'detections', shape: [1, 25200, 85], dtype: 'float32' }],
      });
      meta.inferredTask = inferTaskFromMeta(meta);
      const { outputs } = metaToDefaultPorts(meta);
      expect(meta.inferredTask).toBe('detection');
      expect(outputs[0].dataType).toBe('roi');
    });

    it('SR-shaped model gets sampler2D output ports', () => {
      const meta = makeMeta({
        inputs: [{ name: 'lr', shape: [1, 3, 2, 2], dtype: 'float32' }],
        outputs: [{ name: 'hr', shape: [1, 3, 4, 4], dtype: 'float32' }],
      });
      meta.inferredTask = inferTaskFromMeta(meta);
      const { outputs } = metaToDefaultPorts(meta);
      expect(meta.inferredTask).toBe('super-resolution');
      expect(outputs[0].dataType).toBe('sampler2D');
    });

    it('unknown model with no shape info gets sampler2D output ports (image-to-image default)', () => {
      const meta = makeMeta();
      meta.inferredTask = inferTaskFromMeta(meta);
      const { outputs } = metaToDefaultPorts(meta);
      expect(meta.inferredTask).toBe('super-resolution');
      expect(outputs[0].dataType).toBe('sampler2D');
    });
  });
});
