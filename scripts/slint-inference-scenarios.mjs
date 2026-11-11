import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { copyFile, mkdir, readFile, stat, writeFile } from 'node:fs/promises';
import { basename, dirname, isAbsolute, join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { createClient, saveJson } from './slint-ui-client.mjs';
import { decodePng, parseStatus } from './slint-ui-scenarios.mjs';
import { encodePng } from './shader-e2e-reference.mjs';

const errorText = (error) => error?.stack || String(error);
const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex');
const escapeRegex = (value) => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
const center = (element) => ({ x: (element.absolutePosition?.x ?? 0) + element.size.width / 2, y: (element.absolutePosition?.y ?? 0) + element.size.height / 2 });
const port = (id, label, direction) => ({ id, label, direction, dataType: 'sampler2D' });
export let caseDefinitions = [];

function nodeState(element) {
  const state = parseStatus(element.accessibleDescription);
  assert.ok(state['Node id'], `Node identity missing: ${element.accessibleDescription}`);
  return { id: state['Node id'], label: element.accessibleLabel.slice('Select node '.length), ...state };
}

async function sourceNode(fixture, mode) {
  const data = {
    type: 'input', label: 'Inference fixture', shaderCode: '', uniforms: {}, inputs: [],
    outputs: [port('fixture_image', 'image', 'output')], inputMode: mode,
    inputDataType: 'sampler2D', texFilter: 'linear', texWrap: 'clamp',
  };
  if (mode === 'image') {
    const bytes = await readFile(fixture.path);
    const decoded = decodePng(bytes);
    assert.equal(decoded.width, fixture.width, 'Image fixture width differs from manifest');
    assert.equal(decoded.height, fixture.height, 'Image fixture height differs from manifest');
    Object.assign(data, { imageDataUrl: `data:image/png;base64,${bytes.toString('base64')}`, imageFileName: basename(fixture.path), imageWidth: fixture.width, imageHeight: fixture.height });
  } else {
    Object.assign(data, { videoSourceType: 'file', videoFilePath: fixture.path, videoLoop: true, videoPlaybackRate: 1 });
  }
  return { id: 'fixture_source', type: 'input', position: { x: -360, y: 40 }, data };
}

function rendererNode() {
  return { id: 'oracle_renderer', type: 'renderer', position: { x: 360, y: 40 }, data: {
    type: 'renderer', label: 'Oracle Renderer', shaderCode: '', uniforms: {},
    inputs: [port('oracle_input', 'inputTexture', 'input')], outputs: [],
  } };
}

async function writeProject(path, name, nodes, edges = []) {
  await saveJson(path, { version: '0.4.0', name, createdAt: '', updatedAt: '', graph: { nodes, edges } });
}

/** Sample the actual contain-fitted native Renderer image, not its status text.
 * A fixed normalized grid also works for ESR outputs larger than the preview.
 * The oracle is another normal Renderer screenshot of the actual production
 * PNG, so native texture sampling, display conversion and alpha-over-Theme.card
 * compositing are identical on both sides; RGBA is never compared to opaque RGB.
 */
function displayedPixels(png, preview, window, dimensions) {
  const image = decodePng(png);
  const scale = window.scaleFactor || 1;
  const sx = image.width / (window.size.width / scale);
  const sy = image.height / (window.size.height / scale);
  const fit = Math.min(preview.size.width / dimensions.width, preview.size.height / dimensions.height);
  assert.ok(fit > 0, 'Renderer preview has no image area');
  const left = preview.absolutePosition.x + (preview.size.width - dimensions.width * fit) / 2;
  const top = preview.absolutePosition.y + (preview.size.height - dimensions.height * fit) / 2;
  const roi = { left: left * sx, top: top * sy, width: dimensions.width * fit * sx, height: dimensions.height * fit * sy };
  assert.ok(roi.width >= 64 && roi.height >= 64, 'Renderer preview must display at least 64 physical pixels on each axis');
  const width = 96, height = 96;
  const pixels = Buffer.alloc(width * height * 4);
  for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
    const px = Math.floor(roi.left + (x + 0.5) * roi.width / width);
    const py = Math.floor(roi.top + (y + 0.5) * roi.height / height);
    assert.ok(px >= 0 && py >= 0 && px < image.width && py < image.height, 'Renderer pixels outside screenshot');
    const input = (py * image.width + px) * image.channels;
    const output = (y * width + x) * 4;
    for (let channel = 0; channel < 3; channel++) pixels[output + channel] = image.pixels[input + (image.channels >= 3 ? channel : 0)];
    pixels[output + 3] = 255;
  }
  return { width, height, pixels, roi, windowSize: window.size, scaleFactor: scale, sourceDimensions: dimensions };
}

