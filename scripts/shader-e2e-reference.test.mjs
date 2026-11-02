import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { prepareShaderFixtures, readShaderCatalog } from './shader-e2e-fixtures.mjs';
import { REFERENCE_LABELS, compareShaderOutput, makeReference, makeFeedbackReference } from './shader-e2e-reference.mjs';

test('shader fixtures cover exactly every canonical catalog shader', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'open-quartz-shader-fixtures-'));
  try {
    const catalog = readShaderCatalog();
    const cases = await prepareShaderFixtures(directory);
    assert.equal(cases.length, 34);
    assert.deepEqual(cases.map((entry) => entry.id), catalog.map((entry) => entry.id));
    assert.equal(new Set(cases.map((entry) => entry.id)).size, cases.length);
    assert.deepEqual(cases.map((entry) => entry.label), REFERENCE_LABELS);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('reference rejects a catalog label without an independent oracle', () => {
  assert.throws(() => makeReference('__new_catalog_shader__', [], {}), /Unknown shader reference/);
});

test('independent CPU references compute static shader formulas', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'open-quartz-shader-reference-'));
  try {
    const cases = await prepareShaderFixtures(directory);
    const byLabel = new Map(cases.map((entry) => [entry.label, entry]));
    const pixel = (entry, index = 0) => entry.expected.pixels.subarray(index * 4, index * 4 + 4);
    assert.deepEqual([...pixel(byLabel.get('Invert'), 0)], [255 - 20, 255 - 25, 255 - 35, 255]);
    const grayInput = [20, 25, 35];
    const gray = Math.round(grayInput[0] * 0.299 + grayInput[1] * 0.587 + grayInput[2] * 0.114);
    assert.deepEqual([...pixel(byLabel.get('Grayscale'), 0)], [gray, gray, gray, 255]);
    const sepia = [Math.min(255, Math.round(20 * 0.393 + 25 * 0.769 + 35 * 0.189)), Math.min(255, Math.round(20 * 0.349 + 25 * 0.686 + 35 * 0.168)), Math.min(255, Math.round(20 * 0.272 + 25 * 0.534 + 35 * 0.131)), 255];
    assert.deepEqual([...pixel(byLabel.get('Sepia'), 0)], sepia);
    assert.deepEqual([...pixel(byLabel.get('Add'), 0)], [23, 33, 51, 255]);
    assert.deepEqual([...pixel(byLabel.get('Multiply'), 0)], [0, 1, 2, 255]);
    assert.notDeepEqual([...pixel(byLabel.get('Displacement'), 0)], [...pixel(byLabel.get('Resample'), 0)], 'displacement must use a distinct map and source');
    const noise = byLabel.get('Noise').expected.pixels;
    assert.ok(new Set(noise.filter((_, index) => index % 4 === 0)).size > 16, 'noise reference must not be constant');
    const feedback = byLabel.get('Gray-Scott Reaction-Diffusion');
    assert.equal(feedback.oracle.kind, 'feedback-dynamic');
    assert.equal(feedback.oracle.seedFrame, 0);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('pixel comparison rejects unchanged input and an all-black output for Invert', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'open-quartz-shader-negative-'));
  try {
    const cases = await prepareShaderFixtures(directory);
    const invert = cases.find((entry) => entry.label === 'Invert');
    const identity = cases.find((entry) => entry.label === 'Resample').expected;
    assert.equal(compareShaderOutput(invert, identity).verified, false);
    const black = { width: 64, height: 64, pixels: Buffer.alloc(64 * 64 * 4, 0) };
    for (let offset = 3; offset < black.pixels.length; offset += 4) black.pixels[offset] = 255;
    assert.equal(compareShaderOutput(invert, black).verified, false);
    assert.equal(compareShaderOutput(cases.find((entry) => entry.label === 'Noise'), black).verified, false);
  } finally { await rm(directory, { recursive: true, force: true }); }
});


test('noise oracle rejects the wrong frequency and spatially shuffled gray noise', () => {
  const expected = makeReference('Noise', [], { scale: 13.7 });
  const entry = { expected, oracle: expected.oracle };
  assert.equal(compareShaderOutput(entry, makeReference('Noise', [], { scale: 1 })).verified, false);
  const shuffled = { width: 64, height: 64, pixels: Buffer.alloc(64 * 64 * 4) };
  for (let index = 0; index < 4096; index++) {
    const source = (index * 1543) % 4096;
    expected.pixels.copy(shuffled.pixels, index * 4, source * 4, source * 4 + 4);
  }
  assert.equal(compareShaderOutput(entry, shuffled).verified, false);
});

test('feedback quantizes the initial seed and every diffusion step to RGBA8', () => {
  const first = makeFeedbackReference({}, 1);
  const pixel = (x, y) => [...first.pixels.subarray((y * 64 + x) * 4, (y * 64 + x) * 4 + 4)];
  assert.deepEqual(pixel(32, 32), [127, 65, 0, 255]);
  assert.deepEqual(pixel(28, 32), [154, 58, 0, 255]);
  assert.deepEqual(pixel(27, 32), [228, 7, 0, 255]);
});
