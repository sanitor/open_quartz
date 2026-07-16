import { defineConfig } from 'vitest/config';
import { resolve } from 'node:path';

/**
 * Functional / integration tests that use real ONNX models and real inference.
 * Runs in Node environment (onnxruntime-node, not jsdom).
 *
 * Separated from the unit test suite because these tests:
 * - Download real models from the internet (~100MB+ total)
 * - Run real inference (~seconds per model)
 * - Cannot run in jsdom (no WebGL, no ORT web)
 *
 * Run with: npm run test:functional
 */
export default defineConfig({
  resolve: {
    alias: {
      '@nodes/yolo-detector': resolve(__dirname, 'rust/crates/yolo-detector/pkg/yolo_detector.js'),
    },
  },
  test: {
    globals: true,
    environment: 'node',
    include: ['tests/functional/**/*.test.ts'],
    pool: 'forks',         // isolate ORT sessions across test files
    testTimeout: 120_000,
    hookTimeout: 120_000,
  },
});
