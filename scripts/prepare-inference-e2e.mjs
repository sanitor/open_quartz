import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { createReadStream, createWriteStream } from 'node:fs';
import { copyFile, mkdir, readFile, rename, rm, stat, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { Readable, Transform } from 'node:stream';
import { pipeline } from 'node:stream/promises';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const fixtureDirectory = join(root, 'tests', 'fixtures', 'inference');
const defaultOutput = join(root, 'test-results', 'inference-e2e');

function catalogExport(signal) {
  return new Promise((accept, reject) => {
    const child = spawn('cargo', ['run', '--offline', '--locked', '-p', 'open_quartz', '--example', 'inference_e2e_catalog'], {
      cwd: root, stdio: ['ignore', 'pipe', 'inherit'], signal,
    });
    const chunks = [];
    child.stdout.on('data', (chunk) => chunks.push(chunk));
    child.once('error', reject);
    child.once('close', (code, terminationSignal) => {
      if (code !== 0) return reject(new Error(`Canonical Rust catalog export failed: ${terminationSignal || code}`));
      try { accept(JSON.parse(Buffer.concat(chunks).toString('utf8'))); } catch (error) { reject(error); }
    });
  });
}

function validatePin(pin, label) {
  assert.match(pin.sha256, /^[a-f0-9]{64}$/, `${label}: a full SHA-256 pin is required`);
  assert.ok(Number.isSafeInteger(pin.byteLength) && pin.byteLength > 0, `${label}: a positive byte length is required`);
}

async function exists(path) {
  try { return (await stat(path)).isFile(); } catch (error) {
    if (error.code === 'ENOENT') return false;
    throw error;
  }
}

async function verify(path, pin, signal) {
  signal?.throwIfAborted();
  const hash = createHash('sha256');
  let byteLength = 0;
  for await (const chunk of createReadStream(path, { signal })) {
    hash.update(chunk);
    byteLength += chunk.length;
  }
  assert.equal(byteLength, pin.byteLength, `${path}: byte length mismatch`);
  assert.equal(hash.digest('hex'), pin.sha256, `${path}: SHA-256 mismatch`);
}

// Never mutate the application's cache, or expose partially copied/downloaded
// files. The published manifest refers only to independently verified snapshots.
async function publishFile(destination, pin, produce, signal) {
  if (await exists(destination)) {
    await verify(destination, pin, signal);
    return destination;
  }
  const temporary = `${destination}.${process.pid}.${randomUUID()}.part`;
  try {
    await produce(temporary);
    await verify(temporary, pin, signal);
    try { await rename(temporary, destination); } catch (error) {
      if (!['EEXIST', 'EPERM'].includes(error.code) || !(await exists(destination))) throw error;
      await verify(destination, pin, signal);
    }
    return destination;
  } finally {
    await rm(temporary, { force: true });
  }
}

function applicationModelDirectory() {
  const home = homedir();
  const dataDirectory = process.platform === 'darwin' ? join(home, 'Library', 'Application Support')
    : process.platform === 'win32' ? process.env.APPDATA || join(home, 'AppData', 'Roaming')
      : process.env.XDG_DATA_HOME || join(home, '.local', 'share');
  return join(dataDirectory, 'com.openquartz.app', 'models');
}

async function prepareModel(entry, pin, directory, { signal, offline }) {
  validatePin(pin, entry.id);
  assert.equal(entry.downloadUrl, pin.catalogUrl, `${entry.id}: catalog URL changed; review and repin the original artifact`);
  assert.equal(entry.fileSize, pin.byteLength, `${entry.id}: catalog artifact size changed`);
  if (entry.sha256) assert.equal(entry.sha256, pin.sha256, `${entry.id}: catalog digest differs from asset lock`);
  assert.equal(new URL(pin.url).protocol, 'https:', `${entry.id}: pinned download must use HTTPS`);
  const destination = join(directory, `${entry.id}.onnx`);
  return publishFile(destination, pin, async (temporary) => {
    for (const cache of [applicationModelDirectory(), '/tmp/open-quartz-all-models']) {
      const candidate = join(cache, `${entry.id}.onnx`);
      if (!(await exists(candidate))) continue;
      await verify(candidate, pin, signal);
      await copyFile(candidate, temporary);
      return;
    }
    assert.ok(!offline, `${entry.id}: no hash-verified cached model exists (offline mode)`);
    const timeout = AbortSignal.timeout(600_000);
    const response = await fetch(pin.url, { signal: signal ? AbortSignal.any([signal, timeout]) : timeout });
    assert.ok(response.ok && response.body, `${entry.id}: download failed (${response.status}): ${pin.url}`);
    // Bound the transfer by the pinned artifact size, regardless of HTTP framing.
    let received = 0;
    const bound = new Transform({ transform(chunk, _encoding, callback) {
      received += chunk.length;
      callback(received > pin.byteLength ? new Error(`${entry.id}: download exceeds pinned byte length`) : null, chunk);
    } });
    await pipeline(Readable.fromWeb(response.body), bound, createWriteStream(temporary, { flags: 'wx' }), { signal });
  }, signal);
}

/** Prepare once, then run every production/UI test without network access. */
export async function prepareInferenceE2e(outputDir = defaultOutput, { signal, offline = false } = {}) {
  signal?.throwIfAborted();
  const output = resolve(outputDir);
  const lock = JSON.parse(await readFile(join(fixtureDirectory, 'assets.json'), 'utf8'));
  assert.equal(lock.schemaVersion, 1, 'Unsupported inference asset lock version');
  const catalog = await catalogExport(signal);
  assert.ok(Array.isArray(catalog) && catalog.length > 0, 'Canonical catalog must be a nonempty array');
  const ids = catalog.map((entry) => entry.id);
  assert.equal(new Set(ids).size, ids.length, 'Canonical catalog IDs must be unique');
  assert.deepEqual([...ids].sort(), Object.keys(lock.models).sort(), 'Every canonical model must have exactly one asset pin');
  for (const id of ids) assert.match(id, /^[A-Za-z0-9_-]+$/, 'Unsafe canonical model ID');
  await mkdir(join(output, 'models'), { recursive: true });
  await mkdir(join(output, 'fixtures'), { recursive: true });
  const models = [];
  for (const entry of catalog) {
    const pin = lock.models[entry.id];
    const path = await prepareModel(entry, pin, join(output, 'models'), { signal, offline });
    models.push({ id: entry.id, path, sha256: pin.sha256 });
  }
  // The original natural photograph and its attribution are checked as well as
  // the committed FFmpeg derivatives. Codec versions cannot silently change the
  // test corpus: regeneration is an intentional asset-lock update, not a test.
  for (const source of lock.sources) {
    validatePin(source, source.file);
    await verify(join(fixtureDirectory, source.file), source, signal);
  }
  const fixtures = {};
  for (const name of ['image', 'alternateImage', 'constantImage', 'video']) {
    const pin = lock.fixtures[name];
    validatePin(pin, name);
    assert.ok(Number.isSafeInteger(pin.width) && pin.width > 0 && Number.isSafeInteger(pin.height) && pin.height > 0,
      `${name}: invalid fixture dimensions`);
    const source = join(fixtureDirectory, pin.file);
    await verify(source, pin, signal);
    const path = await publishFile(join(output, 'fixtures', pin.file), pin, (temporary) => copyFile(source, temporary), signal);
    fixtures[name] = { path, width: pin.width, height: pin.height, sha256: pin.sha256 };
    if (name === 'video') {
      assert.ok(Number.isSafeInteger(pin.frameCount) && pin.frameCount > 1, 'Video must contain multiple pinned frames');
      Object.assign(fixtures[name], { frameCount: pin.frameCount, frameRate: pin.frameRate, frameGroups: pin.frameGroups });
    }
  }
  assert.equal(fixtures.image.width, fixtures.alternateImage.width, 'Natural fixtures must share width');
  assert.equal(fixtures.image.height, fixtures.alternateImage.height, 'Natural fixtures must share height');
  const manifest = { schemaVersion: 1, catalog, models, fixtures };
  const manifestPath = join(output, 'manifest.json');
  const temporary = `${manifestPath}.${process.pid}.${randomUUID()}.part`;
  try {
    await writeFile(temporary, `${JSON.stringify(manifest, null, 2)}\n`, { flag: 'wx', signal });
    await rename(temporary, manifestPath);
  } finally { await rm(temporary, { force: true }); }
  return manifestPath;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  let outputDir = defaultOutput;
  let offline = false;
  const args = process.argv.slice(2);
  for (let index = 0; index < args.length; index++) {
    if (args[index] === '--output-dir') {
      assert.ok(args[index + 1] && !args[index + 1].startsWith('--'), '--output-dir requires a path');
      outputDir = resolve(args[++index]);
    } else if (args[index] === '--offline') offline = true;
    else throw new Error(`Unknown preparation option: ${args[index]}`);
  }
  console.log(await prepareInferenceE2e(outputDir, { offline }));
}
