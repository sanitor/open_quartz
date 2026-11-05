import assert from 'node:assert/strict';
import { copyFile, mkdir, readFile, writeFile } from 'node:fs/promises';
import { basename, join, resolve } from 'node:path';
import { createHash } from 'node:crypto';
import { createClient, saveJson } from './slint-ui-client.mjs';
import { decodePng, parseStatus } from './slint-ui-scenarios.mjs';
import { prepareShaderFixtures } from './shader-e2e-fixtures.mjs';
import { compareShaderOutput, encodePng } from './shader-e2e-reference.mjs';

const escapeRegex = (value) => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex');
const errorText = (error) => error?.stack || String(error);

// Filled from the catalog-backed fixture inventory, not a second shader list.
export let caseDefinitions = [];

function nodeState(element) {
  const state = parseStatus(element.accessibleDescription);
  assert.ok(state['Node id'], `Missing node identity: ${element.accessibleDescription}`);
  return { id: state['Node id'], label: element.accessibleLabel.slice('Select node '.length), ...state };
}

function numericValues(value) {
  if (Array.isArray(value)) return value.flat(Infinity).map(Number);
  return String(value).trim().replace(/^\[|\]$/g, '').split(/[,\s]+/).map(Number);
}

function assertUniformValue(actual, expected, name) {
  const observed = numericValues(actual);
  const wanted = numericValues(expected);
  assert.equal(observed.length, wanted.length, `Uniform ${name} component count`);
  for (let index = 0; index < wanted.length; index++) {
    assert.ok(Number.isFinite(observed[index]) && Math.abs(observed[index] - wanted[index]) <= 1e-5 * Math.max(1, Math.abs(wanted[index])), `Uniform ${name}[${index}]: expected ${wanted[index]}, received ${observed[index]}`);
  }
}

/** Extract texel-center samples from the actual contain-fitted Renderer image.
 * The original full-window screenshot is retained separately as UI evidence.
 */
function rendererPixels(png, preview, windowProperties, width, height) {
  const screenshot = decodePng(png);
  const scale = windowProperties.scaleFactor || 1;
  const sx = screenshot.width / (windowProperties.size.width / scale);
  const sy = screenshot.height / (windowProperties.size.height / scale);
  const fit = Math.min(preview.size.width / width, preview.size.height / height);
  assert.ok(fit >= 1, `Renderer preview too small for ${width}×${height} texel sampling`);
  const left = (preview.absolutePosition?.x ?? 0) + (preview.size.width - width * fit) / 2;
  const top = (preview.absolutePosition?.y ?? 0) + (preview.size.height - height * fit) / 2;
  const pixels = Buffer.alloc(width * height * 4);
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const px = Math.floor((left + (x + 0.5) * fit) * sx);
      const py = Math.floor((top + (y + 0.5) * fit) * sy);
      assert.ok(px >= 0 && py >= 0 && px < screenshot.width && py < screenshot.height, 'Renderer image lies outside screenshot');
      const source = (py * screenshot.width + px) * screenshot.channels;
      const target = (y * width + x) * 4;
      pixels[target] = screenshot.pixels[source];
      pixels[target + 1] = screenshot.channels >= 3 ? screenshot.pixels[source + 1] : screenshot.pixels[source];
      pixels[target + 2] = screenshot.channels >= 3 ? screenshot.pixels[source + 2] : screenshot.pixels[source];
      // This is the displayed (composited) pixel, not a GPU alpha readback.
      pixels[target + 3] = 255;
    }
  }
  return { width, height, pixels, roi: { left: left * sx, top: top * sy, width: width * fit * sx, height: height * fit * sy }, screenshotWidth: screenshot.width, screenshotHeight: screenshot.height };
}

/**
 * Run every catalog shader in its own real native app, using preloaded, edge-free
 * projects. Project loading exercises the normal image file-service decoder;
 * every graph edge and transport action is genuine MCP pointer input. This
 * suite does not claim that loading a project tests the shader creation menu.
 */
