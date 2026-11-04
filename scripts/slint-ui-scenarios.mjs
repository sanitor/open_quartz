import assert from 'node:assert/strict';
import { readFile, mkdir } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { inflateSync } from 'node:zlib';
import { createClient, saveJson } from './slint-ui-client.mjs';
import { UI_CASES, STATUS_LABELS } from './slint-ui-report.mjs';

export const BLUE_SHADER_SOURCE = `// UI automation persistence and GPU output probe.
@fragment
fn main(@location(0) uv: vec2f) -> @location(0) vec4f {
  return vec4f(0.0, 0.0, 1.0, 1.0);
}
`;
export const PERSISTED_SHADER_LABEL = 'UI Blue Shader';

const escapeRegex = (text) => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
const center = (element) => ({ x: (element.absolutePosition?.x ?? 0) + element.size.width / 2, y: (element.absolutePosition?.y ?? 0) + element.size.height / 2 });
const near = (actual, expected, tolerance, message) => assert.ok(Math.abs(actual - expected) <= tolerance, `${message}: expected ${expected} ± ${tolerance}, received ${actual}`);

export function parseStatus(description = '') {
  return Object.fromEntries(description.split(';').map((part) => {
    const separator = part.indexOf(':');
    return separator < 0 ? [] : [part.slice(0, separator).trim(), part.slice(separator + 1).trim()];
  }).filter((entry) => entry.length === 2));
}

function nodeState(element) {
  const values = parseStatus(element.accessibleDescription);
  const id = values['Node id'];
  const x = Number(values['World x']);
  const y = Number(values['World y']);
  assert.ok(id && Number.isFinite(x) && Number.isFinite(y), `Node header needs Node id/World x/World y: ${element.accessibleDescription}`);
  return { id, x, y, label: element.accessibleLabel.slice('Select node '.length), center: center(element), size: element.size };
}

/** Decode non-interlaced, 8-bit PNG screenshots using only Node built-ins. */
export function decodePng(png) {
  assert.ok(png.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10])), 'Invalid PNG signature');
  let width, height, channels;
  const idat = [];
  let ended = false;
  for (let offset = 8; offset + 12 <= png.length;) {
    const length = png.readUInt32BE(offset);
    const type = png.toString('ascii', offset + 4, offset + 8);
    assert.ok(offset + 12 + length <= png.length, 'Truncated PNG chunk');
    const data = png.subarray(offset + 8, offset + 8 + length);
    if (type === 'IHDR') {
      assert.equal(length, 13, 'Invalid PNG IHDR');
      width = data.readUInt32BE(0); height = data.readUInt32BE(4);
      assert.equal(data[8], 8, 'PNG pixel verification requires 8-bit samples');
      channels = ({ 0: 1, 2: 3, 4: 2, 6: 4 })[data[9]];
      assert.ok(channels, `Unsupported PNG color type ${data[9]}`);
      assert.equal(data[10], 0, 'Unsupported PNG compression');
      assert.equal(data[11], 0, 'Unsupported PNG filter method');
      assert.equal(data[12], 0, 'PNG pixel verification does not support interlacing');
    } else if (type === 'IDAT') idat.push(data);
    else if (type === 'IEND') { ended = true; break; }
    offset += length + 12;
  }
  assert.ok(ended && width > 0 && height > 0 && channels && idat.length, 'Incomplete PNG');
  const stride = width * channels;
  const expectedBytes = height * (stride + 1);
  assert.ok(expectedBytes < 256 * 1024 * 1024, 'Screenshot exceeds PNG decoder limit');
  const raw = inflateSync(Buffer.concat(idat), { maxOutputLength: expectedBytes });
  assert.equal(raw.length, expectedBytes, 'Incorrect PNG scanline size');
  const pixels = Buffer.alloc(height * stride);
  const paeth = (a, b, c) => {
    const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c);
    return pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
  };
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)];
    assert.ok(filter <= 4, `Unsupported PNG row filter ${filter}`);
    for (let x = 0; x < stride; x++) {
      const index = y * stride + x;
      const a = x >= channels ? pixels[index - channels] : 0;
      const b = y > 0 ? pixels[index - stride] : 0;
      const c = y > 0 && x >= channels ? pixels[index - stride - channels] : 0;
      const predictor = [0, a, b, Math.floor((a + b) / 2), paeth(a, b, c)][filter];
      pixels[index] = (raw[y * (stride + 1) + 1 + x] + predictor) & 255;
    }
  }
  return { width, height, channels, pixels };
}

