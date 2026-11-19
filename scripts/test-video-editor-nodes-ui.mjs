#!/usr/bin/env node
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { createServer } from 'node:net';
import { setTimeout as delay } from 'node:timers/promises';
import { SlintUiClient } from './slint-ui-client.mjs';
import { decodePng } from './slint-ui-scenarios.mjs';
import { writeHtmlReport } from './slint-ui-report.mjs';

// Run against an integrated ui-automation binary. No project/graph fixture is
// injected: every saved node, edge and parameter originates in the native UI.
const output = resolve(process.env.VIDEO_EDITOR_NODES_TEST_OUTPUT || `test-results/clip-pipeline/nodes-ui-${new Date().toISOString().replaceAll(/[:.]/g, '-')}`);
const binary = resolve(process.argv[2] || 'apps/open_quartz_video_editor/target/debug/open_quartz_video_editor');
const ffmpeg = resolve(process.env.OPEN_QUARTZ_FFMPEG_PATH || 'apps/open_quartz_slint/runtime/ffmpeg');
const project = join(output, 'clip-pipelines.oqvideo.json');
await mkdir(output, { recursive: true });
const report = { passed: false, binary, project, steps: [], screenshots: [], launches: [] };
const server = createServer();
await new Promise(accept => server.listen(0, '127.0.0.1', accept));
const serverPort = server.address().port;
await new Promise(accept => server.close(accept));
let child, client, exited, stdout = '', stderr = '', scrollTool;
let originalData, savedData, customId, invertId;
const GREEN = '@group(0) @binding(0) var image: texture_2d<f32>; @group(0) @binding(1) var imageSampler: sampler; @fragment fn main(@location(0) uv: vec2f) -> @location(0) vec4f { let source=textureSample(image,imageSampler,uv); return vec4f(0.0, 1.0, 0.0, 1.0); }';
const GAIN = '@group(0) @binding(0) var image: texture_2d<f32>; @group(0) @binding(1) var imageSampler: sampler; @group(0) @binding(2) var<uniform> gain: f32; // Green level. Default 1\n@fragment fn main(@location(0) uv: vec2f) -> @location(0) vec4f { let source=textureSample(image,imageSampler,uv); return vec4f(0.0, gain, 0.0, 1.0); }';
const RENAMED = 'Green grade';
const BAD = '@fragment fn main( this is deliberately invalid WGSL';
const state = async () => (await client.properties(client.rootHandle)).accessibleDescription;
const waitState = fragment => client.waitFor(async () => (await state()).includes(fragment), { description: fragment });
const escape = text => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

