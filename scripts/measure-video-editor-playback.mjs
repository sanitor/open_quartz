#!/usr/bin/env node
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { createServer } from 'node:net';
import { setTimeout as delay } from 'node:timers/promises';
import { performance } from 'node:perf_hooks';
import { SlintUiClient } from './slint-ui-client.mjs';

// Measures the actual native monitor, not playhead motion or worker submission.
// The project must already reference real media; no rendering mocks or proxy edits.
const [binaryArg, projectArg, outputArg, secondsArg = '6'] = process.argv.slice(2);
assert(binaryArg && projectArg && outputArg, 'Usage: node scripts/measure-video-editor-playback.mjs <ui-automation-binary> <project.oqvideo.json> <output-dir> [seconds]');
const seconds = Number(secondsArg);
assert(Number.isFinite(seconds) && seconds >= 1 && seconds <= 120, 'Measure 1–120 seconds');
const binary = resolve(binaryArg), project = resolve(projectArg), output = resolve(outputArg);
const file = JSON.parse(await readFile(project, 'utf8'));
await mkdir(output, { recursive: true });
const socket = createServer();
await new Promise(accept => socket.listen(0, '127.0.0.1', accept));
const port = socket.address().port;
await new Promise(accept => socket.close(accept));
const child = spawn(binary, ['--project', project], {
  env: { ...process.env, SLINT_MCP_PORT: String(port), OPEN_QUARTZ_UI_CLOSE_STDIN: '1' },
  stdio: ['pipe', 'pipe', 'pipe'],
});
let stdout = '', stderr = '', exit;
child.stdout.on('data', data => stdout += data);
child.stderr.on('data', data => stderr += data);
const exited = new Promise((accept, reject) => {
  child.once('error', reject);
  child.once('close', (code, signal) => { exit = { code, signal }; accept(exit); });
});
const client = new SlintUiClient(`http://127.0.0.1:${port}/mcp`, { timeoutMs: 30_000 });
const launchedAt = performance.now();
const report = {
  binary, project, requestedSeconds: seconds,
  sequence: { width: file.data.width, height: file.data.height, fps: file.data.fps },
  assets: file.data.assets.map(asset => ({ name: asset.name, width: asset.width, height: asset.height, fps: asset.fps, path: asset.path })),
  correctors: (file.data.tracks?.flatMap(track => track.clips) ?? file.data.clips).flatMap(clip => clip.pipeline.graph.nodes.filter(node => node.type === 'colorcorrector').map(node => ({ clip: clip.id, config: node.data.colorCorrector }))),
  measurement: 'Slint native monitor Frame status changes sampled over HTTP; sampled counts are a lower bound, not GPU timestamps or a universal FPS guarantee. Playback metrics, when available, distinguish worker publication from UI display.',
  samples: [],
};
const state = async () => (await client.properties(client.rootHandle)).accessibleDescription ?? '';
const button = label => client.find({ label, role: 'Button' });
function frameOf(status) {
  const frame = /(?:^|;\s*)Frame (\d+)\s*· source PTS/.exec(status);
  return frame ? Number(frame[1]) : undefined;
}
function percentile(values, q) {
  if (!values.length) return null;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * q))];
}
try {
  await client.waitFor(async () => {
    if (exit) throw Error(`Native process exited: ${JSON.stringify(exit)}\n${stderr}`);
    try { await client.init(); await client.useWindow(); return true; } catch { return false; }
  }, { description: 'real playback measurement window' });
  await client.waitFor(async () => {
    const status = await state();
    if (/failed|exceeds|capacity|unsupported|cannot/i.test(status)) throw Error(status);
    return frameOf(status) !== undefined;
  }, { timeoutMs: 60_000, description: 'real initial GPU frame' });
  report.initialFrameWaitMs = performance.now() - launchedAt;
  await client.screenshot(join(output, 'initial.png'));
  const metricElements = await client.byLabel(/^Playback metrics /, { role: 'Text', visible: false });
  const metricElement = metricElements[0];
  const readMetrics = async () => {
    if (!metricElement) return undefined;
    const label = (await client.properties(metricElement)).accessibleLabel;
    return JSON.parse(label.slice('Playback metrics '.length));
  };
  report.metrics = [];
  let nextMetricMs = 0;
  await client.click(await button('PLAY'));
  const started = performance.now();
  while (performance.now() - started < seconds * 1000) {
    report.samples.push({ ms: performance.now() - started, status: await state() });
    if (performance.now() - started >= nextMetricMs) {
      const metrics = await readMetrics();
      if (metrics) report.metrics.push({ ms: performance.now() - started, ...metrics });
      nextMetricMs += 100;
    }
    await delay(8);
  }
  const firstRunning = report.metrics.find(metrics => metrics.playing && !metrics.preparing);
  report.sampledPrefillMs = firstRunning?.ms ?? null;
  report.finalMetrics = await readMetrics();
  const pause = await client.byLabel('PAUSE', { role: 'Button' });
  if (pause.length) await client.click(pause[0]);
  report.finalStatus = await state();
  await client.screenshot(join(output, 'played.png'));
  const changes = report.samples.filter((sample, index, all) => frameOf(sample.status) !== undefined
    && (index === 0 || frameOf(sample.status) !== frameOf(all[index - 1].status)));
  const intervals = changes.slice(1).map((sample, index) => sample.ms - changes[index].ms);
  const trailingMs = changes.length ? report.samples.at(-1).ms - changes.at(-1).ms : seconds * 1000;
  report.summary = {
    sampleCount: report.samples.length, observedMonitorChanges: changes.length,
    firstFrame: changes.length ? frameOf(changes[0].status) : null,
    lastFrame: changes.length ? frameOf(changes.at(-1).status) : null,
    sampledIntervalP50Ms: percentile(intervals, 0.5), sampledIntervalP95Ms: percentile(intervals, 0.95),
    longestUnchangedMs: Math.max(0, trailingMs, ...intervals), trailingUnchangedMs: trailingMs,
  };
  assert(changes.length > 1, 'Playback clock ran without multiple actual monitor deliveries');
  if (report.finalMetrics?.displayed !== undefined) {
    const metrics = report.finalMetrics;
    assert.equal(metrics.displayed_width, file.data.width, 'Monitor must receive full sequence width');
    assert.equal(metrics.displayed_height, file.data.height, 'Monitor must receive full sequence height');
    assert.equal(metrics.cpu_copy_bytes, 0, 'Normal playback must not copy CPU image pixels');
    assert(metrics.bytes_peak <= metrics.byte_limit, 'Physical frame budget was exceeded');
    assert(metrics.queue_peak <= metrics.window_high_water, 'Admission window exceeded its bound');
    assert(metrics.displayed > 1, 'Submission or worker publication is not proof of native image delivery');
  }
  report.passed = true;
} catch (error) {
  report.passed = false;
  report.error = error.stack;
  try { report.finalStatus = await state(); await client.screenshot(join(output, 'failure.png')); } catch {}
} finally {
  try {
    if (!exit) await client.click(await button('Close window'));
    const result = await Promise.race([exited, delay(10_000).then(() => null)]);
    if (!result) { child.kill('SIGTERM'); report.forcedShutdown = true; await exited; }
    report.exit = exit;
  } catch {
    if (!exit) { child.kill('SIGTERM'); report.forcedShutdown = true; await exited; }
  }
  await writeFile(join(output, 'application.log'), stdout + stderr);
  await writeFile(join(output, 'measurement.json'), JSON.stringify(report, null, 2));
}
console.log(JSON.stringify({ passed: report.passed, ...report.summary, finalMetrics: report.finalMetrics, error: report.error, finalStatus: report.finalStatus, output }, null, 2));
if (!report.passed || report.forcedShutdown || report.exit?.code !== 0) process.exitCode = 1;
