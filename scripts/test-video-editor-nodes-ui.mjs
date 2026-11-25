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
const args = process.argv.slice(2);
const smoke = args.includes('--smoke');
const positional = args.filter(arg => arg !== '--smoke');
assert(positional.length <= 1 && positional.every(arg => !arg.startsWith('--')), 'Expected [binary] [--smoke]');
const tier = smoke ? 'smoke' : 'full';
const output = resolve(process.env.VIDEO_EDITOR_NODES_TEST_OUTPUT || `test-results/clip-pipeline/nodes-ui-${tier}-${new Date().toISOString().replaceAll(/[:.]/g, '-')}`);
const binary = resolve(positional[0] || 'apps/open_quartz_video_editor/target/debug/open_quartz_video_editor');
const ffmpeg = resolve(process.env.OPEN_QUARTZ_FFMPEG_PATH || 'apps/open_quartz_slint/runtime/ffmpeg');
const project = join(output, 'clip-pipelines.oqvideo.json');
await mkdir(output, { recursive: true });
const report = { passed: false, tier, binary, project, steps: [], screenshots: [], launches: [], caseDefinitions: [] };
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
    env: { ...process.env, SLINT_MCP_PORT: String(serverPort), OPEN_QUARTZ_UI_CLOSE_STDIN: '1', OPEN_QUARTZ_EDITOR_RECOVERY_DIR: join(output, 'recovery') },
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
async function headers() {
  return client.waitFor(async () => {
    try { return await client.byLabel(/^Select node /, { role: 'Button', visible: false }); }
    catch (error) {
      if (/destroyed|stale/i.test(error.message)) return false;
      throw error;
    }
  }, { description: 'coherent shared graph header snapshot' });
}
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
    try {
      const inputs = await client.byLabel('Node label', { role: 'TextInput', visible: false });
      if (!inputs.length) return false;
      return inputs[0].accessibleValue === nodeState(await header(id)).label;
    } catch (error) {
      if (/destroyed|stale/i.test(error.message)) return false;
      throw error;
    }
  }, { description: `Inspector selection for ${id}` });
}
async function waitNodeCount(count) {
  return client.waitFor(async () => {
    try { return (await headers()).length === count; }
    catch (error) { if (/destroyed|stale/i.test(error.message)) return false; throw error; }
  }, { description: `${count} actual shared graph nodes` });
}
async function oneNewNode(before) {
  return client.waitFor(async () => {
    try {
      const nodes = (await headers()).map(nodeState).filter(node => !before.has(node.id));
      return nodes.length === 1 && nodes[0];
    } catch (error) {
      if (/destroyed|stale/i.test(error.message)) return false;
      throw error;
    }
  }, { description: 'new committed pipeline node' });
}
async function addEffect(label) {
  const before = new Set((await headers()).map(element => nodeState(element).id));
  await client.click(await button(`Add ${label} node`));
  await click('Fit graph in view');
  return oneNewNode(before);
}
async function selectClip(name) {
  await click(`Pipeline for ${name}`);
  await client.waitFor(async()=>{try{return (await headers()).some(element=>nodeState(element).id==='video');}catch{return false;}},{description:`${name} graph published`});
  await click('Fit graph in view');
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
async function step(name, run, { core = false } = {}) {
  if (smoke && !core) return;
  report.caseDefinitions.push({ id: name, title: name, description: 'Native UI actions and observable results', screenshotPrefixes: [] });
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
  const clips = data.tracks.flatMap(track => track.clips).filter(clip => clip.asset_id === asset.id);
  assert.equal(clips.length, 1, `Expected one saved ${name} clip`);
  return clips[0];
}
function graphFor(data, name) { return clipFor(data, name).pipeline.graph; }
const CORRECTOR_EXPOSURE = 'Corrector Exposure (stops)';
const correctorNode = (data, id) => graphFor(data, 'red.mp4').nodes.find(node => node.id === id);
async function correctorValue(value) {
  await client.waitFor(async () => {
    try {
      return (await client.find({ label: CORRECTOR_EXPOSURE, role: 'TextInput', visible: false })).accessibleValue === Number(value).toFixed(4);
    } catch (error) { if (/destroyed|stale/i.test(error.message)) return false; throw error; }
  }, { description: `committed native Corrector exposure ${value}` });
}
async function editCorrector(value) {
  const input = await revealInspector(CORRECTOR_EXPOSURE);
  await client.click(input); await client.setValue(input, String(value)); await client.key('\n');
  await correctorValue(value);
}
async function primaryKey(text) {
  // Slint's event layer maps native Command to logical Control on macOS.
  const primary = '\u0011';
  await client.key(primary, 'Press');
  try { await client.key(text); } finally { await client.key(primary, 'Release'); }
}

try {
  const red = generate('red.mp4', 'red'), blue = generate('blue.mp4', 'blue');
  await step('Create two real hardware-decoded clips through the native UI', async () => {
    await launch(); await waitState('assets 0; clips 0');
    await fileAction('IMPORT', red, 'Import media');
    await waitState('source PTS 0 ns · CPU image bytes 0');
    await click('ADD CLIP'); await waitState('clips 1');
    await fileAction('IMPORT', blue, 'Import media'); await waitState('assets 2');
    await waitColor('blue');
    await click('Add blue.mp4 to timeline'); await waitState('clips 2');
    await click('FIT TIMELINE');
    await click('Nodes workspace'); await selectClip('red.mp4');
    await waitNodeCount(2); await snapshot('01-nodes-default-red', 'red');
    originalData = await save(true);
    assert.equal(originalData.schema_version, 23);
    assert.deepEqual(originalData.tracks.flatMap(track => track.clips).map(clip => [clip.timeline_start, clip.duration]), [[0, 90], [90, 90]]);
  }, { core: true });
  await step('Source and output remain selectable, read-only and non-deletable', async () => {
    for (const id of ['video', 'output']) {
      await selectNode(id);
      const label = await client.query({ label: 'Node label', role: 'TextInput', visible: false });
      if (label.length) assert.equal(Boolean(label[0].accessibleEnabled), false, `${id} label must be protected`);
      await client.waitFor(async () => {
        try {
          assert.equal((await client.byLabel('Delete node', { role: 'Button' })).length, 0);
          return true;
        } catch (error) {
          if (/destroyed|stale/i.test(error.message)) return false;
          throw error;
        }
      }, { description: `${id} protected inspector settles without Delete` });
      await waitNodeCount(2); await snapshot(`02-protected-${id}`, 'red');
    }
    assert.deepEqual(graphFor(await save(), 'red.mp4'), graphFor(originalData, 'red.mp4'));
  });
  await step('The real catalog Invert transforms red GPU video to cyan', async () => {
    await selectNode('video');
    const invert = await addEffect('Invert'); invertId = invert.id;
    await waitNodeCount(3); await snapshot('03-invert-red-to-cyan', 'cyan');
    const data = await save(); const graph = graphFor(data, 'red.mp4');
    assert(graph.edges.some(edge => edge.source === 'video' && edge.target === invertId));
    assert(graph.edges.some(edge => edge.source === invertId && edge.target === 'output'));
    assert.deepEqual(graphFor(data, 'blue.mp4'), graphFor(originalData, 'blue.mp4'));
    return { nodeId: invertId };
  }, { core: true });
  await step('Clip switching isolates graphs and uses the same final video viewer', async () => {
    await selectClip('blue.mp4'); await waitNodeCount(2); await snapshot('04-isolated-blue', 'blue');
    await selectClip('red.mp4'); await waitNodeCount(3); await snapshot('05-return-red-cyan', 'cyan');
  }, { core: true });
  await step('Shared custom shader editor applies real constant green output', async () => {
    await selectNode('output');
    const custom = await addEffect('Custom Shader'); customId = custom.id;
    await waitNodeCount(4); await waitColor('cyan');
    await selectNode(customId); await setShader(GREEN);
    await snapshot('06-custom-shader-green', 'green');
    const node = graphFor(await save(), 'red.mp4').nodes.find(node => node.id === customId);
    assert.equal(node.data.shaderCode, GREEN);
    return { nodeId: customId, shader: GREEN };
  }, { core: true });
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
  }, { core: true });
  await step('Downstream effect order, real port reconnection and serial removal change GPU pixels', async () => {
    await selectNode(customId);
    const secondInvert = await addEffect('Invert');
    await waitNodeCount(5); await snapshot('08-green-then-invert-magenta', 'magenta');
    await click('Fit graph in view');
    const customLabel=nodeState(await header(customId)).label;
    const outputLabel=nodeState(await header('output')).label;
    await client.activate(await port(customLabel, true));
    await client.waitFor(async () => {
      try { return (await client.byLabel('Cancel connection', { role: 'Button' })).length === 1; }
      catch (error) { if (/destroyed|stale/i.test(error.message)) return false; throw error; }
    }, { description: 'source port activates the native connection transaction' });
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
  }, { core: true });
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
    await selectNode(customId);
    await setShader(GAIN);
    await client.waitFor(async()=> { try { return (await client.byLabel('Apply shader',{role:'Button'})).every(button=>!button.accessibleEnabled); } catch(error) { if (/destroyed|stale/i.test(error.message)) return false; throw error; } },{description:'shader commit completed'});
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
    await client.waitFor(async()=> { try { return (await client.byLabel('Apply node label',{role:'Button'})).length===1; } catch(error) { if (/destroyed|stale/i.test(error.message)) return false; throw error; } },{description:'node label draft is dirty'});
    await activate('Apply node label');
    await click('UNDO');
    await client.waitFor(async () => nodeState(await header(customId)).label === originalLabel, { description: 'undo node label' });
    await click('REDO');
    await client.waitFor(async () => nodeState(await header(customId)).label === RENAMED, { description: 'redo node label' });
    await snapshot('16-label-redo', 'dim-green');
  }, { core: true });
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
  }, { core: true });
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
  }, { core: true });
  let correctorId, correctorSaved;
  await step('Native Color Corrector creates a typed serial node and previews without saving transient values', async () => {
    await launch(project); await waitState('clips 2');
    await click('Nodes workspace'); await selectClip('red.mp4');
    await selectNode(customId); await setShader(GREEN); await waitColor('green');
    const corrector = await addEffect('Color Corrector'); correctorId = corrector.id;
    await waitNodeCount(5); await selectNode(correctorId); await snapshot('20-native-corrector-identity', 'green');
    const baseline = await save(), node = correctorNode(baseline, correctorId);
    assert.equal(node.type, 'colorcorrector'); assert.equal(node.data.type, 'colorcorrector');
    assert.equal(node.data.shaderCode, ''); assert.equal(node.data.colorCorrector.primaries.exposure, 0);
    assert(graphFor(baseline, 'red.mp4').edges.some(edge => edge.source === customId && edge.target === correctorId));
    assert(graphFor(baseline, 'red.mp4').edges.some(edge => edge.source === correctorId && edge.target === 'output'));
    const exposure = await revealInspector(CORRECTOR_EXPOSURE);
    await client.click(exposure); await client.setValue(exposure, '-2');
    await snapshot('21-native-corrector-transient-gpu', 'dim-green');
    assert.deepEqual(JSON.parse(await readFile(project, 'utf8')).data, baseline, 'GPU preview must not serialize its draft');
    await client.key('\n'); await correctorValue(-2);
    const graded = await save(); assert.equal(correctorNode(graded, correctorId).data.colorCorrector.primaries.exposure, -2);
    await click('UNDO'); await correctorValue(0); await snapshot('22-native-corrector-one-step-undo', 'green');
    await click('REDO'); await correctorValue(-2); await snapshot('23-native-corrector-redo', 'dim-green');
    assert.deepEqual(graphFor(graded, 'blue.mp4'), graphFor(originalData, 'blue.mp4'));
  }, { core: true });
  await step('Native bypass, stage reset, cancellation and wheel drag retain one-gesture history', async () => {
    await client.click(await revealInspector('Corrector enabled', 'Checkbox')); await snapshot('24-native-corrector-bypass', 'green');
    assert.equal(correctorNode(await save(), correctorId).data.colorCorrector.enabled, false);
    await client.click(await revealInspector('Corrector enabled', 'Checkbox')); await waitColor('dim-green');
    await client.click(await revealInspector('Primaries enabled', 'Checkbox')); await waitColor('green');
    await client.click(await revealInspector('Primaries enabled', 'Checkbox')); await waitColor('dim-green');
    await client.activate(await revealInspector('Reset Primaries', 'Button')); await correctorValue(0); await waitColor('green');
    await click('UNDO'); await correctorValue(-2); await waitColor('dim-green');
    const beforeCancel = await save();
    const input = await revealInspector(CORRECTOR_EXPOSURE);
    await client.click(input); await client.setValue(input, '-1');
    await client.activate(await revealInspector('Cancel Corrector gesture', 'Button'));
    await correctorValue(-2); await waitColor('dim-green');
    assert.deepEqual(await save(), beforeCancel, 'Cancelled native edit must not create document state');
    const switching = await revealInspector(CORRECTOR_EXPOSURE);
    await client.click(switching); await client.setValue(switching, '-1');
    await selectClip('blue.mp4'); await waitColor('blue');
    await selectClip('red.mp4'); await selectNode(correctorId); await correctorValue(-2); await waitColor('dim-green');
    assert.deepEqual(await save(), beforeCancel, 'Selection changes must discard a transient Corrector gesture');
    await client.activate(await revealInspector('Corrector primaries Offset', 'Button'));
    const wheel = await revealInspector('Offset color wheel; arrow keys adjust, Escape cancels', 'Button');
    await client.drag(wheel, { x: wheel.absolutePosition.x + wheel.size.width / 2 + 20, y: wheel.absolutePosition.y + wheel.size.height / 2 + 8 });
    await revealInspector('Corrector Offset R');
    await client.waitFor(async () => {
      const offset = await client.find({ label: 'Corrector Offset R', role: 'TextInput', visible: false });
      return Math.abs(Number(offset.accessibleValue)) > 0.001 && !(await client.byLabel('Discard changes', { role: 'Button', visible: false })).length;
    }, { description: 'wheel release committed through document history' });
    const afterWheel = await save();
    assert.notDeepEqual(correctorNode(afterWheel, correctorId).data.colorCorrector, correctorNode(beforeCancel, correctorId).data.colorCorrector);
    await click('UNDO'); await waitColor('dim-green');
    assert.deepEqual(await save(), beforeCancel, 'One undo must remove every native wheel preview sample');
    await click('REDO');
    assert.deepEqual(await save(), afterWheel, 'One redo must restore the whole wheel gesture');
    await click('UNDO'); await waitColor('dim-green');
  });
  await step('Native Corrector clipboard and split clones own independent typed configurations', async () => {
    await selectNode(correctorId); await client.click(await header(correctorId)); await primaryKey('c');
    await selectNode('output');
    const beforePaste = new Set((await headers()).map(element => nodeState(element).id));
    await primaryKey('v'); await click('Fit graph in view'); const pasted = await oneNewNode(beforePaste); await waitNodeCount(6);
    await selectNode(pasted.id); await editCorrector(-1);
    let data = await save();
    assert.equal(correctorNode(data, correctorId).data.colorCorrector.primaries.exposure, -2);
    assert.equal(correctorNode(data, pasted.id).data.colorCorrector.primaries.exposure, -1);
    await activate('Delete node'); await waitNodeCount(5); await waitColor('dim-green');
    await click('UNDO'); await waitNodeCount(6);
    assert.equal(correctorNode(await save(), pasted.id).data.colorCorrector.primaries.exposure, -1);
    await click('REDO'); await waitNodeCount(5); await waitColor('dim-green');
    const beforeSplit = await save();
    await click('NEXT'); await waitState('playhead 1 frames'); await click('SPLIT'); await waitState('clips 3');
    await selectNode(correctorId); await editCorrector(-1);
    data = await save();
    const asset = data.assets.find(asset => asset.name === 'red.mp4');
    const halves = data.tracks.flatMap(track => track.clips).filter(clip => clip.asset_id === asset.id).sort((a, b) => a.timeline_start - b.timeline_start);
    assert.equal(halves.length, 2);
    assert.deepEqual(halves.map(clip => clip.pipeline.graph.nodes.find(node => node.id === correctorId).data.colorCorrector.primaries.exposure), [-2, -1]);
    await click('UNDO'); await correctorValue(-2); await click('UNDO'); await waitState('clips 2');
    assert.deepEqual(await save(), beforeSplit, 'Undo split must restore the exact independent pipeline snapshot');
    await selectClip('blue.mp4'); await snapshot('25-corrector-clip-isolation', 'blue');
    await selectClip('red.mp4'); await selectNode(correctorId); await snapshot('26-corrector-isolated-return', 'dim-green');
    correctorSaved = await save(); await close();
  });
  await step('Save and reopen restore the actual native Corrector and managed GPU result', async () => {
    await launch(project); await waitState('clips 2'); await click('Nodes workspace'); await selectClip('red.mp4');
    await click('Fit graph in view');
    await waitNodeCount(5); await selectNode(correctorId); await correctorValue(-2);
    await snapshot('27-reopened-native-corrector', 'dim-green');
    assert.equal((await client.byLabel('Shader source', { role: 'TextInput', visible: false })).length, 0, 'Native Corrector must not expose a shader disguise');
    assert.deepEqual(await save(), correctorSaved);
    await click('PLAY');
    await client.waitFor(async () => (await client.byLabel('PAUSE', { role: 'Button' })).length === 1, { description: 'playing transport before Corrector gesture' });
    const playingInput = await revealInspector(CORRECTOR_EXPOSURE);
    await client.click(playingInput); await client.setValue(playingInput, '-1');
    await client.waitFor(async () => (await client.byLabel('PLAY', { role: 'Button' })).length === 1, { description: 'Corrector gesture publishes paused transport' });
    await client.activate(await revealInspector('Cancel Corrector gesture', 'Button'));
    await correctorValue(-2);
    assert.deepEqual(await save(), correctorSaved, 'Pausing and cancelling must not change accepted clip parameters');
    await close();
  });
  await step('The same clip graph applies an effect to a still image and survives reopening', async () => {
    const image = join(output, 'red-still.png');
    execFileSync(ffmpeg, ['-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'color=c=red:s=320x180',
      '-frames:v', '1', '-c:v', 'png', '-threads', '1', '-y', image],
    { env: { ...process.env, DYLD_LIBRARY_PATH: '', DYLD_FALLBACK_LIBRARY_PATH: '' } });
    await launch();
    await fileAction('IMPORT', image, 'Import media'); await waitState('assets 1');
    await waitColor('red');
    await click('ADD CLIP'); await waitState('clips 1');
    await click('Nodes workspace'); await selectClip('red-still.png');
    await waitNodeCount(2);
    await selectNode('video');
    await addEffect('Invert');
    await waitNodeCount(3); await snapshot('28-still-inverted', 'cyan');
    const saved = join(output, 'still-pipeline.oqvideo.json');
    await fileAction('SAVE AS', saved, 'Save project'); await waitState('Project saved');
    await close();
    await launch(saved); await waitState('clips 1');
    await click('Nodes workspace'); await selectClip('red-still.png');
    await waitNodeCount(3); await waitColor('cyan');
    await close();
  });
  if (smoke) await close();
  await step('Duplicate, switch and remove independent grade versions on a clip', async () => {
    const path = join(output, 'grade-versions.oqvideo.json');
    await launch();
    await fileAction('IMPORT', join(output, 'red.mp4'), 'Import media'); await waitState('assets 1');
    await click('ADD CLIP'); await waitState('clips 1');
    await click('Nodes workspace'); await selectClip('red.mp4'); await waitColor('red');
    const options = () => client.find({ label: 'Grade version', role: 'Combobox' });
    const choose = async (key, label) => {
      await client.click(await options()); await client.key(key); await client.key('\n');
      await client.waitFor(async () => (await options()).accessibleValue === label,
        { description: `active grade ${label}` });
    };
    await click('Grade versions'); await click('Duplicate grade');
    const name = await client.find({ label: 'Grade version name', role: 'TextInput' });
    await client.click(name); await client.setValue(name, 'Night'); await client.key('\n');
    assert.equal((await options()).accessibleValue, 'Night');
    await click('Close versions');
    await selectNode('video'); await addEffect('Invert'); await waitColor('cyan');
    await click('Grade versions'); await choose('\uf700', 'Original');
    await click('Close versions'); await waitColor('red');
    await click('Grade versions'); await choose('\uf701', 'Night');
    await click('Close versions'); await waitColor('cyan');
    await fileAction('SAVE AS', path, 'Save project'); await waitState('Project saved');
    const data = JSON.parse(await readFile(path, 'utf8')).data;
    const grades = data.tracks[0].clips[0].grade_versions;
    assert.equal(grades.active.name, 'Night');
    assert.equal(grades.alternatives[0].label.name, 'Original');
    assert.equal(grades.alternatives[0].pipeline.graph.nodes.length, 2);
    await close();
    await launch(path); await waitState('clips 1'); await click('Nodes workspace'); await selectClip('red.mp4');
    await waitColor('cyan');
    await click('Grade versions'); assert.equal((await options()).accessibleValue, 'Night');
    await click('Remove grade'); assert.equal((await options()).accessibleValue, 'Original');
    await click('Close versions'); await waitColor('red');
    await click('UNDO'); await waitColor('cyan');
    await click('SAVE'); await waitState('Project saved');
    await close();
  }, { core: true });
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
    caseDefinitions: report.caseDefinitions,
  });
  console.log(`[video-editor-nodes] Evidence: ${output}`);
}
