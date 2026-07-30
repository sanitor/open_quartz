import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { initializeSdk } from '../src/sdk/runtime';
import type { RawWasmBindings } from '../src/sdk';

const require = createRequire(import.meta.url);
const nodeBindings = require(
  resolve('node_modules/.tmp/open_quartz-sdk-node/open_quartz.js'),
) as Omit<RawWasmBindings, 'default'>;

await initializeSdk(async () => ({
  ...nodeBindings,
  default: async () => undefined,
}));

import '@testing-library/jest-dom/vitest';
import { vi } from 'vitest';

// Mock URL.createObjectURL / revokeObjectURL
if (typeof URL.createObjectURL === 'undefined') {
  URL.createObjectURL = vi.fn(() => 'blob:mock-url');
}
if (typeof URL.revokeObjectURL === 'undefined') {
  URL.revokeObjectURL = vi.fn();
}
