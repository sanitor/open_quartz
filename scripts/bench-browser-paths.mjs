#!/usr/bin/env node
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { performance } from 'node:perf_hooks';

const args = new Map();
for (let index = 2; index < process.argv.length; index++) {
  const arg = process.argv[index];
  if (!arg.startsWith('--')) continue;
  const [key, inlineValue] = arg.slice(2).split('=', 2);
  const value = inlineValue ?? (process.argv[index + 1]?.startsWith('--') ? 'true' : process.argv[++index]);
  args.set(key, value);
}

const variant = args.get('variant') ?? 'baseline';
const outputPath = args.get('out')
  ?? `artifacts/task_4324e956da82_browser_paths_${variant}.json`;

function deterministicBytes(byteLength) {
  const bytes = new Uint8Array(byteLength);
  let state = 0x7a5f1d2b;
  for (let index = 0; index < bytes.length; index++) {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    bytes[index] = state & 0xff;
  }
  return bytes;
}

function currentBrowserDataUrl(bytes, mimeType) {
  let binary = '';
  for (let offset = 0; offset < bytes.length; offset += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
  }
  return `data:${mimeType};base64,${btoa(binary)}`;
}

function nodeNativeReferenceDataUrl(bytes, mimeType) {
  return `data:${mimeType};base64,${Buffer.from(bytes).toString('base64')}`;
}

function timeOperation(iterations, operation) {
  const warmup = Math.min(8, iterations);
  for (let index = 0; index < warmup; index++) operation(index);
  const start = performance.now();
  let checksum = 0;
  for (let index = 0; index < iterations; index++) {
    checksum ^= operation(index).length;
  }
  const totalMs = performance.now() - start;
  return {
    iterations,
    totalMs,
    meanMs: totalMs / iterations,
    checksum,
  };
}

function benchDataUrls() {
  const fixtures = [
    { name: 'preview-png-256kib', byteLength: 256 * 1024, iterations: 80 },
    { name: 'preview-png-2mib', byteLength: 2 * 1024 * 1024, iterations: 16 },
  ];
  return fixtures.map((fixture) => {
    const bytes = deterministicBytes(fixture.byteLength);
    const reference = nodeNativeReferenceDataUrl(bytes, 'image/png');
    const current = currentBrowserDataUrl(bytes, 'image/png');
    if (current !== reference) {
      throw new Error(`data URL mismatch for ${fixture.name}`);
    }
    return {
      ...fixture,
      currentBrowserByteString: timeOperation(
        fixture.iterations,
        () => currentBrowserDataUrl(bytes, 'image/png'),
      ),
      nodeNativeReference: timeOperation(
        fixture.iterations,
        () => nodeNativeReferenceDataUrl(bytes, 'image/png'),
      ),
    };
  });
}

function currentFrameInputs(iterations) {
  let total = 0;
  for (let index = 0; index < iterations; index++) {
    const date = new Date(1_700_000_000_000 + index * 16);
    const dateInput = new Float32Array([
      date.getFullYear(),
      date.getMonth() + 1,
      date.getDate(),
      date.getHours() * 3600 + date.getMinutes() * 60 + date.getSeconds(),
    ]);
    const mouseInput = new Float32Array(4);
    total += dateInput[0] + dateInput[1] + dateInput[2] + dateInput[3] + mouseInput[0];
  }
  return total;
}

function reusedFrameInputs(iterations) {
  const dateInput = new Float32Array(4);
  const mouseInput = new Float32Array(4);
  let total = 0;
  for (let index = 0; index < iterations; index++) {
    const date = new Date(1_700_000_000_000 + index * 16);
    dateInput[0] = date.getFullYear();
    dateInput[1] = date.getMonth() + 1;
    dateInput[2] = date.getDate();
    dateInput[3] = date.getHours() * 3600 + date.getMinutes() * 60 + date.getSeconds();
    mouseInput.fill(0);
    total += dateInput[0] + dateInput[1] + dateInput[2] + dateInput[3] + mouseInput[0];
  }
  return total;
}

function benchFrameInputs() {
  const iterations = 1_000_000;
  const currentTotal = currentFrameInputs(1024);
  const reusedTotal = reusedFrameInputs(1024);
  if (currentTotal !== reusedTotal) throw new Error('frame input totals diverged');
  return {
    iterations,
    currentAllocatingInputs: timeOperation(iterations, () => String(currentFrameInputs(1))),
    reusedTypedArrayInputs: timeOperation(iterations, () => String(reusedFrameInputs(1))),
  };
}

const result = {
  taskId: 'task_4324e956da82',
  variant,
  timestamp: new Date().toISOString(),
  runtime: {
    node: process.version,
    platform: process.platform,
    arch: process.arch,
    fileReaderAvailable: typeof FileReader !== 'undefined',
    offscreenCanvasAvailable: typeof OffscreenCanvas !== 'undefined',
    imageBitmapAvailable: typeof ImageBitmap !== 'undefined',
  },
  notes: [
    'Data URL fixture measures the existing worker byte-string path after PNG bytes already exist.',
    'nodeNativeReference is a correctness and lower-bound reference only; it is not a browser production path.',
    'Frame input fixture measures the per-frame Date plus vec4 construction used by BrowserRuntimeWorker.',
  ],
  dataUrl: benchDataUrls(),
  frameInputs: benchFrameInputs(),
};

mkdirSync(dirname(resolve(outputPath)), { recursive: true });
writeFileSync(outputPath, `${JSON.stringify(result, null, 2)}\n`);
console.log(JSON.stringify(result, null, 2));