async function launch(path) {
  stdout = ''; stderr = '';
  child = spawn(binary, path ? ['--project', path] : [], {
    env: { ...process.env, SLINT_MCP_PORT: String(serverPort), OPEN_QUARTZ_UI_CLOSE_STDIN: '1' },
    stdio: ['pipe', 'pipe', 'pipe'],
  });
  child.stdout.on('data', data => stdout += data);
  child.stderr.on('data', data => stderr += data);
  exited = new Promise((accept, reject) => {
    child.once('error', reject);
    child.once('close', (code, signal) => accept({ code, signal }));
  });
  client = new SlintUiClient(`http://127.0.0.1:${serverPort}/mcp`);
  await client.waitFor(async () => {
    try { await client.init(); await client.useWindow(); return true; } catch { return false; }
  }, { description: 'real native video editor window' });
}
async function button(label, { visible = true } = {}) {
  for(let retry=0;retry<6;retry++){
    try{
      const items=await client.byLabel(label,{role:'Button',visible});
      assert(items.length>0,`Missing button ${label}`);
      assert(items.length===1||label==='IMPORT',`Ambiguous button ${label}`);
      return items[0];
    }catch(error){if(!/destroyed|stale/i.test(error.message)||retry===5)throw error;await delay(30);}
  }
}
async function click(label) { await client.click(await button(label)); }
async function activateVisible(label) { await client.activate(await button(label)); }
async function activate(label) { await client.activate(await button(label, { visible: false })); }
async function close() {
  await click('Close window');
  const exit = await Promise.race([exited, delay(10000).then(() => { throw Error('Native window failed to close normally'); })]);
  assert.deepEqual(exit, { code: 0, signal: null });
  report.launches.push({ ...exit, log: `application-${report.launches.length + 1}.log` });
  await writeFile(join(output, report.launches.at(-1).log), stdout + stderr);
  child = undefined;
}
async function fileAction(action, path, submit) {
  await click(action);
  await client.setValue(await client.find({ label: 'File path', role: 'TextInput' }), path);
  const matches = await client.byLabel(submit, { role: 'Button' });
  assert(matches.length, `Missing file submission ${submit}`);
  await client.click(matches.at(-1));
}
async function save(first = false) {
  if (first) await fileAction('SAVE AS', project, 'Save project'); else await click('SAVE');
  await waitState('Project saved');
  return JSON.parse(await readFile(project, 'utf8')).data;
}
async function headers() { return client.byLabel(/^Select node /, { role: 'Button', visible: false }); }
function nodeState(element) {
  const description = element.accessibleDescription || '';
  const id = /Node id: ([^;]+)/.exec(description)?.[1];
  const x = Number(/World x: ([^;]+)/.exec(description)?.[1]);
  const y = Number(/World y: ([^;]+)/.exec(description)?.[1]);
  assert(id && Number.isFinite(x) && Number.isFinite(y), `Missing graph node state: ${description}`);
  return { id, x, y, label: element.accessibleLabel.slice('Select node '.length) };
}
async function header(id) {
  for(let retry=0;retry<6;retry++){
    try{
      const matches=(await headers()).filter(element=>nodeState(element).id===id);
      if(matches.length===1)return matches[0];
      if(matches.length>1)throw Error(`Expected exactly one shared GraphView node ${id}`);
    }catch(error){if(!/destroyed|stale/i.test(error.message))throw error;}
    await delay(30);
  }
  throw Error(`Shared GraphView node ${id} did not settle`);
}
async function selectNode(id) {
  await client.activate(await header(id));
  await client.waitFor(async () => {
    const inputs = await client.byLabel('Node label', { role: 'TextInput', visible: false });
    if (!inputs.length) return true;
    return inputs[0].accessibleValue === nodeState(await header(id)).label;
  }, { description: `Inspector selection for ${id}` });
}
async function waitNodeCount(count) {
  return client.waitFor(async () => (await headers()).length === count, { description: `${count} actual shared graph nodes` });
}
async function oneNewNode(before) {
  return client.waitFor(async () => {
    const nodes = (await headers()).map(nodeState).filter(node => !before.has(node.id));
    return nodes.length === 1 && nodes[0];
  }, { description: 'new committed pipeline node' });
}
async function addEffect(label) {
  const before = new Set((await headers()).map(element => nodeState(element).id));
  // Catalog rows are real standard buttons; accessibility activation also works
  // for a row clipped by the short, scrollable effect library.
  await activateVisible(label);
  return oneNewNode(before);
}
async function selectClip(name) {
  await click(`Pipeline for ${name}`);
  await client.waitFor(async()=>{try{return (await headers()).some(element=>nodeState(element).id==='video');}catch{return false;}},{description:`${name} graph published`});
}
async function port(elementLabel, outputPort) {
  for(let retry=0;retry<6;retry++){
    try{
      const items=await client.byLabel(new RegExp(`^${outputPort?'Output':'Input'} ${escape(elementLabel)} `),{role:'Button',visible:false});
      if(items.length===1)return items[0];
      if(items.length>1)throw Error(`Expected one ${outputPort?'output':'input'} port on ${elementLabel}`);
    }catch(error){if(!/destroyed|stale/i.test(error.message))throw error;}
    await delay(30);
  }
  throw Error(`Missing ${outputPort?'output':'input'} port on ${elementLabel}`);
}
async function scrollInspector(deltaY) {
  const viewport = await client.find({ id: 'Inspector::properties-scroll' });
  if (!scrollTool) {
    const tools = (await client.tools()).tools.filter(tool => {
      const properties = tool.inputSchema?.properties;
      return properties?.windowHandle && properties.position && properties.deltaY;
    });
    assert.equal(tools.length, 1, 'MCP must provide one real positioned pointer-scroll tool');
    scrollTool = tools[0].name;
  }
  await client.value(scrollTool, {
    windowHandle: client.windowId,
    position: { x: viewport.absolutePosition.x + 4, y: viewport.absolutePosition.y + viewport.size.height / 2 },
    deltaX: 0, deltaY,
  });
}
async function revealInspector(label, role = 'TextInput') {
  await scrollInspector(10000);
  for(let attempt=0;attempt<12;attempt++){
    let inputs=[];
    try{inputs=await client.byLabel(label,{role,visible:false});}catch(error){if(!/destroyed|stale/i.test(error.message))throw error;}
    if(inputs.length===1){
      const input=inputs[0],viewport=await client.find({id:'Inspector::properties-scroll'});
      const center=input.absolutePosition.y+input.size.height/2;
      if(input.size.height>0&&center>viewport.absolutePosition.y+4&&center<viewport.absolutePosition.y+viewport.size.height-4)return input;
      const delta=viewport.absolutePosition.y+viewport.size.height/2-center;
      await scrollInspector(Math.abs(delta)<10?-100:delta);
    }else{await scrollInspector(-600);}
  }
  throw Error(`Inspector did not reveal ${label} after real scrolling`);
}
async function expandShader() {
  const controls = await client.byLabel('Expand SHADER EDITOR', { role: 'Button' });
  if (controls.length) await client.activate(controls[0]);
}
async function collapseSection(title) {
  for(let retry=0;retry<5;retry++){
    try{
      const controls=await client.byLabel(`Collapse ${title}`,{role:'Button',visible:false});
      if(!controls.length)return;
      await client.activate(controls[0]);return;
    }catch(error){if(!/destroyed|stale/i.test(error.message))throw error;await delay(30);}
  }
}
async function setShader(code) {
  await expandShader();
  const source = await revealInspector('Shader source');
  assert.equal(Boolean(source.accessibleReadOnly), false, 'Custom source must be the editable shared CodeEditor');
  await client.setValue(source, code);
  await client.waitFor(async () => (await client.find({ label: 'Shader source', role: 'TextInput', visible: false })).accessibleValue === code, { description: 'real code editor draft' });
  await activate('Apply shader');
}
async function previewMatches(png, color) {
  const preview = await client.find({ label: 'Video preview', role: 'Image' });
  const image = decodePng(png);
  const window = await client.windowProperties();
  const scale = window.scaleFactor || 1;
  const sx = image.width / (window.size.width / scale), sy = image.height / (window.size.height / scale);
  const x = Math.floor((((preview.absolutePosition.x || 0) + preview.size.width / 2)) * sx);
  const y = Math.floor((((preview.absolutePosition.y || 0) + preview.size.height / 2)) * sy);
  assert(x > 15 && y > 15 && x + 15 < image.width && y + 15 < image.height, 'Monitor sample must be inside screenshot');
  let hits = 0;
  for (let dy = -15; dy < 15; dy++) for (let dx = -15; dx < 15; dx++) {
    const at = ((y + dy) * image.width + x + dx) * image.channels;
    const [r, g, b] = image.pixels.subarray(at, at + 3);
    const matches = color === 'red' ? r > 220 && g < 30 && b < 30
      : color === 'blue' ? b > 220 && r < 30 && g < 30
      : color === 'cyan' ? r < 35 && g > 220 && b > 220
      : color === 'green' ? r < 25 && g > 225 && b < 25
      : color === 'magenta' ? r > 225 && g < 25 && b > 225
      : color === 'dim-green' ? r < 25 && g >= 45 && g <= 155 && b < 25
      : false;
    if (matches) hits++;
  }
  return hits >= 890;
}
async function waitColor(color) {
  await client.waitFor(async () => previewMatches(await client.screenshot(), color), { description: `real GPU monitor pixels ${color}` });
}
async function snapshot(name, color) {
  if (color) await waitColor(color);
  const path = join(output, `${name}.png`);
  const png = await client.screenshot(path);
  report.screenshots.push(path);
  if (color) assert(await previewMatches(png, color), `${name}: expected ${color} GPU pixels`);
}
async function step(name, run) {
  const startedAt = new Date().toISOString(), first = report.screenshots.length;
  try {
    const details = await run();
    report.steps.push({ name, passed: true, startedAt, finishedAt: new Date().toISOString(), screenshots: report.screenshots.slice(first), ...(details ? { details } : {}) });
    console.log(`[video-editor-nodes] PASS ${name}`);
  } catch (error) {
    report.steps.push({ name, passed: false, startedAt, finishedAt: new Date().toISOString(), error: error.message, screenshots: report.screenshots.slice(first) });
    throw error;
  }
}
function generate(name, color) {
  const path = join(output, name);
  execFileSync(ffmpeg, ['-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', `color=c=${color}:s=320x180:r=30`, '-vf', 'scale=in_color_matrix=bt601:out_color_matrix=bt709', '-t', '3', '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-colorspace', 'bt709', '-y', path], {
    env: { ...process.env, DYLD_LIBRARY_PATH: '', DYLD_FALLBACK_LIBRARY_PATH: '' },
  });
  return path;
}
function clipFor(data, name) {
  const asset = data.assets.find(asset => asset.name === name);
  assert(asset, `Missing saved asset ${name}`);
  const clips = data.clips.filter(clip => clip.asset_id === asset.id);
  assert.equal(clips.length, 1, `Expected one saved ${name} clip`);
  return clips[0];
}
function graphFor(data, name) { return clipFor(data, name).pipeline.graph; }

