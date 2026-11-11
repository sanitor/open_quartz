import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { join, resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');
function run(args) {
  return new Promise((accept, reject) => {
    const child = spawn('cargo', args, { cwd: root, env: process.env, stdio: 'inherit' });
    child.once('error', reject);
    child.once('close', (code, signal) => code === 0 ? accept() : reject(new Error(`Performance command failed: ${signal || code}`)));
  });
}

const output = resolve(process.env.OPEN_QUARTZ_INFERENCE_E2E_OUTPUT || 'test-results/inference-e2e');
const directory = join(output, 'performance');
await mkdir(directory, { recursive: true });
const report = { passed: false, startedAt: new Date().toISOString(), cases: [],
  method: 'Serial completion wall latency; independent processes, one artifact per catalog model. FP32 original graphs, GPU gradient, 3 warmups and 30 samples/path; parity outside measurement. No UI FPS or GPU timestamp claim; no wall-time threshold on shared runners.' };
try {
  assert.equal(process.platform, 'darwin', 'Performance regression requires hardware Metal on macOS');
  const manifest = JSON.parse(await readFile(process.env.OPEN_QUARTZ_INFERENCE_E2E_MANIFEST || join(output, 'manifest.json'), 'utf8'));
  assert.equal(manifest.schemaVersion, 1);
  assert.equal(manifest.models.length, manifest.catalog.length, 'Every catalog model needs its pinned artifact');
  for (const descriptor of manifest.catalog) {
    const model = manifest.models.find((entry) => entry.id === descriptor.id);
    assert.ok(model, `Missing ${descriptor.id}`);
    assert.equal(createHash('sha256').update(await readFile(model.path)).digest('hex'), model.sha256, `${descriptor.id} model bytes changed`);
    const path = join(directory, `${descriptor.id}.json`);
    const args = ['run', '--offline', '--locked', '--release', '-p', 'open_quartz', '--features', 'native-runtime,native-onnx', '--example'];
    if (descriptor.id === 'super-resolution-3x') {
      args.push('resident_benchmark', '--', model.path, path);
    } else {
      args.push('catalog_benchmark', '--', descriptor.id, model.path, path, '--width', '1280', '--height', '720');
    }
    args.push('--warmup', '3', '--samples', '30');
    const entry = { modelId: descriptor.id, modelSha256: model.sha256, report: path, passed: false };
    report.cases.push(entry);
    await run(args);
    const result = JSON.parse(await readFile(path, 'utf8'));
    if (descriptor.id === 'super-resolution-3x') {
      assert.equal(result.residentHostImageBytes, 0);
      assert.ok(result.cases.every((item) => item.residentHostImageBytes === 0));
    } else {
      assert.equal(result.status, 'passed');
      assert.equal(result.parity.passed, true);
      assert.equal(result.sampleCounterDelta.hostImageBytes, 0);
    }
    entry.passed = true;
  }
  assert.equal(report.cases.length, manifest.catalog.length);
  report.passed = true;
} catch (error) {
  report.error = error.stack || String(error);
  process.exitCode = 1;
  console.error(error);
} finally {
  report.finishedAt = new Date().toISOString();
  await writeFile(join(directory, 'summary.json'), JSON.stringify(report, null, 2));
}