/** Sample only the center of the actual Renderer preview, excluding UI chrome. */
export function bluePixelStats(png, preview, windowProperties) {
  const image = decodePng(png);
  const scale = windowProperties.scaleFactor || 1;
  const logicalWidth = windowProperties.size.width / scale;
  const logicalHeight = windowProperties.size.height / scale;
  const sx = image.width / logicalWidth, sy = image.height / logicalHeight;
  const mid = center(preview);
  // A small central square stays inside a contained, square renderer image.
  const half = Math.min(preview.size.width, preview.size.height) * 0.15;
  assert.ok(half >= 8, 'Renderer preview is too small for a useful pixel assertion');
  const roi = { left: Math.ceil((mid.x - half) * sx), top: Math.ceil((mid.y - half) * sy), right: Math.floor((mid.x + half) * sx), bottom: Math.floor((mid.y + half) * sy) };
  assert.ok(roi.left >= 0 && roi.top >= 0 && roi.right <= image.width && roi.bottom <= image.height, 'Renderer ROI lies outside screenshot');
  let bluePixels = 0, totalPixels = 0;
  for (let y = roi.top; y < roi.bottom; y++) {
    for (let x = roi.left; x < roi.right; x++) {
      const offset = (y * image.width + x) * image.channels;
      const r = image.pixels[offset];
      const g = image.channels < 3 ? r : image.pixels[offset + 1];
      const b = image.channels < 3 ? r : image.pixels[offset + 2];
      const alpha = image.channels === 4 ? image.pixels[offset + 3] : image.channels === 2 ? image.pixels[offset + 1] : 255;
      if (b >= 180 && r <= 70 && g <= 70 && alpha >= 240) bluePixels++;
      totalPixels++;
    }
  }
  return { screenshotWidth: image.width, screenshotHeight: image.height, roi, bluePixels, totalPixels, blueRatio: totalPixels ? bluePixels / totalPixels : 0 };
}

/**
 * Exercise the real app exclusively through MCP UI input and accessibility.
 * The caller launches an empty default app and supplies an existing empty project.
 * restart({project}) must stop/relaunch the app and resolve to its HTTP MCP URL.
 * On failure result.json/failure-tree.json/failure.png are attempted, then thrown.
 */
