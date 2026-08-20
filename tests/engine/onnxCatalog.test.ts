import { describe, it, expect } from 'vitest';
import { ONNX_CATALOG, CATALOG_CATEGORIES } from '../../src/catalog/onnxCatalog';
import { Catalog, getOnnxModelDescriptor } from '../../src/sdk/catalog';

const catalogKeys = Object.keys(ONNX_CATALOG);
const catalogEntries = Object.entries(ONNX_CATALOG);

describe('ONNX catalog metadata projection', () => {
  it('is non-empty and keeps menu categories sorted', () => {
    expect(catalogKeys.length).toBeGreaterThan(0);
    expect(CATALOG_CATEGORIES).toEqual([...CATALOG_CATEGORIES].sort());
    expect(new Set(CATALOG_CATEGORIES).size).toBe(CATALOG_CATEGORIES.length);
  });

  it.each(catalogEntries)('%s exposes only UI metadata', (key, entry) => {
    expect(entry.id).toBe(key);
    expect(entry.label.length).toBeGreaterThan(0);
    expect(entry.category.length).toBeGreaterThan(0);
    expect(entry.taskLabel.length).toBeGreaterThan(0);
    expect(entry).not.toHaveProperty('downloadUrl');
    expect(entry).not.toHaveProperty('fileSize');
    expect(entry).not.toHaveProperty('sha256');
    expect(entry).not.toHaveProperty('expectedIO');
    expect(entry).not.toHaveProperty('defaultParams');
  });

  it('matches the SDK catalog descriptor labels and categories', () => {
    const snapshot = Catalog.snapshot();
    expect(CATALOG_CATEGORIES).toEqual(snapshot.onnxCategories);
    for (const descriptor of snapshot.onnxModels) {
      expect(ONNX_CATALOG[descriptor.id]).toMatchObject({
        id: descriptor.id,
        label: descriptor.label,
        category: descriptor.category,
        taskLabel: descriptor.task,
      });
    }
  });
});

describe('ONNX SDK descriptor contract', () => {
  it('freezes yolov8n task, defaults, IO, and integrity fields', () => {
    const entry = getOnnxModelDescriptor('yolov8n')!;
    expect(entry.task).toBe('detection');
    expect(entry.downloadUrl).toMatch(/^https:\/\/.+/);
    expect(entry.fileSize).toBe(12_851_098);
    expect(entry.sha256).toBe('');
    expect(entry.defaultParams?.scoreThreshold.default).toBe(0.25);
    expect(entry.defaultParams?.iouThreshold.default).toBe(0.45);
    expect(entry.expectedIO.outputs.map((port) => [port.id, port.dataType])).toEqual([
      ['onnx_out_detections', 'roi'],
      ['onnx_out_overlay', 'sampler2D'],
    ]);
  });

  it('freezes super-resolution defaults and output contract', () => {
    const entry = getOnnxModelDescriptor('super-resolution-3x')!;
    expect(entry.task).toBe('super-resolution');
    expect(entry.defaultParams).toBeUndefined();
    expect(entry.expectedIO.outputs).toEqual([
      { id: 'onnx_out_upscaled', label: 'upscaled', dataType: 'sampler2D', direction: 'output' },
    ]);
  });
});