try {
  const red = generate('red.mp4', 'red'), blue = generate('blue.mp4', 'blue');
  await step('Create two real hardware-decoded clips through the native UI', async () => {
    await launch(); await waitState('assets 0; clips 0');
    await fileAction('IMPORT', red, 'Import media');
    await waitState('source PTS 0 ns · CPU image bytes 0');
    await click('ADD CLIP'); await waitState('clips 1');
    await fileAction('IMPORT', blue, 'Import media'); await waitState('assets 2');
    await click('Add blue.mp4 to timeline'); await waitState('clips 2');
    await click('FIT TIMELINE');
    await click('Nodes workspace'); await selectClip('red.mp4');
    await waitNodeCount(2); await snapshot('01-nodes-default-red', 'red');
    originalData = await save(true);
    assert.equal(originalData.schema_version, 2);
    assert.deepEqual(originalData.clips.map(clip => [clip.timeline_start, clip.duration]), [[0, 90], [90, 90]]);
  });
  await step('Source and output remain selectable, read-only and non-deletable', async () => {
    for (const id of ['video', 'output']) {
      await selectNode(id);
      const label = await client.query({ label: 'Node label', role: 'TextInput', visible: false });
      if (label.length) assert.equal(Boolean(label[0].accessibleEnabled), false, `${id} label must be protected`);
      assert.equal((await client.byLabel('Delete node', { role: 'Button' })).length, 0);
      await waitNodeCount(2); await snapshot(`02-protected-${id}`, 'red');
    }
    assert.deepEqual(graphFor(await save(), 'red.mp4'), graphFor(originalData, 'red.mp4'));
  });
  await step('The real catalog Invert transforms red GPU video to cyan', async () => {
    await selectNode('video');
    const invert = await addEffect('COLOR / Invert'); invertId = invert.id;
    await waitNodeCount(3); await snapshot('03-invert-red-to-cyan', 'cyan');
    const data = await save(); const graph = graphFor(data, 'red.mp4');
    assert(graph.edges.some(edge => edge.source === 'video' && edge.target === invertId));
    assert(graph.edges.some(edge => edge.source === invertId && edge.target === 'output'));
    assert.deepEqual(graphFor(data, 'blue.mp4'), graphFor(originalData, 'blue.mp4'));
    return { nodeId: invertId };
  });
  await step('Clip switching isolates graphs and uses the same final video viewer', async () => {
    await selectClip('blue.mp4'); await waitNodeCount(2); await snapshot('04-isolated-blue', 'blue');
    await selectClip('red.mp4'); await waitNodeCount(3); await snapshot('05-return-red-cyan', 'cyan');
  });
  await step('Shared custom shader editor applies real constant green output', async () => {
    await selectNode('output');
    const custom = await addEffect('Custom Shader'); customId = custom.id;
    await waitNodeCount(4); await waitColor('cyan');
    await selectNode(customId); await setShader(GREEN);
    await snapshot('06-custom-shader-green', 'green');
    const node = graphFor(await save(), 'red.mp4').nodes.find(node => node.id === customId);
    assert.equal(node.data.shaderCode, GREEN);
    return { nodeId: customId, shader: GREEN };
  });
  await step('Node drag commits layout, and undo/redo restore exact world coordinates', async () => {
    await click('Fit graph in view');
    const item = await header(customId), before = nodeState(item);
    await client.drag(item, { x: item.absolutePosition.x + item.size.width / 2 + 24, y: item.absolutePosition.y + item.size.height / 2 + 12 });
    const after = await client.waitFor(async () => {
      const current = nodeState(await header(customId));
      return (Math.abs(current.x - before.x) > 1 || Math.abs(current.y - before.y) > 1) && current;
    }, { description: 'shared graph drag changes world coordinates' });
    await click('UNDO');
    await client.waitFor(async () => { const current = nodeState(await header(customId)); return Math.abs(current.x - before.x) < 0.01 && Math.abs(current.y - before.y) < 0.01; }, { description: 'undo node layout' });
    await click('REDO');
    await client.waitFor(async () => { const current = nodeState(await header(customId)); return Math.abs(current.x - after.x) < 0.01 && Math.abs(current.y - after.y) < 0.01; }, { description: 'redo node layout' });
    await snapshot('07-layout-redo', 'green');
    return { before, after };
  });
  await step('Downstream effect order, real port reconnection and serial removal change GPU pixels', async () => {
    await selectNode(customId);
    const secondInvert = await addEffect('COLOR / Invert');
    await waitNodeCount(5); await snapshot('08-green-then-invert-magenta', 'magenta');
    await click('Fit graph in view');
    const customLabel=nodeState(await header(customId)).label;
    const outputLabel=nodeState(await header('output')).label;
    await client.activate(await port(customLabel, true));
    await client.activate(await port(outputLabel, false));
    await snapshot('09-port-bypass-green', 'green');
    let graph = graphFor(await save(), 'red.mp4');
    assert(graph.edges.some(edge => edge.source === customId && edge.target === 'output'));
    await click('UNDO'); await snapshot('10-undo-port-reconnect-magenta', 'magenta');
    await selectNode(secondInvert.id); await activate('Delete node');
    await waitNodeCount(4); await snapshot('11-safe-serial-remove-green', 'green');
    await click('UNDO'); await waitNodeCount(5); await waitColor('magenta');
    await click('REDO'); await waitNodeCount(4); await snapshot('12-redo-serial-remove-green', 'green');
    graph = graphFor(await save(), 'red.mp4');
    assert(!graph.nodes.some(node => node.id === secondInvert.id));
    assert(graph.edges.some(edge => edge.source === customId && edge.target === 'output'));
  });
  await step('Bad shader syntax preserves the last valid frame and saved document', async () => {
    await selectNode(customId);
    const before = await save();
    assert.equal((await client.byLabel('Pipeline error')).length, 0);
    await setShader(BAD);
    await client.waitFor(async () => (await client.byLabel('Pipeline error')).length === 1, { description: 'visible real compiler error' });
    assert.equal((await client.find({ label: 'Shader source', role: 'TextInput', visible: false })).accessibleValue, BAD);
    await snapshot('13-rejected-shader-retains-green', 'green');
    await activate('Discard changes');
    await client.waitFor(async () => (await client.byLabel('Pipeline error')).length === 0 && (await client.byLabel('Discard changes', { role: 'Button' })).length === 0, { description: 'discard restores clean valid shader' });
    assert.equal((await client.find({ label: 'Shader source', role: 'TextInput', visible: false })).accessibleValue, GREEN);
    assert.deepEqual(await save(), before, 'Rejected shader must not change the serialized document');
  });
  await step('Uniform edits and label edits use real Inspector commits and one-step history', async () => {
    await setShader(GAIN);
    await client.waitFor(async()=> (await client.byLabel('Apply shader',{role:'Button'})).every(button=>!button.accessibleEnabled),{description:'shader commit completed'});
    await collapseSection('SHADER EDITOR'); await collapseSection('OUTPUT SIZE');
    let gain = await revealInspector('Uniform gain');
    await client.click(gain); await client.setValue(gain,'1'); await client.key('\n');
    await waitColor('green');
    gain = await revealInspector('Uniform gain');
    await client.click(gain); await client.setValue(gain,'0.25'); await client.key('\n');
    await snapshot('14-uniform-dim-green','dim-green');
    await click('UNDO'); await waitColor('green');
    await click('REDO'); await snapshot('15-uniform-redo-dim-green', 'dim-green');
    const originalLabel=nodeState(await header(customId)).label;
    await client.setValue(await client.find({ label: 'Node label', role: 'TextInput' }), RENAMED);
    await client.key('\n');
    await client.waitFor(async()=> (await client.byLabel('Apply node label',{role:'Button'})).length===1,{description:'node label draft is dirty'});
    await activate('Apply node label');
    await click('UNDO');
    await client.waitFor(async () => nodeState(await header(customId)).label === originalLabel, { description: 'undo node label' });
    await click('REDO');
    await client.waitFor(async () => nodeState(await header(customId)).label === RENAMED, { description: 'redo node label' });
    await snapshot('16-label-redo', 'dim-green');
  });
  await step('Save preserves graph, uniforms, layout, clip isolation and source timestamps', async () => {
    savedData = await save();
    const graph = graphFor(savedData, 'red.mp4'), custom = graph.nodes.find(node => node.id === customId);
    assert.equal(custom.data.label, RENAMED); assert.equal(custom.data.shaderCode, GAIN);
    assert.equal(custom.data.uniforms.gain, 0.25);
    assert.deepEqual(graphFor(savedData, 'blue.mp4'), graphFor(originalData, 'blue.mp4'));
    for (const name of ['red.mp4', 'blue.mp4']) {
      const before = clipFor(originalData, name), after = clipFor(savedData, name);
      for (const key of ['timeline_start', 'duration', 'source_in_pts', 'source_subtick_numerator', 'source_subtick_denominator']) assert.equal(after[key], before[key], `${name} ${key} must not be retimed by a static graph`);
    }
    await writeFile(join(output, 'saved-project.json'), JSON.stringify(savedData, null, 2));
    await close();
  });
  await step('Reopen real native window restores editable graph and actual per-clip GPU output', async () => {
    await launch(project); await waitState('clips 2');
    await click('Nodes workspace'); await selectClip('red.mp4');
    await waitNodeCount(4); await snapshot('17-reopened-dim-green', 'dim-green');
    await selectNode(customId);
    assert.equal(nodeState(await header(customId)).label, RENAMED);
    await collapseSection('SHADER EDITOR'); await collapseSection('OUTPUT SIZE');
    assert.equal((await revealInspector('Uniform gain')).accessibleValue, '0.25');
    await selectClip('blue.mp4'); await waitNodeCount(2); await snapshot('18-reopened-isolated-blue', 'blue');
    await selectClip('red.mp4'); await waitNodeCount(4); await snapshot('19-reopened-red-grade', 'dim-green');
    assert.deepEqual(await save(), savedData, 'Save/reopen must preserve the complete document');
    await close();
  });
  report.passed = true;
} catch (error) {
  report.error = error.stack;
  if (client) {
    try {
      const path = join(output, 'failure.png'); await client.screenshot(path); report.screenshots.push(path);
      await writeFile(join(output, 'failure-tree.json'), JSON.stringify(await client.tree({ maxElements: 5000 }), null, 2));
    } catch (evidenceError) { report.evidenceError = evidenceError.stack; }
  }
  throw error;
} finally {
  if (child) {
    await writeFile(join(output, `application-${report.launches.length + 1}-interrupted.log`), stdout + stderr);
    child.kill('SIGTERM');
  }
  await writeFile(join(output, 'result.json'), JSON.stringify(report, null, 2));
  await writeHtmlReport(output, report, {
    ...report,
    caseDefinitions: report.steps.map(step => ({ id: step.name, title: step.name, description: 'Real native Slint graph/Inspector interactions, saved project assertions and decoded GPU screenshot pixels', screenshotPrefixes: [] })),
  });
  console.log(`[video-editor-nodes] Evidence: ${output}`);
}
