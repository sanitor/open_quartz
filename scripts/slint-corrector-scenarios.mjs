import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { createClient } from './slint-ui-client.mjs';
import { decodePng, parseStatus } from './slint-ui-scenarios.mjs';

export async function runCorrectorScenarios({ endpoint, outputDir, projectPath, restart, signal, onProgress, smoke = false }) {
  const cases = [
    ['native-corrector-factory-connect', 'Create and connect the native Color Corrector'],
    ['native-corrector-exposure-bypass-history', 'Exposure, stage bypass and undo/redo update native pixels'],
    ['native-corrector-cancel-tabs-wheel-roundtrip', 'Cancel a draft, navigate palettes, drag a wheel and reopen'],
  ];
  const caseDefinitions = (smoke ? cases.slice(0, 2) : cases)
    .map(([id, title]) => ({ id, title, description: title, screenshotPrefixes: [] }));
  const report = { suite: 'corrector', passed: false, steps: [], screenshots: [], caseDefinitions };
  let client = await createClient(endpoint, { signal }); await client.useWindow();
  let scrollTool;
  const button = label => client.find({ label, role: 'Button' });
  const click = async label => client.click(await (typeof label === 'string' && /^(Corrector |Cancel Corrector|Apply Color|Reset )/.test(label) ? reveal(label, 'Button') : button(label)));
  const state = async () => parseStatus((await client.properties(client.rootHandle)).accessibleDescription);
  const settle = async () => client.waitFor(async () => (await state())['Inspector dirty'] === 'false', { description: 'Corrector candidate accepted' });
  const header = label => client.find({ label: `Select node ${label}`, role: 'Button', visible: false });
  const select = async label => client.activate(await header(label));
  const reveal = async (label, role = 'TextInput') => {
    if (!scrollTool) {
      const tools = (await client.tools()).tools.filter(tool => tool.inputSchema?.properties?.windowHandle && tool.inputSchema.properties.position && tool.inputSchema.properties.deltaY);
      assert.equal(tools.length, 1); scrollTool = tools[0].name;
    }
    const scroll = async deltaY => {
      const view = await client.find({ id: 'Inspector::properties-scroll' });
      await client.value(scrollTool, { windowHandle: client.windowId, position: { x: view.absolutePosition.x + 4, y: view.absolutePosition.y + view.size.height / 2 }, deltaX: 0, deltaY });
    };
    await scroll(10000);
    for (let attempt = 0; attempt < 15; attempt++) {
      const matches = await client.byLabel(label, { role, visible: false });
      const view = await client.find({ id: 'Inspector::properties-scroll' });
      if (matches.length === 1) {
        const item = matches[0], y = item.absolutePosition.y + item.size.height / 2;
        if (item.size.height > 0 && y > view.absolutePosition.y + 4 && y < view.absolutePosition.y + view.size.height - 4) return item;
      }
      await scroll(-150);
    }
    throw new Error(`Could not reveal Corrector control ${label}`);
  };
  const numeric = () => reveal('Corrector Exposure (stops)');
  const edit = async value => { const input = await numeric(); await client.click(input); await client.setValue(input, String(value)); await client.key('\n'); await settle(); };
  const saved = async () => { await click('FILE ▾'); await click('SAVE'); await client.waitFor(async () => /Saved /.test((await state())['IO status'] || ''), { description: 'saved accepted typed config' }); return JSON.parse(await readFile(projectPath, 'utf8')); };
  const step = async (id, action) => {
    const definition = caseDefinitions.find(entry => entry.id === id);
    assert.ok(definition, `Unexpected Corrector case ${id}`);
    const entry = { name: id, title: definition.title }; report.steps.push(entry); onProgress?.(report);
    try { entry.details = await action(); entry.passed = true; } catch (error) { entry.passed = false; throw error; }
  };
  async function gray(name, expected) {
    await client.waitFor(async () => (await state())['Renderer image'] === 'available', { description: 'real GPU renderer preview' });
    return client.waitFor(async () => {
    const preview = await client.find({ label: 'Renderer preview' });
    const window = await client.windowProperties();
    const path = join(outputDir, `${name}.png`), png = decodePng(await client.screenshot(path)); report.screenshots.push(path);
    const scale = png.width / (window.size.width / (window.scaleFactor || 1));
    const x = Math.round((preview.absolutePosition.x + preview.size.width / 2) * scale);
    const y = Math.round((preview.absolutePosition.y + preview.size.height / 2) * scale);
    const rgb = [0, 0, 0]; let count = 0;
    for (let dy = -8; dy <= 8; dy++) for (let dx = -8; dx <= 8; dx++) {
      const offset = ((y + dy) * png.width + x + dx) * png.channels;
      for (let c = 0; c < 3; c++) rgb[c] += png.pixels[offset + c]; count++;
    }
    if (rgb.some(value => Math.abs(value / count - expected) >= 5)) return false;
    return rgb.map(value => value / count);
    }, { description: `${name}: shared GPU RGB ${expected}` });
  }
  try {
    const fixture = { version: '0.4.0', name: 'Native Corrector oracle', createdAt: '', updatedAt: '', graph: {
      nodes: [
        { id: 'gray', type: 'shader', position: { x: 0, y: 0 }, data: { type: 'shader', label: 'Oracle gray', shaderCode: '@fragment fn main() -> @location(0) vec4f { return vec4f(0.25,0.25,0.25,1.0); }', inputs: [], outputs: [{ id: 'out', label: 'fragColor', dataType: 'vec4', direction: 'output' }], uniforms: {}, autoSize: false, width: 64, height: 64 } },
        { id: 'screen', type: 'renderer', position: { x: 520, y: 0 }, data: { type: 'renderer', label: 'Oracle screen', shaderCode: '', inputs: [{ id: 'in', label: 'image', dataType: 'sampler2D', direction: 'input' }], outputs: [], uniforms: {} } },
      ], edges: [] } };
    await writeFile(projectPath, JSON.stringify(fixture));
    client = await createClient(await restart({ project: projectPath }), { signal }); await client.useWindow();
    let correctorId, correctorLabel;
    await step('native-corrector-factory-connect', async () => {
      await click('SHADER ▾'); await click(/^COLOR\s+›$/); await click('COLOR CORRECTOR');
      await client.waitFor(async () => (await state()).Nodes === '3', { description: 'native factory node created' });
      const node = await client.find({ label: /^Select node color_corrector_/, role: 'Button', visible: false });
      correctorId = /Node id: ([^;]+)/.exec(node.accessibleDescription)[1];
      correctorLabel = node.accessibleLabel.slice('Select node '.length);
      await client.activate(node);
      const ports = async (label, output) => {
        const found = await client.byLabel(new RegExp(`^${output ? 'Output' : 'Input'} ${label} `), { role: 'Button', visible: false }); assert.equal(found.length, 1); return found[0];
      };
      await client.activate(await ports('Oracle gray', true)); await client.activate(await ports(correctorLabel, false));
      await client.waitFor(async () => (await state()).Connections === '1', { description: 'native source → Corrector connection accepted' });
      await client.activate(await ports(correctorLabel, true)); await client.activate(await ports('Oracle screen', false));
      await client.waitFor(async () => (await state()).Connections === '2', { description: 'native Corrector → Renderer connection accepted' });
      await select('Oracle screen'); await click('PLAY');
      await gray('corrector-neutral', 64);
      await select(correctorLabel);
      const file = await saved(), nodeData = file.graph.nodes.find(node => node.id === correctorId);
      assert.equal(nodeData.type, 'colorcorrector'); assert.equal(nodeData.data.shaderCode, '');
      return { nodeId: correctorId };
    });
    await step('native-corrector-exposure-bypass-history', async () => {
      await edit(1); const expected = Math.round((1.055 * Math.pow(2 * Math.pow((0.25 + 0.055) / 1.055, 2.4), 1 / 2.4) - 0.055) * 255);
      await gray('corrector-exposure-one', expected);
      await click('UNDO'); await settle(); await gray('corrector-undo', 64);
      await click('REDO'); await settle(); await gray('corrector-redo', expected);
      await client.click(await reveal('Primaries enabled', 'Checkbox')); await settle(); await gray('corrector-local-bypass', 64);
      const file = await saved(); assert.equal(file.graph.nodes.find(node => node.id === correctorId).data.colorCorrector.primaries.exposure, 1);
      await client.click(await reveal('Primaries enabled', 'Checkbox')); await settle();
      return { expected, oracle: 'independent sRGB decode → linear exposure ×2 → sRGB encode' };
    });
    if (!smoke) await step('native-corrector-cancel-tabs-wheel-roundtrip', async () => {
      const input = await numeric(); await client.click(input); await client.setValue(input, '2');
      await click('Cancel Corrector gesture'); await settle(); assert.equal(Number((await numeric()).accessibleValue), 1);
      await click('Corrector palette HDR Zones'); await click('Corrector palette Primaries');
      await click('Corrector primaries Offset');
      const wheel = await reveal('Offset color wheel; arrow keys adjust, Escape cancels', 'Button');
      await client.drag(wheel, { x: wheel.absolutePosition.x + wheel.size.width / 2 + 15, y: wheel.absolutePosition.y + wheel.size.height / 2 + 5 }); await settle();
      await click('UNDO'); await settle();
      await click('Corrector primaries Basic'); assert.equal(Number((await numeric()).accessibleValue), 1);
      const file = await saved(), config = file.graph.nodes.find(node => node.id === correctorId).data.colorCorrector;
      assert.deepEqual(config.primaries.offset, [0, 0, 0]);
      client = await createClient(await restart({ project: projectPath }), { signal }); await client.useWindow();
      await select(correctorLabel); assert.equal(Number((await numeric()).accessibleValue), 1);
      return { persisted: config };
    });
    assert.deepEqual(report.steps.map(step => step.name), caseDefinitions.map(entry => entry.id));
    report.passed = true; return report;
  } catch (error) {
    report.error = error.stack || String(error);
    report.diagnosticErrors = [];
    for (const [name, capture] of [
      ['status', async () => writeFile(join(outputDir, 'failure-status.json'), JSON.stringify(await state(), null, 2))],
      ['tree', async () => client.tree({ path: join(outputDir, 'failure-tree.json'), maxElements: 4000 })],
      ['screenshot', async () => client.screenshot(join(outputDir, 'failure.png'))],
    ]) {
      try { await capture(); } catch (failure) { report.diagnosticErrors.push(`${name}: ${failure.message}`); }
    }
    error.scenarioResult = report; throw error;
  }
}
