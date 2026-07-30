#!/usr/bin/env node

import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { pathToFileURL, fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const packageDir = resolve(root, 'public/open_quartz-sdk');
const bindings = await import(pathToFileURL(resolve(packageDir, 'open_quartz.js')).href);
const wasm = await readFile(resolve(packageDir, 'open_quartz_bg.wasm'));
bindings.initSync({ module: wasm });

assert.equal(bindings.apiVersion(), 1);
const capabilities = JSON.parse(bindings.capabilities());
assert.equal(capabilities.structuredEngine, true);
assert.equal(capabilities.typedFramePlanning, true);
assert.equal(capabilities.resourceGenerations, true);
assert.equal(capabilities.gpuExecution, false);

const engine = new bindings.Engine();
assert.equal(engine.engineState(), 'empty');
assert.equal(engine.setGraph('{"nodes":[],"edges":[]}'), 1);
assert.deepEqual(JSON.parse(engine.drainEvents()), [
  { type: 'graph-ready', revision: 1 },
  { type: 'state', state: 'ready' },
]);
engine.runFrame(
  1,
  1 / 60,
  1n,
  new Float32Array([2026, 7, 29, 0]),
  new Float32Array(4),
  new Float32Array([640, 360, 1]),
);
assert.equal(engine.engineState(), 'running');
assert.equal(engine.lastFrame, 1n);
assert.equal(engine.pendingCommandCount, 0);
assert.deepEqual(JSON.parse(engine.drainEvents()), [
  { type: 'state', state: 'running' },
  {
    type: 'frame-planned',
    frame: 1,
    revision: 1,
    commandCount: 0,
    dirtyNodeCount: 0,
  },
]);
engine.pause();
assert.equal(engine.engineState(), 'paused');
engine.resume();
engine.stop();
assert.equal(engine.engineState(), 'stopped');
engine.dispose();
engine.free();

console.log('[sdk-smoke] loaded WASM API v1 and completed typed Engine frame lifecycle');