function difference(actual, expected) {
  assert.equal(actual.pixels.length, expected.pixels.length);
  let max = 0, total = 0, changedPixels = 0;
  for (let offset = 0; offset < actual.pixels.length; offset += 4) {
    let pixelMax = 0;
    for (let channel = 0; channel < 3; channel++) {
      const delta = Math.abs(actual.pixels[offset + channel] - expected.pixels[offset + channel]);
      max = Math.max(max, delta); pixelMax = Math.max(pixelMax, delta); total += delta;
    }
    if (pixelMax >= 3) changedPixels++;
  }
  return { maxByteError: max, meanByteError: total / (actual.width * actual.height * 3), changedPixels, totalPixels: actual.width * actual.height, actualSha256: sha256(actual.pixels), expectedSha256: sha256(expected.pixels) };
}

function assertOracle(actual, expected) {
  // Both screenshots use the same Renderer inspector and contain-fit geometry.
  for (const key of ['width', 'height', 'left', 'top']) assert.ok(Math.abs(actual.roi[key] - expected.roi[key]) < 0.01, `Oracle/actual Renderer ${key} geometry differs`);
  const metrics = difference(actual, expected);
  assert.ok(metrics.maxByteError <= 3 && metrics.meanByteError <= 0.5, `Displayed Renderer differs from actual production output: ${JSON.stringify(metrics)}`);
  return metrics;
}

function assertDisplayedContent(image) {
  const low = [255, 255, 255], high = [0, 0, 0];
  const colors = new Set();
  for (let offset = 0; offset < image.pixels.length; offset += 4) {
    colors.add(image.pixels.readUIntBE(offset, 3));
    for (let channel = 0; channel < 3; channel++) {
      low[channel] = Math.min(low[channel], image.pixels[offset + channel]);
      high[channel] = Math.max(high[channel], image.pixels[offset + channel]);
    }
  }
  const channelRanges = high.map((value, channel) => value - low[channel]);
  assert.ok(colors.size >= 2 && Math.max(...channelRanges) >= 16, `Natural-image Renderer oracle displayed a flat/blank surface: ${JSON.stringify({ colors: colors.size, channelRanges })}`);
  return { uniqueDisplayedColors: colors.size, channelRanges };
}

/** Full catalog UI lane. Only input media are preloaded for tested graphs.
 * Every inference node and Renderer is created through its actual menu, all
 * tested graph edges are pointer-created, and Load/transport/Save are real UI.
 * Production execution must finish first; independent numeric status is gated separately.
 */
