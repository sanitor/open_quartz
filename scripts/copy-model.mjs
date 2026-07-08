#!/usr/bin/env node
// Copy the yolov8n.onnx model from the upstream `rimeflow-yolov8n` git checkout
// (fetched by Cargo when wasm-pack builds `rust/crates/yolo-detector/`) into
// `public/models/`. The model is a build artifact — always version-locked to
// whatever revision the yolo-detector Cargo.toml pins, never committed here.
//
// Prereq: `npm run build:wasm` must have run at least once so Cargo has fetched
// the git dep. If the checkout is missing we run `cargo fetch` first.
//
// Usage:
//   node scripts/copy-model.mjs

import { existsSync, mkdirSync, copyFileSync, statSync } from 'node:fs';
import { resolve, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = resolve(__dirname, '..');
const cratePath = resolve(root, 'rust/crates/yolo-detector');
const dstDir = resolve(root, 'public/models');
const dstFile = join(dstDir, 'yolov8n.onnx');

function cargoMetadata() {
  const stdout = execFileSync(
    'cargo',
    ['metadata', '--format-version', '1'],
    { cwd: cratePath, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'], maxBuffer: 64 * 1024 * 1024 },
  );
  return JSON.parse(stdout);
}

let meta;
try {
  meta = cargoMetadata();
} catch (err) {
  console.error('[copy-model] cargo metadata failed:', err.message);
  console.error('[copy-model] make sure Rust + Cargo are installed and rust/crates/yolo-detector is intact.');
  process.exit(1);
}

const pkg = meta.packages.find((p) => p.name === 'rimeflow-yolov8n');
if (!pkg) {
  console.error('[copy-model] rimeflow-yolov8n not found in cargo metadata — is it still a dependency of yolo-detector?');
  process.exit(1);
}

const upstreamRoot = dirname(pkg.manifest_path);
const srcFile = join(upstreamRoot, 'models', 'yolov8n.onnx');

if (!existsSync(srcFile)) {
  console.error(
    `[copy-model] source model not found at ${srcFile}\n` +
    '            Run `npm run build:wasm` once to make Cargo materialize the checkout.',
  );
  process.exit(1);
}

// Skip when already up-to-date (mtime + size heuristic).
if (existsSync(dstFile)) {
  const s = statSync(srcFile);
  const d = statSync(dstFile);
  if (s.size === d.size && d.mtimeMs >= s.mtimeMs) {
    console.log(`[copy-model] up-to-date (${(d.size / 1024 / 1024).toFixed(1)} MB)`);
    process.exit(0);
  }
}

mkdirSync(dstDir, { recursive: true });
copyFileSync(srcFile, dstFile);
const size = (statSync(dstFile).size / 1024 / 1024).toFixed(1);
console.log(`[copy-model] copied yolov8n.onnx (${size} MB) from ${upstreamRoot}`);
