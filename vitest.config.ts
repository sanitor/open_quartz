import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import { resolve } from 'node:path';

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      '@nodes/yolo-detector': resolve(__dirname, 'rust/crates/yolo-detector/pkg/yolo_detector.js'),
      '@nodes/yolo-sem': resolve(__dirname, 'rust/crates/yolo-sem/pkg/yolo_sem.js'),
    },
  },
  test: {
    globals: true,
    environment: 'jsdom',
    setupFiles: ['./tests/setup.ts'],
    include: ['tests/**/*.test.{ts,tsx}'],
    exclude: ['tests/functional/**', 'tests/shaders/**'],
    testTimeout: 10000,
    coverage: {
      provider: 'istanbul',
      reporter: ['text', 'text-summary', 'lcov', 'json-summary'],
      reportsDirectory: './coverage',
      include: ['src/**/*.{ts,tsx}'],
      exclude: [
        'src/main.tsx',
        'src/vite-env.d.ts',
        'src/version.ts',
        'src/components/NodeGraph/index.tsx',
        'src/engine/clock.ts',
        'src/engine/compositor.ts',
        'src/engine/realtimeHost.ts',
        'src/engine/mouseState.ts',
        'src/engine/videoSource.ts',
        'src/utils/tauri.ts',
      ],
      thresholds: {
        lines: 70,
        statements: 70,
        branches: 55,
        functions: 64,
      },
    },
  },
});
