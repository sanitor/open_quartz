#!/usr/bin/env node
import { spawn } from 'node:child_process';
import { createWriteStream } from 'node:fs';
import { mkdir, writeFile } from 'node:fs/promises';
import { createServer } from 'node:net';
import { resolve, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import { runScenarios } from './slint-ui-scenarios.mjs';
import { toolValue } from './slint-ui-client.mjs';
import { writeHtmlReport, consoleSummary } from './slint-ui-report.mjs';

async function availablePort() {
  const server = createServer();
  await new Promise((accept, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', accept);
  });
  const port = server.address().port;
  await new Promise((accept, reject) => server.close((error) => error ? reject(error) : accept()));
  return port;
}

export async function runUiTests(binary, { outputDir = process.env.SLINT_UI_OUTPUT_DIR, suite = 'interaction' } = {}) {
  if (!['interaction', 'shaders'].includes(suite)) throw new Error(`Unknown UI suite: ${suite}`);
  const directory = resolve(outputDir || join('test-results', 'slint-ui', `${suite}-${new Date().toISOString().replaceAll(/[:.]/g, '-')}`));
  await mkdir(directory, { recursive: true });
  const projectPath = join(directory, 'ui-roundtrip.quartz.json');
  await writeFile(projectPath, JSON.stringify({
    version: '0.4.0', name: 'UI automation', createdAt: '', updatedAt: '',
    graph: { nodes: [], edges: [] },
  }, null, 2));
  const port = await availablePort();
  const endpoint = `http://127.0.0.1:${port}/mcp`;
  let current;
  let stopping;
  let launchNumber = 0;
  const controller = new AbortController();
  const { signal } = controller;
  const lifecycleFailure = new Promise((accept, reject) => {
    signal.addEventListener('abort', () => reject(signal.reason), { once: true });
  });
  lifecycleFailure.catch(() => {});
  const launches = [];
  const timeoutMs = Number(process.env.SLINT_UI_TIMEOUT_MS || (suite === 'shaders' ? 600_000 : 180_000));
  if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) throw new Error('SLINT_UI_TIMEOUT_MS must be positive');

  function stop() {
    if (stopping) return stopping;
    if (!current) return Promise.resolve();
    const running = current;
    current = undefined;
    stopping = (async () => {
      try {
        if (running.child.exitCode === null && running.child.signalCode === null) running.child.kill('SIGTERM');
        const force = setTimeout(() => running.child.kill('SIGKILL'), 5000);
        try { await running.exited; } finally { clearTimeout(force); }
        await new Promise((accept) => running.log.end(accept));
      } finally { stopping = undefined; }
    })();
    return stopping;
  }

  function fail(error) {
    controller.abort(error);
    void stop().catch(() => {});
  }

  async function restart({ project } = {}) {
    await stop();
    signal.throwIfAborted();
    launchNumber += 1;
    const args = project ? ['--project', project] : [];
    const logPath = join(directory, `application-${launchNumber}.log`);
    const log = createWriteStream(logPath);
    log.on('error', (error) => fail(new Error(`Cannot write ${logPath}: ${error.message}`)));
    const child = spawn(resolve(binary), args, {
      env: { ...process.env, SLINT_MCP_PORT: String(port) },
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    child.stdout.pipe(log, { end: false });
    child.stderr.pipe(log, { end: false });
    let exit;
    const exited = new Promise((accept) => {
      child.once('error', (error) => { exit = { error: error.message }; });
      child.once('exit', (code, childSignal) => { exit = { code, signal: childSignal }; });
      child.once('close', () => {
        accept(exit);
        if (current?.child === child) fail(new Error(`Application exited unexpectedly: ${JSON.stringify(exit)}. See ${logPath}`));
      });
    });
    current = { child, exited, log };
    launches.push({ args, logPath });
    const deadline = Date.now() + 30_000;
    let lastError;
    while (Date.now() < deadline && !signal.aborted) {
      if (exit) throw new Error(`Application exited during startup: ${JSON.stringify(exit)}. See ${logPath}`);
      try {
        const response = await fetch(endpoint, {
          method: 'POST', headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name: 'list_windows', arguments: {} } }),
          signal: AbortSignal.any([signal, AbortSignal.timeout(2000)]),
        });
        const body = await response.json();
        if (response.ok && body.result && !body.result.isError && toolValue(body.result).windowHandles?.length > 0) return endpoint;
        lastError = new Error(JSON.stringify(body));
      } catch (error) { lastError = error; }
      signal.throwIfAborted();
      await delay(150, undefined, { signal });
    }
    throw new Error(`Slint MCP did not become ready: ${lastError?.message || 'timeout'}. See ${logPath}`);
  }

  const timer = setTimeout(() => fail(new Error(`UI automation exceeded ${timeoutMs}ms`)), timeoutMs);
  const interrupt = (name) => {
    process.exitCode = name === 'SIGINT' ? 130 : 143;
    fail(new Error(`UI automation interrupted by ${name}`));
  };
  const onSigint = () => interrupt('SIGINT');
  const onSigterm = () => interrupt('SIGTERM');
  process.once('SIGINT', onSigint);
  process.once('SIGTERM', onSigterm);
  const startedAt = new Date().toISOString();
  let report;
  let scenarioReport;
  if (suite === 'shaders') {
    const catalog = JSON.parse(await (await import('node:fs/promises')).readFile(new URL('../crates/open_quartz/src/project/shader_catalog.json', import.meta.url), 'utf8'));
    scenarioReport = { suite, caseDefinitions: catalog.groups.flatMap((group) => group.items.map((item) => ({
      id: `shader-${item.label.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/-$/, '')}`,
      title: `${group.category} · ${item.label}`, description: '真实图片输入、原生连接和 GPU 预览验证', screenshotPrefixes: [],
    }))), steps: [], screenshots: [] };
  }
  try {
    const execute = suite === 'shaders' ? (await import('./slint-shader-scenarios.mjs')).runShaderScenarios : runScenarios;
    await Promise.race([restart(), lifecycleFailure]);
    const scenarios = await Promise.race([
      execute({ endpoint, outputDir: directory, projectPath, requireGpu: true, restart, signal,
        onProgress: (progress) => { scenarioReport = progress; } }),
      lifecycleFailure,
    ]);
    signal.throwIfAborted();
    if (current?.child.exitCode !== null || current?.child.signalCode !== null) throw new Error('Application exited unexpectedly');
    report = { passed: true, suite, startedAt, finishedAt: new Date().toISOString(), launches, scenarios };
    console.log(`[slint-ui] PASS\n[slint-ui] Screenshots and report: ${directory}`);
    return report;
  } catch (error) {
    report = { passed: false, suite, startedAt, finishedAt: new Date().toISOString(), launches,
      scenarios: error.scenarioResult || scenarioReport, error: error.stack || String(error) };
    console.error(`[slint-ui] FAIL: ${error.message}\n[slint-ui] Diagnostics: ${directory}`);
    throw error;
  } finally {
    clearTimeout(timer);
    process.removeListener('SIGINT', onSigint);
    process.removeListener('SIGTERM', onSigterm);
    await stop();
    await writeFile(join(directory, 'run.json'), JSON.stringify(report, null, 2));
    const { path, model } = await writeHtmlReport(directory, report);
    console.log(`${consoleSummary(model)}\n[slint-ui] HTML report: ${path}`);
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [binary, ...args] = process.argv.slice(2);
  const suite = args.includes('--shaders') ? 'shaders' : 'interaction';
  const remaining = args.filter((arg) => arg !== '--shaders');
  if (!binary || remaining.length > 1 || remaining.some((arg) => arg.startsWith('--'))) throw new Error('Usage: node scripts/slint-ui-test.mjs <automation-enabled-binary> [output-directory] [--shaders]');
  try { await runUiTests(binary, { outputDir: remaining[0], suite }); }
  catch (error) { console.error(error.stack || String(error)); process.exitCode ||= 1; }
}
