import { defineConfig } from 'vitest/config';

/**
 * Functional / integration tests that use real ONNX models and real inference.
 * Runs in Node environment (onnxruntime-node, not jsdom).
 *
 * Separated from the unit test suite because these tests:
 * - Download real models from the internet (~100MB+ total)
 * - Run real inference (~seconds per model)
 * - Cannot run in jsdom (no WebGL, no ORT web)
 *
 * Run with: npm run test:models
 */
export default defineConfig({
  test: {
    globals: true,
    environment: 'node',
    include: ['tests/functional/**/*.test.ts'],
    pool: 'forks',
    testTimeout: 120_000,
    hookTimeout: 120_000,
  },
});
