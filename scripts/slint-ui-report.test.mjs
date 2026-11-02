import assert from 'node:assert/strict';
import { test } from 'node:test';
import { mkdtemp, writeFile, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { UI_CASES, reportModel, renderHtml, regenerateReport } from './slint-ui-report.mjs';

const startedAt = '2026-10-31T00:00:00.000Z';
const finishedAt = '2026-10-31T00:00:01.000Z';

test('complete success lists every case and only its own screenshots', () => {
  const steps = UI_CASES.map((entry) => ({ name: entry.id, passed: true, startedAt, finishedAt }));
  const model = reportModel({ passed: true, startedAt, finishedAt }, {
    steps, screenshots: ['C:\\artifacts\\11-playing-blue-preview.png'],
  });
  assert.equal(model.passed, true);
  assert.deepEqual(model.counts, { passed: 17, failed: 0, incomplete: 0, not_run: 0 });
  assert.equal(model.cases[13].screenshots.length, 1);
  assert.equal(model.cases[0].screenshots.length, 0);
  assert.match(renderHtml(model), /href="11-playing-blue-preview.png"/);
});

test('failure preserves prior passes, marks later cases not run, and escapes errors', () => {
  const error = '<script>alert("unsafe")</script> & invalid source';
  const model = reportModel({ passed: false, error }, { steps: [
    { name: UI_CASES[0].id, passed: true },
    { name: UI_CASES[1].id, passed: false, error },
  ] });
  assert.deepEqual(model.counts, { passed: 1, failed: 1, incomplete: 0, not_run: 15 });
  const html = renderHtml(model, ['failure.png', 'application-1.log']);
  assert.equal(html.includes(error), false);
  assert.match(html, /&lt;script&gt;alert\(&quot;unsafe&quot;\)&lt;\/script&gt;/);
  assert.match(html, /data-status="not_run"/);
  assert.match(html, /href="failure.png"/);
  assert.match(html, /href="application-1.log"/);
});

test('startup failure and interruption cannot become successful or skipped passes', () => {
  const startup = reportModel({ passed: false, error: 'Application crashed' });
  assert.equal(startup.counts.not_run, 17);
  const aborted = reportModel({ passed: false }, { steps: [{ name: UI_CASES[0].id, startedAt }] });
  assert.equal(aborted.counts.incomplete, 1);
  assert.equal(aborted.counts.not_run, 16);
  assert.equal(aborted.passed, false);
  assert.equal(reportModel({ passed: true }, { steps: [] }).passed, false);
});

test('explicit step screenshot ownership overrides legacy prefix matching', () => {
  const model = reportModel({ passed: false }, {
    steps: [{ name: UI_CASES[0].id, passed: true, screenshots: [] }],
    screenshots: ['/tmp/01-old-screenshot.png'],
  });
  assert.deepEqual(model.cases[0].screenshots, []);
});

test('regeneration rejects stale scenario results after reusing a run directory', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'oq-ui-report-'));
  try {
    await writeFile(join(directory, 'run.json'), JSON.stringify({ passed: false, startedAt: finishedAt, error: 'startup failed' }));
    await writeFile(join(directory, 'result.json'), JSON.stringify({ passed: true, startedAt, steps: UI_CASES.map((entry) => ({ name: entry.id, passed: true })) }));
    const { path, model } = await regenerateReport(directory);
    assert.equal(model.counts.passed, 0);
    assert.equal(model.counts.not_run, 17);
    assert.match(await readFile(path, 'utf8'), /startup failed/);
  } finally { await rm(directory, { recursive: true, force: true }); }
});

test('a newer failed result supersedes an older embedded successful run', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'oq-ui-report-'));
  try {
    await writeFile(join(directory, 'run.json'), JSON.stringify({ passed: true, startedAt, finishedAt,
      scenarios: { steps: UI_CASES.map((entry) => ({ name: entry.id, passed: true })) } }));
    await writeFile(join(directory, 'result.json'), JSON.stringify({ passed: false, startedAt: '2026-10-31T00:00:02.000Z', error: 'new failure',
      steps: [{ name: UI_CASES[0].id, passed: false }] }));
    await writeFile(join(directory, 'failure.png'), 'old screenshot');
    await writeFile(join(directory, 'application-9.log'), 'old log');
    const { path, model } = await regenerateReport(directory);
    assert.equal(model.passed, false);
    assert.equal(model.counts.failed, 1);
    assert.equal(model.counts.not_run, 16);
    const html = await readFile(path, 'utf8');
    assert.match(html, /new failure/);
    assert.equal(html.includes('src="failure.png"'), false);
    assert.equal(html.includes('href="application-9.log"'), false);
  } finally { await rm(directory, { recursive: true, force: true }); }
});

test('shader suite uses its own complete plan and preserves failed and unexecuted entries', () => {
  const caseDefinitions = ['Resample', 'Invert', 'Noise'].map((label, index) => ({ id: `shader-${index}`, title: label, description: `验证 ${label}`, screenshotPrefixes: [] }));
  const model = reportModel({ passed: false }, { caseDefinitions, steps: [
    { name: 'shader-0', passed: true, details: { pixels: { verified: true, oracle: 'CPU pixel reference', matchedPixels: 4096, totalPixels: 4096, meanError: 0.2 } } },
    { name: 'shader-1', passed: false, error: 'pixel mismatch' },
  ] });
  assert.deepEqual(model.cases.map((entry) => entry.title), ['Resample', 'Invert', 'Noise']);
  assert.deepEqual(model.counts, { passed: 1, failed: 1, incomplete: 0, not_run: 1 });
  assert.match(renderHtml(model), /CPU pixel reference/);
  assert.equal(renderHtml(model).includes('默认空白启动'), false);
});
