import { describe, it, expect } from 'vitest';
import { ONNX_CATALOG, CATALOG_CATEGORIES } from '../../src/catalog/onnxCatalog';
import type { CatalogEntry } from '../../src/catalog/onnxCatalog';

const catalogKeys = Object.keys(ONNX_CATALOG);
const catalogEntries = Object.entries(ONNX_CATALOG);

describe('ONNX_CATALOG structure', () => {
  it('is non-empty', () => {
    expect(catalogKeys.length).toBeGreaterThan(0);
  });

  it.each(catalogEntries)('%s — id matches its record key', (key, entry) => {
    expect(entry.id).toBe(key);
  });

  it.each(catalogEntries)('%s — has a non-empty label', (_key, entry) => {
    expect(typeof entry.label).toBe('string');
    expect(entry.label.length).toBeGreaterThan(0);
  });

  it.each(catalogEntries)('%s — has a valid OnnxTask', (_key, entry) => {
    const validTasks = [
      'super-resolution',
      'background-removal',
      'detection',
      'segmentation',
      'style-transfer',
      'denoising',
      'depth-estimation',
      'generic',
    ];
    expect(validTasks).toContain(entry.task);
  });

  it.each(catalogEntries)('%s — has a non-empty category string', (_key, entry) => {
    expect(typeof entry.category).toBe('string');
    expect(entry.category.length).toBeGreaterThan(0);
  });

  it.each(catalogEntries)('%s — downloadUrl is a valid https URL', (_key, entry) => {
    expect(entry.downloadUrl).toMatch(/^https:\/\/.+/);
  });

  it.each(catalogEntries)('%s — fileSize is a positive integer', (_key, entry) => {
    expect(Number.isInteger(entry.fileSize)).toBe(true);
    expect(entry.fileSize).toBeGreaterThan(0);
  });

  it.each(catalogEntries)('%s — sha256 is a string', (_key, entry) => {
    expect(typeof entry.sha256).toBe('string');
  });

  it.each(catalogEntries)(
    '%s — expectedIO has non-empty inputs and outputs arrays',
    (_key, entry) => {
      expect(Array.isArray(entry.expectedIO.inputs)).toBe(true);
      expect(entry.expectedIO.inputs.length).toBeGreaterThan(0);
      expect(Array.isArray(entry.expectedIO.outputs)).toBe(true);
      expect(entry.expectedIO.outputs.length).toBeGreaterThan(0);
    },
  );
});

describe('Port structure', () => {
  const allPorts = catalogEntries.flatMap(([key, entry]) => [
    ...entry.expectedIO.inputs.map((p) => ({ key, port: p })),
    ...entry.expectedIO.outputs.map((p) => ({ key, port: p })),
  ]);

  it.each(allPorts)('$key port $port.id — has id, label, dataType, direction', ({ port }) => {
    expect(typeof port.id).toBe('string');
    expect(port.id.length).toBeGreaterThan(0);
    expect(typeof port.label).toBe('string');
    expect(port.label.length).toBeGreaterThan(0);
    expect(typeof port.dataType).toBe('string');
    expect(port.dataType.length).toBeGreaterThan(0);
    expect(['input', 'output']).toContain(port.direction);
  });

  it.each(allPorts)(
    '$key port $port.id — direction matches its placement in inputs/outputs',
    ({ key, port }) => {
      const entry = ONNX_CATALOG[key];
      if (entry.expectedIO.inputs.includes(port)) {
        expect(port.direction).toBe('input');
      } else {
        expect(port.direction).toBe('output');
      }
    },
  );
});

