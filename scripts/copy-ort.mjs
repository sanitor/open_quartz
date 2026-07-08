#!/usr/bin/env node
// Copy onnxruntime-web runtime files into public/ort/ so they can be
// loaded from a plain <script src="/ort/ort.min.js"> tag without going
// through the bundler.
//
// Prereq: `npm i -D onnxruntime-web` (add manually when you first want
// to use the ONNX node). This script is idempotent — if the source
// files are missing, it prints a hint and exits 0 so `npm run dev`
// doesn't fail for people who don't need the ONNX node.
//
// Usage:
//   node scripts/copy-ort.mjs

import { existsSync, mkdirSync, copyFileSync, readdirSync } from 'node:fs';
import { resolve, join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = resolve(__dirname, '..');
const src = resolve(root, 'node_modules/onnxruntime-web/dist');
const dst = resolve(root, 'public/ort');
if (!existsSync(src)) {
  console.error(
    '\n[copy-ort] onnxruntime-web is not installed. The ONNX node will fail at runtime\n' +
    '           with "Failed to load /ort/ort.min.js" until you run:\n' +
    '             npm i -D onnxruntime-web\n' +
    '           (this script is idempotent — rerun after install to populate public/ort/)\n',
  );
  process.exit(0);
}

mkdirSync(dst, { recursive: true });

const wanted = [
  /^ort\.min\.js(?:\.map)?$/,
  /^ort-wasm.*\.wasm$/,
  /^ort-wasm.*\.mjs$/,
];

let copied = 0;
for (const name of readdirSync(src)) {
  if (!wanted.some((re) => re.test(name))) continue;
  copyFileSync(join(src, name), join(dst, name));
  copied++;
}
console.log(`[copy-ort] copied ${copied} file(s) to ${dst}`);