export async function runInferenceScenarios({ endpoint, outputDir, projectPath, restart, signal, onProgress, timeoutMs = 60_000 } = {}) {
  assert.ok(endpoint && outputDir, 'endpoint and outputDir are required');
  assert.equal(typeof restart, 'function', 'restart({project}) is required');
  const directory = resolve(outputDir);
  await mkdir(directory, { recursive: true });
  const report = {
    suite: 'inference', passed: false, endpoint, projectPath, startedAt: new Date().toISOString(),
    steps: [], screenshots: [], artifacts: [], caseDefinitions: [],
    interactions: {
      setup: 'Test graphs preload one input only through normal project loading/image decoding or native file-video resources; no input creation/file-picker coverage claimed',
      nodes: 'Every catalog Inference and Renderer created via real toolbar/category menus',
      connections: 'MCP pointer click_element; Sampler2d output selected by canonical expectedIO type, never positional first output',
      lifecycle: 'UI local model path + Load/Unload, PLAY/PAUSE/RESUME/STOP, FILE/SAVE and normal app project reopen',
      pixels: 'Actual native Renderer screenshot compared to actual production RGBA PNG displayed through ordinary source/Renderer; this proves display transport, not numerical correctness. Independent CPU parity is reported and gated separately. Identical native alpha compositing and contain-fit sampling; video requires multiple displayed content changes, not a timer or FPS',
    },
  };
  let client, pointerScrollTool;
  const publish = () => onProgress?.(report);
  const abort = () => signal?.throwIfAborted();
  const artifact = (step, path, screenshot = false) => {
    if (!report.artifacts.includes(path)) report.artifacts.push(path);
    if (!step.artifacts.includes(path)) step.artifacts.push(path);
    if (screenshot) {
      if (!report.screenshots.includes(path)) report.screenshots.push(path);
      if (!step.screenshots.includes(path)) step.screenshots.push(path);
    }
    return path;
  };
  const capture = async (step, suffix) => {
    abort();
    const path = join(directory, `${step.name}-${suffix}.png`);
    const png = await client.screenshot(path);
    artifact(step, path, true);
    return png;
  };
  const phase = async (step, id, action) => {
    abort();
    const entry = { id, startedAt: new Date().toISOString() };
    step.details.phases.push(entry); publish();
    try { entry.evidence = await action(); abort(); entry.passed = true; return entry.evidence; }
    catch (error) { entry.passed = false; entry.error = errorText(error); throw error; }
    finally { entry.finishedAt = new Date().toISOString(); publish(); }
  };
  const finishStep = async (step) => {
    step.finishedAt = new Date().toISOString();
    const path = artifact(step, join(directory, `${step.name}-evidence.json`));
    await saveJson(path, step); publish();
  };
  const failure = async (step, error) => {
    step.passed = false; step.error = errorText(error); step.diagnosticErrors = [];
    if (client) {
      try { const path = join(directory, `${step.name}-failure-tree.json`); await client.tree({ path }); artifact(step, path); }
      catch (diagnostic) { step.diagnosticErrors.push(`tree: ${errorText(diagnostic)}`); }
      try { await capture(step, 'failure-window'); }
      catch (diagnostic) { step.diagnosticErrors.push(`screenshot: ${errorText(diagnostic)}`); }
    }
  };
  const open = async (path) => {
    abort(); client = undefined; pointerScrollTool = undefined;
    const url = await restart({ project: path });
    assert.equal(typeof url, 'string', 'restart must return an MCP endpoint');
    client = await createClient(url, { timeoutMs, signal });
    await client.useWindow();
    const tree = await client.tree();
    assert.ok(!tree.debugInfoMissing, 'App must expose Slint debug information');
  };
  const status = async (expected) => {
    let observed;
    try {
      return await client.waitFor(async () => {
        const window = await client.windowProperties(); client.rootHandle = window.rootElementHandle;
        observed = parseStatus((await client.properties(client.rootHandle)).accessibleDescription);
        if (/^Cannot apply inspector edit/.test(observed['IO status'] || '')) throw new Error(observed['IO status']);
        return Object.entries(expected).every(([key, value]) => observed[key] === String(value)) && observed;
      }, { description: `inference UI status ${JSON.stringify(expected)}` });
    } catch (error) { error.message += `; last status: ${JSON.stringify(observed)}`; throw error; }
  };
  const button = (label) => client.find({ label, role: 'Button' });
  const click = async (label) => client.click(await button(label));
  const headers = async () => {
    const tree = await client.tree();
    assert.ok(!tree.truncated, 'Node inspection requires complete UI tree');
    return (tree.elements || []).filter((element) => element.accessibleRole === 'Button' && /^Select node /.test(element.accessibleLabel || ''));
  };
  const header = async (id) => {
    const matches = (await headers()).filter((element) => nodeState(element).id === id);
    assert.equal(matches.length, 1, `Expected one node ${id}`);
    return matches[0];
  };
  const select = async (id) => client.activate(await header(id));
  const scrollInspector = async (deltaY) => {
    const scroll = await client.find({ id: 'Inspector::properties-scroll' });
    if (!pointerScrollTool) {
      const tools = (await client.tools()).tools.filter((tool) => {
        const properties = tool.inputSchema?.properties;
        return properties?.windowHandle && properties.position && properties.deltaY;
      });
      assert.equal(tools.length, 1, 'Expected one positioned MCP pointer-scroll tool');
      pointerScrollTool = tools[0].name;
    }
    await client.value(pointerScrollTool, { windowHandle: client.windowId, position: { x: scroll.absolutePosition.x + 4, y: scroll.absolutePosition.y + scroll.size.height / 2 }, deltaX: 0, deltaY });
  };
  const inspectorControl = async (label, role) => {
    await scrollInspector(10_000);
    for (let attempt = 0; attempt <= 24; attempt++) {
      abort();
      const matches = await client.byLabel(label, { role, visible: false });
      assert.ok(matches.length <= 1, `Expected unique inspector control ${label}`);
      if (matches.length) {
        const control = matches[0];
        const scroll = await client.find({ id: 'Inspector::properties-scroll' });
        const pos = center(control);
        if (pos.y >= scroll.absolutePosition.y + 8 && pos.y <= scroll.absolutePosition.y + scroll.size.height - 8) return control;
      }
      if (attempt < 24) await scrollInspector(-160);
    }
    throw new Error(`Inspector control ${label} not reachable through real scrolling`);
  };
  const sessionStatus = async ({ attached, loading }) => client.waitFor(async () => {
    const window = await client.windowProperties(); client.rootHandle = window.rootElementHandle;
    const observed = parseStatus((await client.properties(client.rootHandle)).accessibleDescription);
    const value = observed['Inference status'] || '';
    if (/failed|refused|unavailable|error/i.test(value)) throw new Error(`Inference session failed: ${value}`);
    return observed['Inference attached'] === String(attached) && observed['Inference loading'] === String(loading)
      && { status: value, attached, loading };
  }, { description: `inference attached=${attached}, loading=${loading}` });
  const load = async (id, model) => {
    await select(id);
    await client.setValue(await inspectorControl('Local model path', 'TextInput'), model.path);
    const control = await inspectorControl('Load model', 'Button');
    assert.equal(control.accessibleEnabled, true, 'Load model must be enabled on this hardware');
    await client.click(control);
    const value = await sessionStatus({ attached: true, loading: false });
    return { modelPath: model.path, sha256: model.sha256, status: value };
  };
  const screenshotPixels = async (step, suffix, dimensions) => {
    await status({ 'Renderer image': 'available', Busy: 'false' });
    const preview = await client.find({ label: 'Renderer preview' });
    assert.equal(preview.accessibleDescription, 'Available');
    const sampled = displayedPixels(await capture(step, suffix), preview, await client.windowProperties(), dimensions);
    const path = join(directory, `${step.name}-${suffix}-renderer.png`);
    await writeFile(path, encodePng(sampled)); artifact(step, path, true);
    return sampled;
  };
  const waitPreviews = async (ids, dimensions) => client.waitFor(async () => {
    const nodes = (await headers()).map(nodeState);
    return ids.every((id) => nodes.find((node) => node.id === id)?.Preview === 'available')
      && nodes.find((node) => node.id === ids.at(-1))?.Resolution === `${dimensions.width}×${dimensions.height}` && nodes;
  }, { description: 'Source, Inference and Renderer actual texture previews and expected Renderer resolution' });
  const videoProgress = async (step, prefix, dimensions, first) => {
    let previous = first || await screenshotPixels(step, `${prefix}-frame-0`, dimensions);
    const evidence = [{ sha256: sha256(previous.pixels), roi: previous.roi }];
    for (let change = 1; change <= 2; change++) {
      let attempt = 0;
      const next = await client.waitFor(async () => {
        const current = await screenshotPixels(step, `${prefix}-change-${change}-probe-${++attempt}`, dimensions);
        const metrics = difference(current, previous);
        // This is a content-change floor, not a numeric-parity tolerance or FPS gate.
        return metrics.changedPixels >= Math.ceil(metrics.totalPixels / 1000) && { current, metrics };
      }, { timeoutMs, intervalMs: 250, description: `video displayed inference content change ${change}/2` });
      evidence.push(next.metrics); previous = next.current;
    }
    return { samples: evidence, atLeastThreeDisplayedFrames: true, last: previous };
  };
  publish();
  try {
    const prerequisite = { name: 'inference-prerequisites', title: 'Pinned artifacts and complete production execution', startedAt: new Date().toISOString(), details: { phases: [] }, artifacts: [], screenshots: [] };
    report.steps.push(prerequisite);
    caseDefinitions = [{ id: prerequisite.name, title: prerequisite.title, description: 'Fail closed on missing manifest, model, fixture, GPU or any production model result; numerical parity remains a separate mandatory gate', screenshotPrefixes: [prerequisite.name] }];
    report.caseDefinitions = caseDefinitions; publish();
    let manifest, production, productionPath;
    try {
      await phase(prerequisite, 'manifest-and-production', async () => {
        const manifestPath = process.env.OPEN_QUARTZ_INFERENCE_E2E_MANIFEST;
        const productionRoot = process.env.OPEN_QUARTZ_INFERENCE_E2E_OUTPUT;
        assert.ok(manifestPath && isAbsolute(manifestPath), 'OPEN_QUARTZ_INFERENCE_E2E_MANIFEST must be an absolute prepared manifest path');
        assert.ok(productionRoot && isAbsolute(productionRoot), 'OPEN_QUARTZ_INFERENCE_E2E_OUTPUT must be the absolute production report directory');
        manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
        assert.equal(manifest.schemaVersion, 1);
        assert.equal(manifest.catalog.length, 7, 'Explicit complete seven-model acceptance requires exactly seven canonical catalog descriptors');
        assert.equal(new Set(manifest.catalog.map((model) => model.id)).size, manifest.catalog.length, 'Duplicate canonical model identity');
        assert.deepEqual(manifest.models.map((model) => model.id).sort(), manifest.catalog.map((model) => model.id).sort(), 'Manifest model artifacts must exactly cover the Rust catalog');
        for (const model of manifest.models) {
          assert.ok(isAbsolute(model.path), `Absolute model path required: ${model.id}`);
          assert.match(model.sha256, /^[a-f0-9]{64}$/);
          assert.equal(sha256(await readFile(model.path)), model.sha256, `Pinned model hash mismatch: ${model.id}`);
        }
        for (const name of ['image', 'alternateImage', 'constantImage', 'video']) {
          const fixture = manifest.fixtures[name];
          assert.ok(fixture && isAbsolute(fixture.path) && fixture.width > 0 && fixture.height > 0, `Required real fixture ${name} missing`);
          assert.ok((await stat(fixture.path)).size > 0, `Empty fixture ${name}`);
        }
        assert.ok(manifest.fixtures.video.frameCount > 1, 'Video fixture must contain multiple content frames');
        productionPath = join(productionRoot, 'production.json');
        production = JSON.parse(await readFile(productionPath, 'utf8'));
        assert.equal(production.productionPassed, true, 'Complete seven-model production execution must pass before UI display diagnostics');
        report.independentReferencePassed = production.passed === true && production.referencePending === false;
        for (const entry of manifest.catalog) {
          assert.match(entry.id, /^[a-z0-9-]+$/, 'Catalog ID must be an artifact-safe identity');
          for (const lane of ['image', 'video']) {
            const matches = production.cases.filter((item) => item.modelId === entry.id && item.input === lane);
            assert.equal(matches.length, 1, `Production must report exactly one ${entry.id}/${lane} case`);
            assert.equal(matches[0].productionPassed, true, 'Partial production artifacts cannot serve as display oracle');
          }
        }
        for (const path of [manifestPath, productionPath]) {
          const local = join(directory, basename(path));
          await copyFile(path, local); artifact(prerequisite, local);
        }
        return { manifestPath, productionPath, catalog: manifest.catalog.map(({ id, label, task }) => ({ id, label, task })), modelHashesVerified: true };
      });
      prerequisite.passed = true;
    } catch (error) { await failure(prerequisite, error); throw error; }
    finally { await finishStep(prerequisite); }

    caseDefinitions = [...caseDefinitions, ...manifest.catalog.flatMap((entry) => ['image', 'video'].map((lane) => ({ id: `inference-${entry.id}-${lane}`, title: `${entry.label} · ${lane}`, description: 'Real catalog menu, typed texture connections, local Load, Renderer screenshot, Pause/Resume, unload/reload, Save/reopen/reload', screenshotPrefixes: [`inference-${entry.id}-${lane}`] })))];
    report.caseDefinitions = caseDefinitions;
    report.catalogCoverage = { source: 'Prepared manifest.catalog exported by Rust inference_models()', expectedModels: manifest.catalog.length, modelIds: manifest.catalog.map((entry) => entry.id), lanes: ['image', 'video'], expectedCases: manifest.catalog.length * 2 };
    publish();
    for (const entry of manifest.catalog) {
      const model = manifest.models.find((item) => item.id === entry.id);
      const output = entry.expectedIO.outputs.filter((item) => item.dataType === 'sampler2D');
      const input = entry.expectedIO.inputs.filter((item) => item.dataType === 'sampler2D');
      const referenceCase = production.cases.find((item) => item.modelId === entry.id && item.input === 'image');
      let oracle;
      for (const lane of ['image', 'video']) {
        abort(); client = undefined;
        const definition = caseDefinitions.find((item) => item.id === `inference-${entry.id}-${lane}`);
        const step = { name: definition.id, title: definition.title, startedAt: new Date().toISOString(), artifacts: [], screenshots: [], details: { modelId: entry.id, lane, phases: [] } };
        report.steps.push(step); publish();
        try {
          assert.equal(input.length, 1, 'Each current catalog model needs one canonical image input');
          assert.equal(output.length, 1, 'Each current catalog model needs one canonical image output');
          const referencePath = join(directory, `${entry.id}-production-renderer.png`);
          await copyFile(resolve(dirname(productionPath), referenceCase.artifacts.renderer), referencePath);
          const reference = decodePng(await readFile(referencePath));
          const dimensions = { width: reference.width, height: reference.height };
          artifact(step, referencePath);
          if (lane === 'image') {
            await phase(step, 'native-screenshot-oracle', async () => {
              const source = await sourceNode({ path: referencePath, ...dimensions }, 'image');
              const renderer = rendererNode();
              const path = join(directory, `${step.name}-oracle.quartz.json`);
              await writeProject(path, `${entry.label} actual production PNG display oracle`, [source, renderer], [{ id: 'oracle_edge', source: source.id, sourceHandle: source.data.outputs[0].id, target: renderer.id, targetHandle: renderer.data.inputs[0].id }]);
              artifact(step, path); await open(path); await status({ Playback: 'stopped', Busy: 'false', Nodes: 2, Connections: 1 });
              await select(renderer.id); await click('PLAY');
              await status({ Playback: 'playing', 'Renderer image': 'available', Busy: 'false' });
              await click('PAUSE'); await status({ Playback: 'paused', 'Renderer image': 'available' });
              oracle = await screenshotPixels(step, 'actual-production-display-oracle', dimensions);
              const content = assertDisplayedContent(oracle);
              return { productionCase: { modelId: referenceCase.modelId, input: referenceCase.input, independentReferencePassed: referenceCase.referencePassed === true }, referencePath, referenceSha256: sha256(await readFile(referencePath)), dimensions, roi: oracle.roi, displayedSha256: sha256(oracle.pixels), displayedContent: content, alphaCompositing: 'Actual production RGBA PNG rendered by normal Slint source -> Renderer with native Theme.card compositing, same image-fit:contain, window and inspector geometry as actual inference; display transport evidence only' };
            });
          }
          const source = await sourceNode(manifest.fixtures[lane], lane);
          const path = join(directory, `${step.name}.quartz.json`);
          let inference, renderer, savedGraph;
          await phase(step, 'source-only-project', async () => {
            await writeProject(path, step.title, [source]); artifact(step, path); await open(path);
            const observed = await status({ Playback: 'stopped', Busy: 'false', Nodes: 1, Connections: 0 });
            assert.equal(resolve(observed['Project path']), path);
            assert.equal(observed['Renderer image'], 'unavailable');
            await capture(step, 'source-only-opened');
            return { path, source: { id: source.id, mode: lane, fixture: manifest.fixtures[lane] }, coverage: 'Only input preloaded. Native image decoder or file-video resource path exercised; input menu/file-picker not claimed.' };
          });
          await phase(step, 'catalog-menu-and-typed-connections', async () => {
            await click('INFERENCE ▾');
            const category = entry.task.replaceAll('-', ' ').toUpperCase();
            await click(new RegExp(`^${escapeRegex(category)}\\s+›$`));
            await click(entry.label.toUpperCase());
            await status({ Playback: 'stopped', Busy: 'false', Nodes: 2, Connections: 0 });
            inference = nodeState((await headers()).find((element) => nodeState(element).id !== source.id));
            const separate = async (id, dx, dy) => {
              const element = await header(id), graph = await client.find({ id: 'AppWindow::graph-view' });
              const position = center(element), left = graph.absolutePosition?.x ?? 0, top = graph.absolutePosition?.y ?? 0;
              await client.drag(element, { x: Math.max(left + element.size.width / 2 + 16, Math.min(position.x + dx, left + graph.size.width - element.size.width / 2 - 16)), y: Math.max(top + 40, Math.min(position.y + dy, top + graph.size.height - 140)) });
            };
            await separate(inference.id, 220, 70); await click('Fit graph in view');
            await click('RENDERER');
            await status({ Playback: 'stopped', Busy: 'false', Nodes: 3, Connections: 0 });
            renderer = nodeState((await headers()).find((element) => ![source.id, inference.id].includes(nodeState(element).id)));
            await separate(renderer.id, 260, 140); await click('Fit graph in view');
            await click(`Output ${source.data.label} image`); await click(`Input ${inference.label} ${input[0].label}`);
            await status({ Playback: 'stopped', Busy: 'false', Connections: 1 });
            await click(`Output ${inference.label} ${output[0].label}`);
            const rendererInputs = await client.byLabel(new RegExp(`^Input ${escapeRegex(renderer.label)} `), { role: 'Button' });
            assert.equal(rendererInputs.length, 1); await client.click(rendererInputs[0]);
            const observed = await status({ Playback: 'stopped', Busy: 'false', Nodes: 3, Connections: 2 });
            assert.equal(observed['Renderer image'], 'unavailable');
            assert.equal((await client.byLabel(/^Select connection /, { role: 'Button' })).length, 2);
            await capture(step, 'menu-created-connected');
            return { category, inference, renderer, canonicalInput: input[0], canonicalOutput: output[0], selectedOutputIndex: entry.expectedIO.outputs.indexOf(output[0]), firstOutputType: entry.expectedIO.outputs[0].dataType, status: observed };
          });
          await phase(step, 'local-model-load', () => load(inference.id, model));
          await phase(step, 'play-and-displayed-content', async () => {
            await select(renderer.id); await click('PLAY');
            const observed = await status({ Playback: 'playing', 'Renderer image': 'available', Busy: 'false' });
            const nodes = await waitPreviews([source.id, inference.id, renderer.id], dimensions);
            if (lane === 'video') {
              const progress = await videoProgress(step, 'playing', dimensions);
              return { status: observed, nodes, progress: { samples: progress.samples, atLeastThreeDisplayedFrames: true } };
            }
            await click('PAUSE'); await status({ Playback: 'paused', 'Renderer image': 'available' });
            assert.ok(oracle, 'Missing production display oracle; image case cannot pass without screenshot comparison');
            const pixels = await screenshotPixels(step, 'image-pixel-oracle', dimensions);
            const metrics = assertOracle(pixels, oracle);
            return { status: observed, nodes, screenshotOracle: metrics };
          });
          await phase(step, 'pause-retention-and-resume', async () => {
            if (lane === 'video') { await click('PAUSE'); await status({ Playback: 'paused', 'Renderer image': 'available', Busy: 'false' }); }
            // Allow any already-submitted native frame to settle, then demand
            // retained displayed content over a second independent screenshot.
            await delay(350, undefined, { signal });
            const first = await screenshotPixels(step, 'pause-retained-before', dimensions);
            await delay(500, undefined, { signal });
            const second = await screenshotPixels(step, 'pause-retained-after', dimensions);
            const retained = difference(second, first);
            assert.equal(retained.maxByteError, 0, 'Pause changed the actual displayed Renderer output');
            await click('RESUME'); await status({ Playback: 'playing', 'Renderer image': 'available' });
            if (lane === 'video') {
              const progress = await videoProgress(step, 'resumed', dimensions, second);
              return { retained, resumed: { samples: progress.samples, atLeastThreeDisplayedFrames: true } };
            }
            await click('PAUSE'); await status({ Playback: 'paused', 'Renderer image': 'available' });
            return { retained, resumed: assertOracle(await screenshotPixels(step, 'image-resumed', dimensions), oracle) };
          });
          await phase(step, 'unload-and-explicit-reload', async () => {
            await click('STOP'); await status({ Playback: 'stopped', Busy: 'false' }); await select(inference.id);
            const unload = await inspectorControl('Unload model', 'Button');
            assert.equal(unload.accessibleEnabled, true); await client.click(unload);
            const unloaded = await sessionStatus({ attached: false, loading: false });
            await select(renderer.id); await status({ 'Renderer image': 'unavailable' }); await capture(step, 'unloaded-no-output');
            const loaded = await load(inference.id, model); await select(renderer.id); await click('PLAY');
            await status({ Playback: 'playing', 'Renderer image': 'available', Busy: 'false' });
            await waitPreviews([source.id, inference.id, renderer.id], dimensions);
            if (lane === 'video') {
              const progress = await videoProgress(step, 'reloaded', dimensions);
              return { unloaded, loaded, progress: { samples: progress.samples, atLeastThreeDisplayedFrames: true } };
            }
            await click('PAUSE'); await status({ Playback: 'paused', 'Renderer image': 'available' });
            return { unloaded, loaded, pixels: assertOracle(await screenshotPixels(step, 'image-reloaded', dimensions), oracle) };
          });
          await phase(step, 'save-and-typed-persistence', async () => {
            await click('STOP'); await status({ Playback: 'stopped', Nodes: 3, Connections: 2, Busy: 'false' });
            await click('FILE ▾'); await click('SAVE');
            const saved = await client.waitFor(async () => {
              let data;
              try { data = JSON.parse(await readFile(path, 'utf8')); }
              catch (error) { if (error.code === 'ENOENT' || error instanceof SyntaxError) return false; throw error; }
              return data.graph?.nodes?.length === 3 && data.graph?.edges?.length === 2 && data;
            }, { description: 'UI SAVE persists complete inference graph' });
            const node = saved.graph.nodes.find((item) => item.id === inference.id);
            assert.equal(node.type, 'inference'); assert.equal(node.data.inferenceCatalogId, entry.id); assert.equal(node.data.inferenceModelId, entry.id);
            assert.equal(node.data.inferenceTask ?? manifest.catalog.find((item) => item.id === node.data.inferenceCatalogId)?.task, entry.task, 'Persisted binding must resolve the canonical task');
            const selected = node.data.outputs.filter((item) => item.dataType === 'sampler2D');
            assert.equal(selected.length, 1); assert.equal(selected[0].label, output[0].label);
            const edge = saved.graph.edges.find((item) => item.source === inference.id && item.target === renderer.id);
            assert.equal(edge?.sourceHandle, selected[0].id, 'Saved edge must use Sampler2d image output, not the first ROI output');
            assert.equal(node.data.inferenceModelPath ?? null, null, 'Temporary local Load path must not be silently saved');
            savedGraph = saved.graph;
            await saveJson(artifact(step, join(directory, `${step.name}-saved-project-observed.json`)), saved);
            await capture(step, 'saved-stopped');
            return { path, inferenceId: inference.id, sourceHandle: edge.sourceHandle, selectedType: selected[0].dataType, nodeIds: saved.graph.nodes.map((item) => item.id), edges: saved.graph.edges };
          });
          await phase(step, 'reopen-stopped-and-explicit-reload', async () => {
            await open(path);
            const observed = await status({ Playback: 'stopped', Busy: 'false', Nodes: 3, Connections: 2, 'Renderer image': 'unavailable' });
            assert.equal(resolve(observed['Project path']), path);
            assert.deepEqual((await headers()).map(nodeState).map((node) => node.id).sort(), savedGraph.nodes.map((node) => node.id).sort(), 'Reopen changed graph node identities');
            assert.deepEqual(JSON.parse(await readFile(path, 'utf8')).graph, savedGraph, 'Reopen mutated the saved graph');
            await select(inference.id); await sessionStatus({ attached: false, loading: false });
            assert.equal((await inspectorControl('Local model path', 'TextInput')).accessibleValue || '', '', 'Live Load override must not survive reopen');
            assert.equal(Boolean((await inspectorControl('Unload model', 'Button')).accessibleEnabled), false, 'Reopen must not silently restore a live model session');
            await capture(step, 'reopened-no-live-session');
            const loaded = await load(inference.id, model); await select(renderer.id); await click('PLAY');
            await status({ Playback: 'playing', Busy: 'false', 'Renderer image': 'available' });
            await waitPreviews([source.id, inference.id, renderer.id], dimensions);
            let pixels;
            if (lane === 'video') {
              const progress = await videoProgress(step, 'reopened', dimensions);
              pixels = { samples: progress.samples, atLeastThreeDisplayedFrames: true };
            } else {
              await click('PAUSE'); await status({ Playback: 'paused', 'Renderer image': 'available' });
              pixels = assertOracle(await screenshotPixels(step, 'image-reopened-reloaded', dimensions), oracle);
            }
            await click('STOP'); await status({ Playback: 'stopped', Busy: 'false', Nodes: 3, Connections: 2 });
            return { status: observed, loaded, pixels };
          });
          step.passed = true;
        } catch (error) {
          await failure(step, error);
          if (signal?.aborted) throw error;
          // Assertion failures remain per-model/lane failures; the next normal
          // app restart isolates them without silently skipping any catalog case.
        } finally { await finishStep(step); }
      }
    }
    report.displayedPixelsPassed = report.steps.length === caseDefinitions.length && report.steps.every((step) => step.passed === true);
    report.passed = report.displayedPixelsPassed && report.independentReferencePassed;
    if (!report.passed) throw new Error(`Inference regression incomplete: displayedPixelsPassed=${report.displayedPixelsPassed}, independentReferencePassed=${report.independentReferencePassed}; UI failures: ${report.steps.filter((step) => !step.passed).map((step) => step.name).join(', ')}`);
    return report;
  } catch (error) { report.error = errorText(error); error.scenarioResult = report; throw error; }
  finally { report.finishedAt = new Date().toISOString(); publish(); await saveJson(join(directory, 'result.json'), report); }
}