export async function runScenarios({ endpoint, outputDir, projectPath, requireGpu = true, restart, dragTolerance = 3, timeoutMs = 15_000, signal, onProgress } = {}) {
  assert.ok(endpoint && outputDir && projectPath, 'endpoint, outputDir and projectPath are required');
  assert.equal(typeof restart, 'function', 'restart({project}) is required for persistence verification');
  const directory = resolve(outputDir);
  const project = resolve(projectPath);
  await mkdir(directory, { recursive: true });
  const report = { passed: false, startedAt: new Date().toISOString(), endpoint, projectPath: project, requireGpu, steps: [], screenshots: [], dragTolerance };
  report.interactions = { menusToolbarPorts: 'MCP click_element pointer input', nodeSelection: 'MCP invoke_accessibility_action Default_', nodeMovement: 'MCP drag_element pointer input', editing: 'MCP set_element_value accessibility input' };
  onProgress?.(report);
  let client;
  let savedGraph;
  const checkpoint = async (name, action) => {
    const caseIndex = UI_CASES.findIndex((entry) => entry.id === name);
    assert.ok(caseIndex >= 0, `Missing case description: ${name}`);
    const step = { name, title: UI_CASES[caseIndex].title, startedAt: new Date().toISOString() };
    const firstScreenshot = report.screenshots.length;
    report.steps.push(step);
    onProgress?.(report);
    console.log(`[slint-ui] ${caseIndex + 1}/${UI_CASES.length} 开始：${step.title}`);
    try { signal?.throwIfAborted(); step.details = await action(); signal?.throwIfAborted(); step.passed = true; }
    catch (error) {
      if (!signal?.aborted) step.passed = false;
      step.error = error.stack || String(error);
      throw error;
    }
    finally {
      step.finishedAt = new Date().toISOString();
      step.screenshots = report.screenshots.slice(firstScreenshot);
      onProgress?.(report);
      console.log(`[slint-ui] ${caseIndex + 1}/${UI_CASES.length} ${STATUS_LABELS[step.passed === true ? 'passed' : step.passed === false ? 'failed' : 'incomplete']}：${step.title}`);
    }
  };
  const capture = async (name) => {
    signal?.throwIfAborted();
    const path = join(directory, `${name}.png`);
    const png = await client.screenshot(path);
    report.screenshots.push(path);
    return png;
  };
  const connect = async (url) => {
    signal?.throwIfAborted();
    client = await createClient(url, { timeoutMs, signal });
    await client.useWindow();
    const tree = await client.tree();
    assert.ok(!tree.debugInfoMissing, 'App must be built with Slint debug information');
  };
  const restartApp = async () => {
    signal?.throwIfAborted();
    const url = await restart({ project });
    signal?.throwIfAborted();
    assert.equal(typeof url, 'string', 'restart must return an endpoint string');
    return url;
  };
  const status = async (expected = {}) => {
    let observed;
    try {
      return await client.waitFor(async () => {
        observed = parseStatus((await client.properties(client.rootHandle)).accessibleDescription);
        if (/^Cannot apply inspector edit/.test(observed['IO status'] ?? '')) throw new Error(`UI rejected inspector edit: ${JSON.stringify(observed)}`);
        return Object.entries(expected).every(([key, value]) => observed[key] === String(value)) && observed;
      }, { description: `status ${JSON.stringify(expected)}` });
    } catch (error) {
      error.observedStatus = observed;
      error.message += `; last UI status: ${JSON.stringify(observed)}`;
      throw error;
    }
  };
  const stopped = (nodes, edges) => status({ Playback: 'stopped', Nodes: nodes, Connections: edges, Busy: 'false' });
  const find = async (label) => client.find({ label, role: label === 'Node label' || label === 'Shader source' ? 'TextInput' : 'Button' });
  const activate = async (label) => client.click(await find(label));
  const headers = async () => client.byLabel(/^Select node /, { role: 'Button' });
  const header = async (id) => {
    const matches = (await headers()).filter((element) => nodeState(element).id === id);
    assert.equal(matches.length, 1, `Expected one node header for ${id}`);
    return matches[0];
  };
  const waitNode = async (id, expected) => client.waitFor(async () => {
    const actual = nodeState(await header(id));
    return Math.abs(actual.x - expected.x) <= 0.1 && Math.abs(actual.y - expected.y) <= 0.1 && actual;
  }, { description: `node ${id} world position ${JSON.stringify(expected)}` });
  const drag = async (id, dx, dy, name) => {
    const element = await header(id);
    const before = nodeState(element);
    const zoom = before.size.height / 28;
    assert.ok(zoom > 0, 'Node header must have positive zoom');
    const target = { x: before.center.x + dx, y: before.center.y + dy };
    await client.drag(element, target);
    const after = await waitNode(id, { x: before.x + dx / zoom, y: before.y + dy / zoom });
    near(after.center.x, target.x, dragTolerance, `${name} screen x`);
    near(after.center.y, target.y, dragTolerance, `${name} screen y`);
    await stopped(1, 0);
    await capture(name);
    return { before, after, target, zoom, dx, dy };
  };
  try {
    await checkpoint('default-empty-start-stopped', async () => {
      await connect(endpoint);
      const observed = await stopped(0, 0);
      assert.equal(observed['Renderer image'], 'unavailable');
      assert.equal((await headers()).length, 0);
      await capture('01-default-empty-stopped');
      return observed;
    });
    await checkpoint('open-empty-project-stopped', async () => {
      const url = await restartApp();
      assert.equal(typeof url, 'string', 'restart must return an endpoint string');
      await connect(url);
      const observed = await stopped(0, 0);
      assert.equal(resolve(observed['Project path'] || ''), project, 'App reopened a different project');
      return observed;
    });
    let shaderId;
    await checkpoint('menu-add-default-editable-shader-stopped', async () => {
      await activate('SHADER ▾');
      await activate(/^CUSTOM\s+›$/);
      await activate('CUSTOM SHADER');
      await stopped(1, 0);
      const nodes = await headers();
      assert.equal(nodes.length, 1);
      shaderId = nodeState(nodes[0]).id;
      await client.activate(nodes[0]);
      const source = await find('Shader source');
      assert.equal(Boolean(source.accessibleReadOnly), false, 'Default custom Shader must be editable');
      assert.ok(source.accessibleValue?.includes('@fragment'), 'Default Shader source is absent');
      await capture('02-default-shader');
      return nodeState(await header(shaderId));
    });
    for (const [name, dx, dy] of [['positive-x', 42, 0], ['negative-x', -42, 0], ['positive-y', 0, 32], ['negative-y', 0, -32]]) {
      await checkpoint(`single-node-drag-${name}`, () => drag(shaderId, dx, dy, `03-drag-${name}`));
    }
    await checkpoint('zoom-out-then-drag', async () => {
      const before = nodeState(await header(shaderId));
      await activate('Zoom out');
      const after = nodeState(await header(shaderId));
      assert.ok(after.size.height < before.size.height, 'Zoom out did not change node screen size');
      return drag(shaderId, 34, -24, '04-zoom-out-drag');
    });
    await checkpoint('zoom-in-then-drag', async () => {
      const before = nodeState(await header(shaderId));
      await activate('Zoom in');
      const after = nodeState(await header(shaderId));
      assert.ok(after.size.height > before.size.height, 'Zoom in did not change node screen size');
      return drag(shaderId, -34, 24, '05-zoom-in-drag');
    });
    let fitDrag;
    await checkpoint('fit-then-drag', async () => {
      await activate('Fit graph in view');
      fitDrag = await drag(shaderId, 38, 26, '06-fit-drag');
      return fitDrag;
    });
    await checkpoint('undo-redo-node-drag', async () => {
      await activate('UNDO');
      const undone = await waitNode(shaderId, fitDrag.before);
      near(undone.center.x, fitDrag.before.center.x, dragTolerance, 'Undo screen x');
      near(undone.center.y, fitDrag.before.center.y, dragTolerance, 'Undo screen y');
      await capture('07-undo');
      await activate('REDO');
      const redone = await waitNode(shaderId, fitDrag.after);
      near(redone.center.x, fitDrag.after.center.x, dragTolerance, 'Redo screen x');
      near(redone.center.y, fitDrag.after.center.y, dragTolerance, 'Redo screen y');
      await stopped(1, 0);
      await capture('08-redo');
      return { undone, redone };
    });
    await checkpoint('edit-shader-label-and-source', async () => {
      await client.activate(await header(shaderId));
      await client.setValue(await find('Node label'), PERSISTED_SHADER_LABEL);
      await activate('Apply node label');
      assert.equal(nodeState(await header(shaderId)).label, PERSISTED_SHADER_LABEL);
      await client.setValue(await find('Shader source'), BLUE_SHADER_SOURCE);
      assert.equal((await client.properties(await find('Shader source'))).accessibleValue, BLUE_SHADER_SOURCE);
      const source = await find('Shader source');
      await client.activate(source);
      const primary = process.platform === 'darwin' ? '\u0017' : '\u0011';
      await client.key(primary, 'Press');
      try { await client.key('a'); } finally { await client.key(primary, 'Release'); }
      await client.key('/');
      assert.equal((await client.properties(source)).accessibleValue, '/', 'Real editor key input must replace the selected source');
      await client.key(primary, 'Press');
      try { await client.key('z'); } finally { await client.key(primary, 'Release'); }
      assert.equal((await client.properties(source)).accessibleValue, BLUE_SHADER_SOURCE, 'One editor undo must restore source without changing graph history');
      await status({ 'Inspector dirty': 'true' });
      const png = await capture('09-shader-draft');
      const image = decodePng(png);
      const surface = await find('Shader source');
      const window = await client.windowProperties();
      const scale = image.width / (window.size.width / window.scaleFactor);
      const hues = new Set();
      for (let y = Math.ceil(surface.absolutePosition.y * scale); y < Math.min(image.height, (surface.absolutePosition.y + surface.size.height) * scale); y++) {
        for (let x = Math.ceil(surface.absolutePosition.x * scale); x < Math.min(image.width, (surface.absolutePosition.x + surface.size.width) * scale); x++) {
          const offset = (y * image.width + x) * image.channels;
          const [r, g, b] = image.pixels.subarray(offset, offset + 3);
          if (Math.max(r, g, b) - Math.min(r, g, b) > 50) {
            hues.add(r > g && b > g ? 'purple' : g > r && g > b ? 'green' : b > r && b > g ? 'blue' : r > b && r > g ? 'warm' : 'other');
          }
        }
      }
      assert.ok(hues.size >= 3, `Editable source must visibly render multiple syntax classes, observed ${[...hues]}`);
      await activate('Apply shader');
      await status({ 'Inspector dirty': 'false', 'IO status': 'Shader source applied through the SDK.' });
      await stopped(1, 0);
      assert.equal((await find('Shader source')).accessibleValue, BLUE_SHADER_SOURCE);
      await capture('09-edited-shader');
      return { label: PERSISTED_SHADER_LABEL, source: BLUE_SHADER_SOURCE };
    });
    let rendererId;
    await checkpoint('menu-add-renderer-connect-stopped', async () => {
      await activate('RENDERER');
      await stopped(2, 0);
      const renderer = (await headers()).filter((element) => nodeState(element).id !== shaderId);
      assert.equal(renderer.length, 1);
      rendererId = nodeState(renderer[0]).id;
      const rendererLabel = nodeState(renderer[0]).label;
      // New nodes can overlap the existing Shader. Lay out the Renderer with a
      // genuine pointer drag before trying to hit either node's port targets.
      await capture('10-renderer-added');
      const rendererBefore = nodeState(await header(rendererId));
      const graphView = await client.find({ id: 'AppWindow::graph-view' });
      const graphPosition = { x: graphView.absolutePosition?.x ?? 0, y: graphView.absolutePosition?.y ?? 0 };
      const rendererTarget = {
        x: Math.min(rendererBefore.center.x + 260, graphPosition.x + graphView.size.width - rendererBefore.size.width / 2 - 16),
        y: Math.min(rendererBefore.center.y + 140, graphPosition.y + graphView.size.height - 140),
      };
      const rendererZoom = rendererBefore.size.height / 28;
      await client.drag(await header(rendererId), rendererTarget);
      const rendererAfter = await waitNode(rendererId, {
        x: rendererBefore.x + (rendererTarget.x - rendererBefore.center.x) / rendererZoom,
        y: rendererBefore.y + (rendererTarget.y - rendererBefore.center.y) / rendererZoom,
      });
      near(rendererAfter.center.x, rendererTarget.x, dragTolerance, 'Renderer layout screen x');
      near(rendererAfter.center.y, rendererTarget.y, dragTolerance, 'Renderer layout screen y');
      await activate('Fit graph in view');
      await capture('10-nodes-separated-fit');
      const outputs = await client.byLabel(new RegExp(`^Output ${escapeRegex(PERSISTED_SHADER_LABEL)} `), { role: 'Button' });
      const inputs = await client.byLabel(new RegExp(`^Input ${escapeRegex(rendererLabel)} `), { role: 'Button' });
      assert.equal(outputs.length, 1, 'Blue Shader should have exactly one output');
      assert.equal(inputs.length, 1, 'Renderer should have exactly one input');
      const circle = async (label) => {
        const port = await client.find({ label, role: 'Button' });
        const tree = await client.tree({ root: port.handle, maxElements: 20 });
        const dots = tree.elements.filter((e) => e.typeNamesAndIds?.some((t) => t.typeName === 'Rectangle') && e.size.width > 0 && Math.abs(e.size.width - e.size.height) < 0.01 && e.size.height < port.size.height);
        assert.equal(dots.length, 1, `Expected one port circle for ${label}`);
        return dots[0];
      };
      await client.drag(await circle(outputs[0].accessibleLabel), center(outputs[0]));
      await stopped(2, 0);
      // Releasing on blank canvas must not create an edge or move either node.
      await client.drag(await circle(outputs[0].accessibleLabel), { x: graphPosition.x + 12, y: graphPosition.y + 12 });
      await stopped(2, 0);
      await client.drag(await circle(outputs[0].accessibleLabel), center(await circle(inputs[0].accessibleLabel)));
      await stopped(2, 1);
      await activate('UNDO');
      await stopped(2, 0);
      await activate('Zoom out');
      await activate('Zoom out');
      // Support starting from the input as well; one drag is one history entry.
      await client.drag(await circle(inputs[0].accessibleLabel), center(await circle(outputs[0].accessibleLabel)));
      await stopped(2, 1);
      await activate('UNDO');
      await stopped(2, 0);
      await activate('Fit graph in view');
      // Keep the existing click-to-connect route covered alongside dragging.
      await client.click(await client.find({ label: outputs[0].accessibleLabel }));
      await client.click(await client.find({ label: inputs[0].accessibleLabel }));
      const observed = await stopped(2, 1);
      assert.equal(observed['Renderer image'], 'unavailable', 'Stopped graph must not start producing renderer frames');
      assert.equal((await client.byLabel(/^Select connection /, { role: 'Button' })).length, 1);
      await client.activate(await header(rendererId));
      await capture('10-connected-stopped');
      return { shaderId, rendererId, rendererLayout: { before: rendererBefore, after: rendererAfter, target: rendererTarget }, status: observed };
    });
    await checkpoint('play-real-renderer-preview', async () => {
      await activate('PLAY');
      const observed = await status({ Playback: 'playing', Busy: 'false', ...(requireGpu ? { 'Renderer image': 'available' } : {}) });
      let pixels = { verified: false, reason: 'requireGpu=false' };
      if (requireGpu) {
        const preview = await client.waitFor(async () => {
          const matches = await client.byLabel('Renderer preview');
          return matches.length === 1 && matches[0].accessibleDescription === 'Available' && matches[0];
        }, { description: 'Renderer preview Available' });
        await client.waitFor(async () => {
          const nodes = await headers();
          return nodes.length === 2 && nodes.every((node) => parseStatus(node.accessibleDescription).Preview === 'available');
        }, { description: 'Shader and Renderer node thumbnails available' });
        const png = await capture('11-playing-blue-preview');
        pixels = bluePixelStats(png, preview, await client.windowProperties());
        assert.ok(pixels.totalPixels >= 256 && pixels.blueRatio >= 0.8, `Renderer preview is not blue: ${JSON.stringify(pixels)}`);
        pixels.verified = true;
      } else await capture('11-playing-preview');
      report.gpu = pixels;
      return { status: observed, pixels };
    });
    await checkpoint('stop-playback', async () => {
      await activate('STOP');
      const observed = await stopped(2, 1);
      await capture('12-stopped');
      return observed;
    });
    await checkpoint('save-project-through-file-menu', async () => {
      const nodes = (await headers()).map(nodeState);
      const edges = (await client.byLabel(/^Select connection /, { role: 'Button' })).map((element) => element.accessibleLabel).sort();
      await activate('FILE ▾');
      await activate('SAVE');
      // Reading the saved artifact checks completion, never creates/mutates app state.
      const saved = await client.waitFor(async () => {
        let data;
        try { data = JSON.parse(await readFile(project, 'utf8')); }
        catch (error) { if (error.code === 'ENOENT' || error instanceof SyntaxError) return false; throw error; }
        return data.graph?.nodes?.length === 2 && data.graph?.edges?.length === 1 && data;
      }, { description: 'SAVE to persist two nodes and one edge' });
      assert.ok(JSON.stringify(saved).includes(JSON.stringify(BLUE_SHADER_SOURCE)), 'Saved project lost edited shader source');
      assert.ok(JSON.stringify(saved).includes(PERSISTED_SHADER_LABEL), 'Saved project lost edited label');
      savedGraph = { nodes, edges, graph: saved.graph };
      await saveJson(join(directory, 'saved-project-observed.json'), saved);
      await client.tree({ path: join(directory, 'saved-tree.json') });
      return savedGraph;
    });
    await checkpoint('restart-same-project-retains-graph-parameters-stopped', async () => {
      await connect(await restartApp());
      const observed = await stopped(2, 1);
      assert.equal(resolve(observed['Project path'] || ''), project, 'Restart opened a different project');
      assert.equal(observed['Renderer image'], 'unavailable', 'Restart should remain stopped without a live preview');
      const nodes = (await headers()).map(nodeState);
      assert.deepEqual(nodes.map((node) => node.id).sort(), savedGraph.nodes.map((node) => node.id).sort(), 'Node IDs changed after restart');
      for (const saved of savedGraph.nodes) {
        const reopened = nodes.find((node) => node.id === saved.id);
        assert.equal(reopened.label, saved.label);
        near(reopened.x, saved.x, 0.1, 'Reopened world x');
        near(reopened.y, saved.y, 0.1, 'Reopened world y');
      }
      assert.deepEqual((await client.byLabel(/^Select connection /, { role: 'Button' })).map((element) => element.accessibleLabel).sort(), savedGraph.edges, 'Connections changed after restart');
      await client.activate(await header(shaderId));
      assert.equal((await find('Node label')).accessibleValue, PERSISTED_SHADER_LABEL, 'Reopened node label editor lost value');
      assert.equal((await find('Shader source')).accessibleValue, BLUE_SHADER_SOURCE, 'Reopened Shader source lost value');
      assert.equal(Boolean((await find('Shader source')).accessibleReadOnly), false);
      const onDisk = JSON.parse(await readFile(project, 'utf8'));
      assert.deepEqual(onDisk.graph, savedGraph.graph, 'Restart modified the saved graph');
      await capture('13-reopened-shader-stopped');
      await client.activate(await header(rendererId));
      await capture('14-reopened-connected-stopped');
      await client.tree({ path: join(directory, 'reopened-tree.json') });
      return { status: observed, nodes, edges: savedGraph.edges };
    });
    report.passed = true;
    return report;
  } catch (error) {
    report.error = error.stack || String(error);
    report.diagnosticErrors = [];
    if (client && !signal?.aborted) {
      try { await client.tree({ path: join(directory, 'failure-tree.json') }); }
      catch (failure) { report.diagnosticErrors.push(`tree: ${failure.message}`); }
      try { await capture('failure'); }
      catch (failure) { report.diagnosticErrors.push(`screenshot: ${failure.message}`); }
    }
    error.scenarioResult = report;
    throw error;
  } finally {
    report.finishedAt = new Date().toISOString();
    await saveJson(join(directory, 'result.json'), report);
  }
}