export async function runShaderScenarios({ endpoint, outputDir, restart, signal, onProgress, timeoutMs = 15_000 } = {}) {
  assert.ok(endpoint && outputDir, 'endpoint and outputDir are required');
  assert.equal(typeof restart, 'function', 'restart({project}) is required');
  const directory = resolve(outputDir);
  await mkdir(directory, { recursive: true });
  const report = {
    suite: 'shaders', passed: false, endpoint, startedAt: new Date().toISOString(),
    steps: [], screenshots: [], artifacts: [], shaderCases: [], caseDefinitions: [],
    interactions: { setup: 'Catalog-backed edge-free project loaded by the app; no menu-creation coverage claimed', nodeSelection: 'MCP invoke_accessibility_action Default_', portsAndTransport: 'MCP click_element real pointer input', pixels: 'Paused native-window Renderer ROI compared with CPU reference; no GPU readback substitution' },
  };
  let client;
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
  const phase = async (step, id, title, action) => {
    abort();
    const entry = { id, title, startedAt: new Date().toISOString() };
    step.details.phases.push(entry);
    publish();
    try {
      entry.evidence = await action();
      abort();
      entry.passed = true;
      return entry.evidence;
    } catch (error) {
      if (!signal?.aborted) entry.passed = false;
      entry.error = errorText(error);
      throw error;
    } finally {
      entry.finishedAt = new Date().toISOString();
      publish();
    }
  };
  const rootStatus = async (expected) => {
    let observed;
    try {
      return await client.waitFor(async () => {
        const window = await client.windowProperties();
        assert.ok(window.rootElementHandle, 'Window must expose its current root element');
        client.rootHandle = window.rootElementHandle;
        observed = parseStatus((await client.properties(client.rootHandle)).accessibleDescription);
        if (/^Cannot apply inspector edit/.test(observed['IO status'] || '')) throw new Error(`App rejected inspector state: ${observed['IO status']}`);
        return Object.entries(expected).every(([key, value]) => observed[key] === String(value)) && observed;
      }, { description: `shader status ${JSON.stringify(expected)}` });
    } catch (error) {
      error.message += `; last UI status: ${JSON.stringify(observed)}`;
      throw error;
    }
  };
  const button = (label) => client.find({ label, role: 'Button' });
  const click = async (label) => client.click(await button(label));
  const collapseShaderEditor = async () => {
    const expanded = await client.byLabel('Collapse SHADER EDITOR', { role: 'Button' });
    assert.ok(expanded.length <= 1, 'Shader editor collapse control must be unique');
    if (expanded.length) await client.click(expanded[0]);
    await client.waitFor(async () => {
      const collapsed = await client.byLabel('Expand SHADER EDITOR', { role: 'Button' });
      assert.ok(collapsed.length <= 1, 'Shader editor expand control must be unique');
      return collapsed.length === 1;
    }, { description: 'Shader editor collapsed before inspecting uniforms' });
  };
  let pointerScrollTool;
  const scrollInspector = async (deltaY) => {
    const scroll = await client.find({ id: 'Inspector::properties-scroll' });
    if (!pointerScrollTool) {
      const tools = (await client.tools()).tools.filter((tool) => {
        const properties = tool.inputSchema?.properties;
        return properties?.windowHandle && properties.position && properties.deltaY;
      });
      assert.equal(tools.length, 1, 'MCP must expose one positioned pointer-scroll tool');
      pointerScrollTool = tools[0].name;
    }
    // The left padding belongs to the outer Inspector, not the nested code
    // editor or a form control that can consume a center-targeted wheel event.
    const position = { x: (scroll.absolutePosition?.x ?? 0) + 4, y: (scroll.absolutePosition?.y ?? 0) + scroll.size.height / 2 };
    await client.value(pointerScrollTool, { windowHandle: client.windowId, position, deltaX: 0, deltaY });
  };
  const headers = async () => {
    // Image uploads may replace node rows between query_handles and properties.
    // Read node identity and properties in one UI-thread tree snapshot instead.
    const tree = await client.tree();
    assert.ok(!tree.truncated, 'Node inspection needs a complete UI tree snapshot');
    return (tree.elements ?? []).filter((element) => element.accessibleRole === 'Button'
      && /^Select node /.test(element.accessibleLabel ?? ''));
  };
  const header = async (id) => {
    const matches = (await headers()).filter((element) => nodeState(element).id === id);
    assert.equal(matches.length, 1, `Expected exactly one UI node ${id}`);
    return matches[0];
  };
  const findUniform = async (name) => {
    const selector = { role: 'TextInput', visible: false };
    const label = `Uniform ${name}`;
    let matches = await client.byLabel(label, selector);
    assert.ok(matches.length <= 1, `Expected a unique ${label}, found ${matches.length}`);
    if (matches.length === 1) return matches[0];
    // Selection can retain the previous inspector offset. Start a bounded,
    // genuine scroll traversal at the top rather than skipping clipped rows.
    await scrollInspector(10_000);
    for (let attempt = 0; attempt <= 12; attempt++) {
      abort();
      matches = await client.byLabel(label, selector);
      assert.ok(matches.length <= 1, `Expected a unique ${label}, found ${matches.length}`);
      if (matches.length === 1) return matches[0];
      if (attempt < 12) await scrollInspector(-200);
    }
    throw new Error(`${label} remained inaccessible after traversing the Inspector with 12 real scroll events`);
  };
  publish();
  try {
    abort();
    const catalogPath = new URL('../src/shaders/catalog.json', import.meta.url);
    const catalog = JSON.parse(await readFile(catalogPath, 'utf8')).groups.flatMap((group) => group.items.map((item) => ({ ...item, category: group.category })));
    assert.equal(catalog.length, 34, 'Update the explicit E2E catalog coverage contract when catalog entries change');
    report.caseDefinitions = catalog.map((entry) => ({
      id: `shader-${entry.label.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/-$/, '')}`,
      title: `${entry.category} · ${entry.label}`, description: '真实图片输入、原生连接和独立 GPU 像素验证', screenshotPrefixes: [],
    }));
    publish();
    const prepared = await prepareShaderFixtures(directory);
    assert.equal(prepared.length, catalog.length, 'The full shader catalog must produce one fixture per entry');
    assert.equal(new Set(prepared.map((entry) => entry.id)).size, prepared.length, 'Duplicate catalog case IDs');
    assert.deepEqual(prepared.map((entry) => entry.label).sort(), catalog.map((entry) => entry.label).sort(), 'Fixture coverage must equal the complete canonical catalog, including both custom entries');
    const cases = prepared.map((entry) => {
      const identity = catalog.find((item) => item.label === entry.label);
      assert.equal(entry.category, identity.category, `Catalog category differs for ${entry.label}`);
      return { ...entry, source: identity.code, catalogIdentity: identity };
    });
    caseDefinitions = cases.map((entry) => ({ id: entry.id, title: entry.title, description: entry.description, screenshotPrefixes: [entry.id] }));
    report.caseDefinitions = caseDefinitions;
    report.catalogCoverage = { expected: catalog.length, prepared: cases.length, ids: cases.map((entry) => entry.id), origin: 'src/shaders/catalog.json + prepareShaderFixtures metadata', labels: cases.map((entry) => entry.label) };
    publish();
    for (const [index, entry] of cases.entries()) {
      abort();
      client = undefined;
      const step = { name: entry.id, title: entry.title, startedAt: new Date().toISOString(), screenshots: [], artifacts: [], details: { category: entry.category, description: entry.description, uniforms: entry.uniforms, textureInputs: entry.textureInputs, projectPath: entry.projectPath, phases: [] } };
      report.steps.push(step);
      report.shaderCases.push({ id: entry.id, label: entry.label, category: entry.category, projectPath: entry.projectPath, uniforms: entry.uniforms, textureInputs: entry.textureInputs, expectedPath: entry.expectedPath });
      publish();
      console.log(`[slint-shaders] ${index + 1}/${cases.length} 开始：${step.title}`);
      try {
        let graph, shader, renderer, images, samplers;
        await phase(step, 'fixtures', '核对真实图片、catalog 源码、参数与无连接预载项目', async () => {
          const project = JSON.parse(await readFile(entry.projectPath, 'utf8'));
          graph = project.graph;
          assert.ok(graph && Array.isArray(graph.nodes) && Array.isArray(graph.edges), 'Fixture must be a real SDK graph project');
          assert.equal(graph.edges.length, 0, 'Fixture must not pre-connect the graph');
          const shaders = graph.nodes.filter((node) => node.type === 'shader');
          const renderers = graph.nodes.filter((node) => node.type === 'renderer');
          images = graph.nodes.filter((node) => node.type === 'input');
          assert.equal(shaders.length, 1);
          assert.equal(renderers.length, 1);
          [shader] = shaders;
          [renderer] = renderers;
          assert.equal(shader.data.label, entry.label);
          assert.equal(shader.data.shaderCode, entry.source, 'Fixture must retain exact catalog source');
          assert.equal(shader.data.templateName ?? null, entry.catalogIdentity.templateName ?? null, 'Fixture template name differs from the canonical catalog');
          assert.equal(shader.data.shaderTemplateId ?? null, entry.catalogIdentity.shaderTemplateId ?? null, 'Fixture template ID differs from the canonical catalog');
          assert.deepEqual(shader.data.uniforms, entry.uniforms, 'Fixture uniform values must match metadata');
          assert.equal(shader.data.width, 64);
          assert.equal(shader.data.height, 64);
          assert.equal(shader.data.autoSize, false);
          samplers = shader.data.inputs.filter((port) => port.dataType === 'sampler2D' && port.label !== 'previousFrame');
          assert.equal(images.length, samplers.length, 'Each actual sampler requires one real image, generators require none');
          assert.equal(graph.nodes.length, images.length + 2, 'Unexpected fixture nodes');
          const inputEvidence = [];
          for (const [inputIndex, image] of images.entries()) {
            assert.equal(image.data.inputMode, 'image');
            assert.equal(image.data.texFilter, 'nearest');
            assert.equal(image.data.texWrap, 'clamp');
            const fixturePath = entry.fixturePaths[inputIndex];
            assert.ok(fixturePath, `Missing real input PNG for ${image.data.label}`);
            const bytes = await readFile(fixturePath);
            const decoded = decodePng(bytes);
            assert.equal(decoded.width, 64);
            assert.equal(decoded.height, 64);
            assert.equal(image.data.imageDataUrl, `data:image/png;base64,${bytes.toString('base64')}`, 'Project image bytes differ from the real fixture PNG');
            const path = join(directory, `${entry.id}-input-${inputIndex + 1}-${basename(fixturePath)}`);
            if (resolve(path) !== resolve(fixturePath)) await copyFile(fixturePath, path);
            artifact(step, path, true);
            inputEvidence.push({ nodeId: image.id, label: image.data.label, sampler: samplers[inputIndex].label, path, sha256: sha256(bytes), width: decoded.width, height: decoded.height });
          }
          if (inputEvidence.length > 1) assert.equal(new Set(inputEvidence.map((input) => input.sha256)).size, inputEvidence.length, 'Multi-input shaders must use distinct real images');
          const expectedPath = join(directory, `${entry.id}-expected.png`);
          if (resolve(expectedPath) !== resolve(entry.expectedPath)) await copyFile(entry.expectedPath, expectedPath);
          const expected = decodePng(await readFile(expectedPath));
          assert.equal(expected.width, 64);
          assert.equal(expected.height, 64);
          artifact(step, expectedPath, true);
          artifact(step, entry.projectPath);
          return { projectPath: resolve(entry.projectPath), nodeCount: graph.nodes.length, connectionCount: 0, shaderId: shader.id, rendererId: renderer.id, sourceSha256: sha256(entry.source), templateName: shader.data.templateName, shaderTemplateId: shader.data.shaderTemplateId, uniforms: entry.uniforms, inputs: inputEvidence, expectedPath, generator: images.length === 0, inputExplanation: images.length ? `${images.length} real PNG sampler input(s)` : 'No sampler input: this catalog shader is a generator; no fake image edge is added' };
        });
        await phase(step, 'open-project', '独立重启并等待真实图片解码完成，确认路径、节点和停止状态', async () => {
          abort();
          const url = await restart({ project: entry.projectPath });
          abort();
          assert.equal(typeof url, 'string', 'restart must return an MCP endpoint');
          client = await createClient(url, { timeoutMs, signal });
          await client.useWindow();
          const tree = await client.tree();
          assert.ok(!tree.debugInfoMissing, 'App needs Slint debug information');
          const observed = await rootStatus({ Playback: 'stopped', Busy: 'false', Nodes: graph.nodes.length, Connections: 0 });
          assert.equal(resolve(observed['Project path'] || ''), resolve(entry.projectPath), 'App loaded a different project path');
          assert.equal(observed['Renderer image'], 'unavailable', 'A preloaded stopped graph must not render');
          const nodes = (await headers()).map(nodeState);
          assert.deepEqual(nodes.map((node) => node.id).sort(), graph.nodes.map((node) => node.id).sort(), 'UI node IDs differ from the fixture graph');
          for (const node of graph.nodes) assert.equal(nodes.find((item) => item.id === node.id).label, node.data.label);
          await capture(step, 'opened-stopped');
          return { endpoint: url, status: observed, nodes };
        });
        await phase(step, 'source-uniform-identity', '选择 Shader 并核对 catalog 源码、只读属性和实际 UI 参数', async () => {
          await client.activate(await header(shader.id));
          const source = await client.find({ label: 'Shader source', role: 'TextInput' });
          assert.equal(source.accessibleValue, entry.source, 'Displayed shader source differs from the original catalog source');
          const expectedReadOnly = Boolean(shader.data.shaderTemplateId);
          assert.equal(Boolean(source.accessibleReadOnly), expectedReadOnly, 'Catalog/custom source read-only identity differs');
          await capture(step, 'source-identity');
          if (Object.keys(entry.uniforms).length) await collapseShaderEditor();
          const uniformEvidence = [];
          for (const [name, value] of Object.entries(entry.uniforms)) {
            const control = await findUniform(name);
            assertUniformValue(control.accessibleValue, value, name);
            uniformEvidence.push({ name, expected: value, actual: control.accessibleValue });
          }
          await capture(step, 'source-and-uniforms');
          return { sourceSha256: sha256(source.accessibleValue), sourceBytes: Buffer.byteLength(source.accessibleValue), readOnly: Boolean(source.accessibleReadOnly), uniforms: uniformEvidence };
        });
        await phase(step, 'connect-graph', 'Fit 后真实点击各图片输出、Shader sampler 和 Renderer 输入', async () => {
          await click('Fit graph in view');
          const connections = [];
          for (const [inputIndex, image] of images.entries()) {
            const outputs = await client.byLabel(new RegExp(`^Output ${escapeRegex(image.data.label)} `), { role: 'Button' });
            assert.equal(outputs.length, 1, 'Each image must expose exactly one texture output');
            await client.click(outputs[0]);
            await client.click(await button(`Input ${shader.data.label} ${samplers[inputIndex].label}`));
            await rootStatus({ Playback: 'stopped', Busy: 'false', Nodes: graph.nodes.length, Connections: connections.length + 1 });
            connections.push({ source: image.id, output: outputs[0].accessibleLabel, target: shader.id, input: samplers[inputIndex].label });
          }
          const shaderOutputs = await client.byLabel(new RegExp(`^Output ${escapeRegex(shader.data.label)} `), { role: 'Button' });
          assert.equal(shaderOutputs.length, 1, 'Each catalog shader must have one fragment output');
          const rendererInputs = await client.byLabel(new RegExp(`^Input ${escapeRegex(renderer.data.label)} `), { role: 'Button' });
          assert.equal(rendererInputs.length, 1);
          await client.click(shaderOutputs[0]);
          await client.click(rendererInputs[0]);
          connections.push({ source: shader.id, output: shaderOutputs[0].accessibleLabel, target: renderer.id, input: rendererInputs[0].accessibleLabel });
          const observed = await rootStatus({ Playback: 'stopped', Busy: 'false', Nodes: graph.nodes.length, Connections: images.length + 1 });
          assert.equal(observed['Renderer image'], 'unavailable');
          const edges = await client.byLabel(/^Select connection /, { role: 'Button' });
          assert.equal(edges.length, images.length + 1, 'Actual visible edge count differs');
          await client.activate(await header(renderer.id));
          await capture(step, 'connected-stopped');
          return { status: observed, connections, visibleEdges: edges.map((edge) => edge.accessibleLabel) };
        });
        await phase(step, 'play', '真实点击 PLAY，等待 Renderer 及每个节点真实预览可用', async () => {
          await click('PLAY');
          const observed = await rootStatus({ Playback: 'playing', Busy: 'false', 'Renderer image': 'available', Nodes: graph.nodes.length, Connections: images.length + 1 });
          await client.waitFor(async () => {
            const nodes = (await headers()).map(nodeState);
            return nodes.length === graph.nodes.length && nodes.every((node) => node.Preview === 'available') && nodes;
          }, { description: 'Every image, shader and Renderer node thumbnail available' });
          const preview = await client.waitFor(async () => {
            const found = await client.byLabel('Renderer preview');
            return found.length === 1 && found[0].accessibleDescription === 'Available' && found[0];
          }, { description: 'Renderer preview Available' });
          const firstPng = await capture(step, 'playing-window');
          let feedback;
          if (entry.category === 'FEEDBACK' || entry.source.includes('previousFrame')) {
            const window = await client.windowProperties();
            const first = rendererPixels(firstPng, preview, window, 64, 64);
            feedback = await client.waitFor(async () => {
              const nextPng = await capture(step, 'feedback-later-window');
              const next = rendererPixels(nextPng, await client.find({ label: 'Renderer preview' }), window, 64, 64);
              let changedPixels = 0;
              for (let offset = 0; offset < first.pixels.length; offset += 4) {
                if (Math.max(...[0, 1, 2].map((channel) => Math.abs(first.pixels[offset + channel] - next.pixels[offset + channel]))) >= 2) changedPixels++;
              }
              return changedPixels >= 4 && { changedPixels, totalPixels: 4096, firstSha256: sha256(first.pixels), laterSha256: sha256(next.pixels), atLeastTwoDistinctRenderedFrames: true };
            }, { description: 'Feedback produces at least two distinct real rendered frames' });
          }
          return { status: observed, preview: { position: preview.absolutePosition, size: preview.size }, nodes: (await headers()).map(nodeState), feedback };
        });
        await phase(step, 'pause-pixel-oracle', '真实 PAUSE 冻结输出，截图采样并与独立 CPU 图像 oracle 比较', async () => {
          await click('PAUSE');
          const observed = await rootStatus({ Playback: 'paused', Busy: 'false', 'Renderer image': 'available' });
          await client.activate(await header(shader.id));
          if (Object.keys(entry.uniforms).length) await collapseShaderEditor();
          const appliedUniforms = [];
          for (const [name, value] of Object.entries(entry.uniforms)) {
            const control = await findUniform(name);
            assertUniformValue(control.accessibleValue, value, name);
            appliedUniforms.push({ name, expected: value, actual: control.accessibleValue });
          }
          await client.activate(await header(renderer.id));
          const preview = await client.find({ label: 'Renderer preview' });
          assert.equal(preview.accessibleDescription, 'Available');
          const png = await capture(step, 'output-window-paused');
          const actual = rendererPixels(png, preview, await client.windowProperties(), 64, 64);
          const actualPath = join(directory, `${entry.id}-actual-renderer.png`);
          await writeFile(actualPath, encodePng(actual));
          artifact(step, actualPath, true);
          const metrics = await compareShaderOutput(entry, actual);
          step.details.pixels = { metrics, roi: actual.roi, actualPath, expectedPath: entry.expectedPath };
          const metricsPath = join(directory, `${entry.id}-pixel-metrics.json`);
          await saveJson(metricsPath, step.details.pixels);
          artifact(step, metricsPath);
          assert.equal(metrics.verified, true, `Renderer pixels failed the CPU oracle: ${JSON.stringify(metrics)}`);
          return { status: observed, appliedUniforms, pixels: step.details.pixels };
        });
        await phase(step, 'stop', '真实点击 STOP 并确认最终节点与连接计数保持', async () => {
          await click('STOP');
          const observed = await rootStatus({ Playback: 'stopped', Busy: 'false', Nodes: graph.nodes.length, Connections: images.length + 1 });
          await capture(step, 'stopped');
          return observed;
        });
        step.passed = true;
      } catch (error) {
        step.error = errorText(error);
        if (signal?.aborted) throw error;
        step.passed = false;
        step.diagnosticErrors = [];
        if (client) {
          try {
            const path = join(directory, `${entry.id}-failure-tree.json`);
            await client.tree({ path });
            artifact(step, path);
          } catch (failure) { step.diagnosticErrors.push(`tree: ${errorText(failure)}`); }
          try { await capture(step, 'failure-window'); }
          catch (failure) { step.diagnosticErrors.push(`screenshot: ${errorText(failure)}`); }
        }
        // A normal assertion failure is isolated by the next case's restart.
        // App crashes/timeouts abort through the runner-owned global signal.
        abort();
      } finally {
        step.finishedAt = new Date().toISOString();
        const path = join(directory, `${entry.id}-evidence.json`);
        artifact(step, path);
        await saveJson(path, step);
        publish();
        console.log(`[slint-shaders] ${index + 1}/${cases.length} ${step.passed === true ? '通过' : step.passed === false ? '失败' : '中断'}：${step.title}`);
      }
    }
    report.passed = report.steps.length === cases.length && report.steps.every((step) => step.passed === true);
    if (!report.passed) {
      const failed = report.steps.filter((step) => step.passed === false);
      throw new Error(`${failed.length}/${cases.length} shader cases failed: ${failed.map((step) => step.name).join(', ')}`);
    }
    return report;
  } catch (error) {
    report.error = errorText(error);
    error.scenarioResult = report;
    throw error;
  } finally {
    report.finishedAt = new Date().toISOString();
    publish();
    await saveJson(join(directory, 'result.json'), report);
  }
}
