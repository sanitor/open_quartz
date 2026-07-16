import { defineConfig } from 'vitest/config';
import { preview } from '@vitest/browser-preview';

/**
 * Shader bit-true tests via vitest browser mode.
 * Uses the system's default browser — no extra Chromium, same GPU driver as Tauri.
 * Opens a browser window briefly to run WebGL2 tests.
 *
 * Run with: npm run test:shaders
 */
export default defineConfig({
  test: {
    globals: true,
    include: ['tests/shaders/**/*.test.ts'],
    testTimeout: 30_000,
    browser: {
      enabled: true,
      provider: preview(),
      instances: [{ browser: 'preview' }],
      headless: false,
    },
  },
});