describe('CATALOG_CATEGORIES', () => {
  it('is a non-empty array of strings', () => {
    expect(Array.isArray(CATALOG_CATEGORIES)).toBe(true);
    expect(CATALOG_CATEGORIES.length).toBeGreaterThan(0);
    for (const cat of CATALOG_CATEGORIES) {
      expect(typeof cat).toBe('string');
    }
  });

  it('is sorted alphabetically', () => {
    const sorted = [...CATALOG_CATEGORIES].sort();
    expect(CATALOG_CATEGORIES).toEqual(sorted);
  });

  it('contains no duplicates', () => {
    const unique = [...new Set(CATALOG_CATEGORIES)];
    expect(CATALOG_CATEGORIES).toEqual(unique);
  });

  it('every catalog entry category appears in CATALOG_CATEGORIES', () => {
    for (const [key, entry] of catalogEntries) {
      expect(
        CATALOG_CATEGORIES,
        `category "${entry.category}" of entry "${key}" missing from CATALOG_CATEGORIES`,
      ).toContain(entry.category);
    }
  });

  it('every category in CATALOG_CATEGORIES has at least one catalog entry', () => {
    const usedCategories = new Set(Object.values(ONNX_CATALOG).map((e) => e.category));
    for (const cat of CATALOG_CATEGORIES) {
      expect(usedCategories.has(cat), `category "${cat}" has no catalog entries`).toBe(true);
    }
  });
});

describe('entry-specific checks', () => {
  describe('yolov8n', () => {
    const entry: CatalogEntry = ONNX_CATALOG['yolov8n'];

    it('exists in the catalog', () => {
      expect(entry).toBeDefined();
    });

    it('has task "detection"', () => {
      expect(entry.task).toBe('detection');
    });

    it('has defaultParams with scoreThreshold and iouThreshold', () => {
      expect(entry.defaultParams).toBeDefined();
      const params = entry.defaultParams!;
      expect(params.scoreThreshold).toBeDefined();
      expect(params.scoreThreshold.type).toBe('float');
      expect(params.scoreThreshold.default).toBe(0.25);
      expect(params.scoreThreshold.min).toBe(0);
      expect(params.scoreThreshold.max).toBe(1);

      expect(params.iouThreshold).toBeDefined();
      expect(params.iouThreshold.type).toBe('float');
      expect(params.iouThreshold.default).toBe(0.45);
      expect(params.iouThreshold.min).toBe(0);
      expect(params.iouThreshold.max).toBe(1);
    });

    it('has detection output ports (detections roi + overlay sampler2D)', () => {
      expect(entry.expectedIO.outputs.length).toBe(2);
      const det = entry.expectedIO.outputs.find((p) => p.id === 'onnx_out_detections');
      const ovl = entry.expectedIO.outputs.find((p) => p.id === 'onnx_out_overlay');
      expect(det).toBeDefined();
      expect(det!.dataType).toBe('roi');
      expect(ovl).toBeDefined();
      expect(ovl!.dataType).toBe('sampler2D');
    });
  });

  describe('super-resolution-3x', () => {
    const entry: CatalogEntry = ONNX_CATALOG['super-resolution-3x'];

    it('exists in the catalog', () => {
      expect(entry).toBeDefined();
    });

    it('has task "super-resolution"', () => {
      expect(entry.task).toBe('super-resolution');
    });

    it('has no defaultParams', () => {
      expect(entry.defaultParams).toBeUndefined();
    });

    it('has a single upscaled output', () => {
      expect(entry.expectedIO.outputs.length).toBe(1);
      expect(entry.expectedIO.outputs[0].id).toBe('onnx_out_upscaled');
      expect(entry.expectedIO.outputs[0].dataType).toBe('sampler2D');
    });
  });

  describe('realesrgan-x4', () => {
    const entry: CatalogEntry = ONNX_CATALOG['realesrgan-x4'];

    it('exists in the catalog', () => {
      expect(entry).toBeDefined();
    });

    it('has task "super-resolution"', () => {
      expect(entry.task).toBe('super-resolution');
    });

    it('has no defaultParams', () => {
      expect(entry.defaultParams).toBeUndefined();
    });

    it('has a single upscaled output', () => {
      expect(entry.expectedIO.outputs.length).toBe(1);
      expect(entry.expectedIO.outputs[0].id).toBe('onnx_out_upscaled');
      expect(entry.expectedIO.outputs[0].dataType).toBe('sampler2D');
    });
  });
});
