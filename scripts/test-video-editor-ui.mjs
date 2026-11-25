#!/usr/bin/env node
import assert from 'node:assert/strict';
import { spawn, spawnSync, execFileSync } from 'node:child_process';
import { mkdir, readFile, writeFile, rename, readdir } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { createServer } from 'node:net';
import { setTimeout as delay } from 'node:timers/promises';
import { SlintUiClient } from './slint-ui-client.mjs';
import { decodePng } from './slint-ui-scenarios.mjs';
import { writeHtmlReport } from './slint-ui-report.mjs';

const args = process.argv.slice(2);
const smoke = args.includes('--smoke');
const audioOnly = args.includes('--audio-only');
const previewOnly = args.includes('--preview-only');
const deliveryOnly = args.includes('--delivery-only');
assert([smoke, audioOnly, previewOnly, deliveryOnly].filter(Boolean).length <= 1, 'Choose at most one test filter');
const positional = args.filter(arg => !['--smoke', '--audio-only', '--preview-only', '--delivery-only'].includes(arg));
assert(positional.length <= 1 && positional.every(arg => !arg.startsWith('--')), 'Expected [binary] and an optional suite filter');
const tier = deliveryOnly ? 'delivery-only' : audioOnly ? 'audio-only' : previewOnly ? 'preview-only' : smoke ? 'smoke' : 'full';
const output=resolve(process.env.VIDEO_EDITOR_TEST_OUTPUT || `test-results/phase6/ui-${tier}-${new Date().toISOString().replaceAll(/[:.]/g,'-')}`);
const binary=resolve(positional[0] || 'apps/open_quartz_video_editor/target/debug/open_quartz_video_editor');
const ffmpeg=resolve(process.env.OPEN_QUARTZ_FFMPEG_PATH || 'apps/open_quartz_slint/runtime/ffmpeg');
await mkdir(output,{recursive:true});
const report={passed:false,tier,steps:[],screenshots:[],launches:[],caseDefinitions:[]};
const server=createServer();await new Promise(r=>server.listen(0,'127.0.0.1',r));const port=server.address().port;await new Promise(r=>server.close(r));
let child,client,exited,stdout='',stderr='';
const project=join(output,'edit.oqvideo.json');
async function launch(path){
  stdout='';stderr='';child=spawn(binary,path?['--project',path]:[],{env:{...process.env,SLINT_MCP_PORT:String(port),OPEN_QUARTZ_UI_CLOSE_STDIN:'1',OPEN_QUARTZ_EDITOR_RECOVERY_DIR:join(output,'recovery'),OPEN_QUARTZ_PROXY_CACHE_DIR:join(output,'proxies')},stdio:['pipe','pipe','pipe']});
  child.stdout.on('data',data=>stdout+=data);child.stderr.on('data',data=>stderr+=data);
  exited=new Promise((accept,reject)=>{child.once('error',reject);child.once('close',(code,signal)=>accept({code,signal}));});
  client=new SlintUiClient(`http://127.0.0.1:${port}/mcp`);
  await client.waitFor(async()=>{try{await client.init();await client.useWindow();return true;}catch{return false;}},{description:'native editor window'});
}
async function close(){await client.activate((await buttons('Close window'))[0]);const exit=await Promise.race([exited,delay(10000).then(()=>{throw Error('Window did not close normally');})]);assert.deepEqual(exit,{code:0,signal:null});report.launches.push(exit);await writeFile(join(output,`application-${report.launches.length}.log`),stdout+stderr);child=undefined;}
const state=async()=> (await client.properties(client.rootHandle)).accessibleDescription;
async function buttons(label) {
  return client.waitFor(async () => {
    try { return await client.byLabel(label, { role: 'Button' }); }
    catch (error) { if (!/destroyed|stale/i.test(error.message)) throw error; }
  }, { description: `stable controls for ${label}` });
}
async function click(label,last=false){const items=await buttons(label);assert(items.length,`Missing UI button ${label}`);await client.click(last?items.at(-1):items[0]);}
async function fileAction(action, path, submit) {
  await click(action);
  const input = await client.waitFor(async () => {
    const fields = await client.byLabel('File path', { role: 'TextInput' });
    return fields.length === 1 && fields[0];
  }, { description: `${action} file dialog` });
  await client.setValue(input, path);
  await click(submit, true);
}
const timeline=async name=>(await buttons(name)).filter(item=>item.accessibleDescription?.startsWith('Timeline start'));
const assetCard=async name=>{const cards=(await buttons(name)).filter(item=>item.typeNamesAndIds?.some(type=>type.typeName==='AssetCard'));assert.equal(cards.length,1,`Expected one media card ${name}`);return cards[0];};
async function waitState(fragment){return client.waitFor(async()=> (await state()).includes(fragment),{description:fragment});}
async function previewHasColor(png,color){const preview=await client.find({label:'Video preview',role:'Image'});const image=decodePng(png);const window=await client.windowProperties();const scale=window.scaleFactor||1;const sx=image.width/(window.size.width/scale),sy=image.height/(window.size.height/scale);const x=Math.floor(((preview.absolutePosition.x||0)+preview.size.width/2)*sx);const y=Math.floor(((preview.absolutePosition.y||0)+preview.size.height/2)*sy);let hits=0;for(let dy=-15;dy<15;dy++)for(let dx=-15;dx<15;dx++){const at=((y+dy)*image.width+x+dx)*image.channels;const [r,g,b]=image.pixels.subarray(at,at+3);if(color==='red'?r>220&&g<25&&b<25:color==='blue'?b>220&&r<25&&g<25:color==='magenta'?r>220&&b>220&&g<25:color==='blend'?r>=178&&r<=196&&g<25&&b>=178&&b<=196:color==='three-quarter'?r>=218&&r<=234&&g<25&&b>=125&&b<=150:r<5&&g<5&&b<5)hits++;}return hits>=890;}
async function snapshot(name,color){const path=join(output,`${name}.png`);const png=await client.screenshot(path);report.screenshots.push(path);if(color)assert(await previewHasColor(png,color),`${name}: wrong visible ${color} pixels`);return png;}
async function expectColor(color) {
  await client.waitFor(async () => previewHasColor(await client.screenshot(), color),
    { description: `composited ${color} pixels` });
}
async function step(name, run, { core = false } = {}) {
  if (smoke && !core) return;
  report.caseDefinitions.push({ id: name, title: name, description: 'Native UI actions and observable results', screenshotPrefixes: [] });
  const startedAt=new Date().toISOString();const first=report.screenshots.length;try{await run();report.steps.push({name,passed:true,startedAt,finishedAt:new Date().toISOString(),screenshots:report.screenshots.slice(first)});console.log(`[video-editor] PASS ${name}`);}catch(error){report.steps.push({name,passed:false,startedAt,finishedAt:new Date().toISOString(),error:error.message,screenshots:report.screenshots.slice(first)});throw error;}}
function generate(name,color,duration=3,fps=30,size='320x180'){const path=join(output,name);execFileSync(process.env.OPEN_QUARTZ_TEST_FFMPEG_PATH || ffmpeg,['-hide_banner','-loglevel','error','-f','lavfi','-i',`color=c=${color}:s=${size}:r=${fps}`,'-vf','scale=in_color_matrix=bt601:out_color_matrix=bt709','-t',String(duration),'-c:v','libx264','-pix_fmt','yuv420p','-color_primaries','bt709','-color_trc','bt709','-colorspace','bt709','-y',path],{env:{...process.env,DYLD_LIBRARY_PATH:'',DYLD_FALLBACK_LIBRARY_PATH:''}});return path;}
async function audioOnlyRoundTrip() {
  await step('Import, play and reopen an independent PCM clip', async () => {
    const path = join(output, 'audio-only-tone.wav');
    const audioProject = join(output, 'audio-only.oqvideo.json');
    execFileSync(ffmpeg, ['-v', 'error', '-f', 'lavfi', '-i', 'sine=frequency=880:sample_rate=44100:duration=5',
      '-c:a', 'pcm_s16le', '-y', path], { env: { ...process.env, DYLD_LIBRARY_PATH: '', DYLD_FALLBACK_LIBRARY_PATH: '' } });
    await launch();
    await fileAction('IMPORT', path, 'Import media');
    await waitState('assets 1');
    await click('Add audio-only-tone.wav to timeline');
    await waitState('audio clips 1');
    const view = await client.find({ id: 'EditorWindow::timeline_scroll' });
    await client.value('dispatch_pointer_scroll', { windowHandle: client.windowId, deltaX: 0, deltaY: -170,
      position: { x: view.absolutePosition.x + 100, y: view.absolutePosition.y + 50 } });
    await client.waitFor(async () => (await client.byLabel('Waveform audio-only-tone.wav', { role: 'Image' })).length === 1,
      { description: 'decoded PCM waveform' });
    await click('PLAY');
    await client.waitFor(async () => /Audio output [1-9][0-9]*%/.test(await state()) && /playhead [1-9]\d* frames/.test(await state()),
      { description: 'audio meter and device-driven playhead advance' });
    await click('PAUSE');
    await client.waitFor(async () => !(await state()).includes('Audio output'), { description: 'pause stops device output' });
    await fileAction('SAVE AS', audioProject, 'Save project');
    await waitState('Project saved');
    const data = JSON.parse(await readFile(audioProject, 'utf8')).data;
    assert.equal(data.assets[0].audio.rate, 44100);
    assert.equal(data.audio_tracks[0].clips[0].placement.duration, 240000);
    await snapshot('audio-timeline');
    await close();
    await launch(audioProject);
    await waitState('audio clips 1');
    await close();
  });
}

const fixtureCommand = args => execFileSync(process.env.OPEN_QUARTZ_TEST_FFMPEG_PATH || ffmpeg,
  ['-hide_banner', '-loglevel', 'error', ...args],
  { env: { ...process.env, DYLD_LIBRARY_PATH: '', DYLD_FALLBACK_LIBRARY_PATH: '' } });

const center = item => ({
  x: item.absolutePosition.x + item.size.width / 2,
   y: item.absolutePosition.y + item.size.height / 2,
 });
 const grip = name => client.find({ label: `Reorder ${name}`, role: 'Button' });
const enter = async (input, value) => {
  await client.click(input);
   await client.setValue(input, String(value));
   await client.key('\n');
 };
const clipControl = async (label, role = 'TextInput', section = 'clip_properties') => {
  const viewport = await client.find({ id: `EditorWindow::${section}` });
   const scroll = deltaY => client.value('dispatch_pointer_scroll', {
     windowHandle: client.windowId, position: center(viewport), deltaX: 0, deltaY,
   });
   await scroll(2000);
   for (let attempt = 0; attempt < 12; attempt++) {
     let control;
     try {
       [control] = await client.byLabel(label, { role, visible: false });
     } catch (error) {
       if (!/destroyed|stale/i.test(error.message)) throw error;
       await delay(80);
       continue;
     }
     const bounds = viewport.absolutePosition.y;
     if (control) {
       const y = center(control).y;
       if (y > bounds + 4 && y < bounds + viewport.size.height - 4) return control;
       await scroll(bounds + viewport.size.height / 2 - y);
     } else {
       await scroll(-140);
     }
     await delay(80);
   }
   throw new Error(`Could not scroll clip Inspector to ${label}`);
 };

async function maskTrackingRoundTrip(decodeOutput) {
  const verifyExport = Boolean(decodeOutput);
 await step(verifyExport ? 'Track a translating clip mask and retain the animated result in export' : 'Track a translating clip mask and verify the monitor', async () => {
   const input = join(output, 'move-mask.mp4');
   const project = join(output, 'track-mask.oqvideo.json');
   fixtureCommand(['-f', 'lavfi', '-i',
     "nullsrc=s=320x180:r=30,geq=lum='if(between(X,40+N*4,79+N*4)*between(Y,65,115),235,16)':cb=128:cr=128",
     '-frames:v', '36', '-pix_fmt', 'yuv420p', '-c:v', 'libx264', '-color_range', 'tv',
     '-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-y', input]);
   await launch();
   await click('SEQUENCE');
   await client.setValue(await client.find({ label: 'Sequence width', role: 'TextInput' }), '320');
   await client.setValue(await client.find({ label: 'Sequence height', role: 'TextInput' }), '180');
   await click('Apply settings');
   await fileAction('IMPORT', input, 'Import media'); await waitState('assets 1'); await click('ADD CLIP'); await waitState('clips 1');
   await client.click((await timeline('move-mask.mp4'))[0]);
   await client.click(await clipControl('Mask shape', 'Combobox'));
   await client.key('\uf701'); await client.key('\n');
   for (const [label, value] of [['center X', 18.75], ['width', 18], ['height', 50]]) {
     await enter(await clipControl(`Mask ${label} (%)`), value);
   }
   const track = async () => client.click(await clipControl('Track mask forward', 'Button'));
   await track();
   await client.click(await client.find({ label: 'Cancel mask tracking', role: 'Button' }));
   await waitState('Mask tracking cancelled');
   await track(); await waitState('Mask tracking complete');
   await fileAction('SAVE AS', project, 'Save project'); await waitState('Project saved');
   const clip = JSON.parse(await readFile(project, 'utf8')).data.tracks[0].clips[0];
   assert.equal(clip.mask_tracking.length, 36);
   assert(clip.mask_tracking[17].x > 0.38 && clip.mask_tracking[17].x < 0.42, JSON.stringify(clip.mask_tracking[17]));
   assert(Math.abs(clip.mask_tracking[17].y - 0.5) < 0.03);
   const check = async white => client.waitFor(async () => {
     const viewer = await client.find({ label: 'Video preview', role: 'Image' });
     const png = decodePng(await client.screenshot());
     const window = await client.windowProperties();
     const scale = window.scaleFactor || 1;
     const sx = png.width / (window.size.width / scale), sy = png.height / (window.size.height / scale);
     const width = Math.min(viewer.size.width, viewer.size.height * 320 / 180);
     const x = Math.floor((viewer.absolutePosition.x + viewer.size.width / 2 - width * 0.1) * sx);
     const y = Math.floor((viewer.absolutePosition.y + viewer.size.height / 2) * sy);
     const level = png.pixels[(y * png.width + x) * png.channels];
     return white ? level > 210 : level < 20;
   }, { description: white ? 'moving patch stays within the tracked monitor mask' : 'static mask excludes the moving patch' });
   for (let i = 0; i < 17; i++) await click('NEXT');
   await check(true);
   await client.click(await clipControl('Clear mask tracking', 'Button')); await check(false);
   await click('UNDO'); await check(true);
   if (verifyExport) {
     const path = join(output, 'tracked-mask.mp4');
     await fileAction('EXPORT', path, 'Export MP4'); await waitState('Export complete');
     const pixels = decodeOutput(['-i', path, '-vf', 'select=eq(n\\,17)', '-frames:v', '1',
       '-an', '-pix_fmt', 'rgb24', '-f', 'rawvideo', '-']);
     assert(pixels[(90 * 320 + 128) * 3] > 210, 'tracked clip export keeps the moving patch');
   }
   await click('SAVE'); await waitState('Project saved'); await close();
 }, { core: true });
}

async function hdrPreviewRoundTrip() {
 await step('Import PQ and HLG and inspect the hardware tone-mapped monitor', async () => {
   const asset = (name, transfer) => {
     const path = join(output, name + '.mp4');
     fixtureCommand(['-f', 'lavfi', '-i',
       "nullsrc=s=320x180:r=30,format=yuv420p10le,geq=lum=509:cb=512:cr=512",
       '-frames:v', '24', '-c:v', 'libx265', '-x265-params', 'pools=1:frame-threads=1:log-level=error',
       '-pix_fmt', 'yuv420p10le', '-tag:v', 'hvc1', '-color_range', 'tv',
       '-colorspace', 'bt2020nc', '-color_primaries', 'bt2020', '-color_trc', transfer, '-y', path]);
     return path;
   };
   const pq = asset('pq-neutral', 'smpte2084');
   const hlg = asset('hlg-neutral', 'arib-std-b67');
   await launch();
   const check = async (name, profile, min, max) => {
     const card = await assetCard(name + '.mp4');
     assert(card.accessibleDescription.includes(profile));
     await client.waitFor(async () => {
       const pixels = decodePng(await client.screenshot());
       const viewer = await client.find({ label: 'Video preview', role: 'Image' });
       const win = await client.windowProperties();
       const scale = pixels.width / (win.size.width / (win.scaleFactor || 1));
       const x = Math.floor((viewer.absolutePosition.x + viewer.size.width / 2) * scale);
       const y = Math.floor((viewer.absolutePosition.y + viewer.size.height / 2) * scale);
       const offset = (y * pixels.width + x) * pixels.channels;
       const [red, green, blue] = pixels.pixels.subarray(offset, offset + 3);
       return red >= min && red <= max && Math.abs(red - green) <= 2 && Math.abs(red - blue) <= 2;
     }, { description: profile + ' neutral display value' });
   };
   await fileAction('IMPORT', pq, 'Import media'); await waitState('assets 1');
   await check('pq-neutral', 'BT.2020 PQ · 10-bit', 140, 157);
   await click('ADD CLIP'); await waitState('clips 1');
   await check('pq-neutral', 'BT.2020 PQ · 10-bit', 140, 157);
   await fileAction('IMPORT', hlg, 'Import media'); await waitState('assets 2');
   await check('hlg-neutral', 'BT.2020 HLG · 10-bit', 114, 132);
   const project = join(output, 'hdr-import.oqvideo.json');
   await fileAction('SAVE AS', project, 'Save project'); await waitState('Project saved'); await close();
   await launch(project); await waitState('clips 1');
   await check('pq-neutral', 'BT.2020 PQ · 10-bit', 140, 157);
   await close();
 }, { core: true });
}

async function linkedAudioPreview() {
  await step('Play and pause hardware video with linked AAC audio', async () => {
    const input = generate('linked-preview-picture.mp4', 'red');
    const linked = join(output, 'linked-preview.mp4');
    execFileSync(ffmpeg, [
      '-v', 'error', '-i', input, '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000:duration=3',
      '-map', '0:v:0', '-map', '1:a:0', '-c:v', 'copy', '-c:a', 'aac', '-y', linked,
    ], { env: { ...process.env, DYLD_LIBRARY_PATH: '', DYLD_FALLBACK_LIBRARY_PATH: '' } });
    await launch();
    await fileAction('IMPORT', linked, 'Import media');
    await waitState('assets 1');
    await click('ADD CLIP');
    await waitState('audio clips 1');
    const clip = await client.find({ label: 'Audio clip linked-preview.mp4', role: 'Button', visible: false });
    assert(clip.accessibleDescription.endsWith('linked video'));
    await click('PLAY');
    await client.waitFor(async () => {
      const status = await state();
      return /Audio output [1-9][0-9]*%/.test(status) && /playhead [1-9]\d* frames/.test(status);
    }, { description: 'linked audio and video advance together' });
    await click('PAUSE');
    await client.waitFor(async () => !(await state()).includes('Audio output'), { description: 'pause stops linked audio' });
    await snapshot('linked-audio', 'red');
    await fileAction('SAVE AS', join(output, 'linked-preview.oqvideo.json'), 'Save project');
    await waitState('Project saved');
    await close();
  });
}

async function proxyRoundTrip() {
 await step('Generate a proxy for reordered VFR footage with a nonzero origin', async () => {
   const original = resolve('crates/oq-media/tests/source-time.mp4');
   const cacheFiles = async () => {
     try { return (await readdir(join(output, 'proxies'))).filter(name => /^[a-f0-9]{64}\.mp4$/.test(name)); }
     catch (error) { if (error.code === 'ENOENT') return []; throw error; }
   };
   const previous = new Set(await cacheFiles());
   await launch();
   await fileAction('IMPORT', original, 'Import media');
   await waitState('source PTS 0 ns');
   await click('Prepare proxy'); await waitState('Proxy ready');
   const created = (await cacheFiles()).filter(name => !previous.has(name));
   assert.equal(created.length, 1);
   const timestamps = path => {
     const decoded = spawnSync(ffmpeg, ['-hide_banner', '-copyts', '-i', path,
       '-vf', 'showinfo', '-fps_mode', 'passthrough', '-f', 'null', '-'], { env: { ...process.env, DYLD_LIBRARY_PATH: '', DYLD_FALLBACK_LIBRARY_PATH: '' }, encoding: 'utf8' });
     assert.equal(decoded.status, 0, decoded.stderr);
     return Array.from(decoded.stderr.matchAll(/\bn:\s+\d+ pts:\s+(\d+)\b/g), match => Number(match[1]));
   };
   const source = timestamps(original);
   assert.equal(source.length, 34);
   assert.equal(source[0], 2000);
   assert(source.some((timestamp, index) => timestamp - source[index - 1] === 200));
   const proxy = timestamps(join(output, 'proxies', created[0]));
   assert.deepEqual(proxy, source.map(pts => pts - source[0]));
   await click('Use proxies'); await waitState('Proxy preview enabled');
   await click('ADD CLIP'); await waitState('clips 1');
   await click('NEXT'); await waitState('source PTS 100000000 ns');
   await click('Use originals'); await waitState('Original preview enabled');
   await fileAction('SAVE AS', join(output, 'proxy-vfr.oqvideo.json'), 'Save project');
   await waitState('Project saved');
   await close();
 });
}

async function hardwareDeliveryRoundTrip() {
  await step('Deliver SDR, HDR10 and HLG MP4 with linked AAC audio', async () => {
    const picture = generate('delivery-picture.mp4', 'red');
    const input = join(output, 'delivery-linked.mp4');
    fixtureCommand(['-i', picture, '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000:duration=3',
      '-map', '0:v:0', '-map', '1:a:0', '-c:v', 'copy', '-c:a', 'aac', '-y', input]);
    await launch();
    await click('SEQUENCE');
    await client.setValue(await client.find({ label: 'Sequence width', role: 'TextInput' }), '320');
    await client.setValue(await client.find({ label: 'Sequence height', role: 'TextInput' }), '180');
    await click('Apply settings');
    await fileAction('IMPORT', input, 'Import media');
    await waitState('assets 1');
    await click('ADD CLIP'); await waitState('audio clips 1');
    const profile = () => client.find({ label: 'Export color profile', role: 'Combobox' });
    const labels = ['SDR Rec.709', 'HDR10', 'HLG'];
    for (const [name, index, codec, transfer] of [
      ['sdr', 0, /Video: h264/, 'bt709'],
      ['hdr10', 1, /Video: hevc \(Main 10\)/, 'smpte2084'],
      ['hlg', 2, /Video: hevc \(Main 10\)/, 'arib-std-b67'],
    ]) {
      const target = join(output, name + '.mp4');
      await click('EXPORT');
      const field = await profile();
      const selected = labels.findIndex(label => field.accessibleValue.startsWith(label));
      assert(selected >= 0, field.accessibleValue);
      if (selected !== index) {
        await client.click(field);
        for (let i = 0; i < Math.abs(index - selected); i++) {
          await client.key(index > selected ? '\uf701' : '\uf700');
        }
        await client.key('\n');
      }
      assert((await profile()).accessibleValue.startsWith(labels[index]));
      await client.setValue(await client.find({ label: 'Export end frame (exclusive)', role: 'TextInput' }), '6');
      await client.setValue(await client.find({ label: 'File path', role: 'TextInput' }), target);
      await click('Export MP4');
      await client.waitFor(async () => {
        const current = await state();
        assert(!current.includes('Export failed'), current);
        return current.includes('Export complete') && current.includes(name + '.mp4');
      }, { description: name + ' delivery' });
      const probe = spawnSync(ffmpeg, ['-hide_banner', '-i', target, '-vf', 'showinfo', '-f', 'null', '-'],
        { env: { ...process.env, DYLD_LIBRARY_PATH: '', DYLD_FALLBACK_LIBRARY_PATH: '' }, encoding: 'utf8' });
      assert.equal(probe.status, 0, probe.stderr);
      assert.match(probe.stderr, codec);
      assert.match(probe.stderr, /Audio: aac/);
      assert(probe.stderr.includes(transfer), probe.stderr);
      assert.equal([...probe.stderr.matchAll(/\bn:\s+\d+ pts:/g)].length, 6);
      if (name === 'hdr10') assert((await readFile(target)).includes(Buffer.from('mdcv')));
    }
    const cancelled = join(output, 'cancelled-delivery.mp4');
    await click('EXPORT');
    await client.setValue(await client.find({ label: 'Export end frame (exclusive)', role: 'TextInput' }), '90');
    await client.setValue(await client.find({ label: 'File path', role: 'TextInput' }), cancelled);
    await click('Export MP4'); await waitState('Exporting MP4');
    await click('Cancel export'); await waitState('Export cancelled');
    const files = await readdir(output);
    assert(!files.includes('cancelled-delivery.mp4'));
    assert(!files.some(name => name.startsWith('.oq-export-')));
    await fileAction('SAVE AS', join(output, 'delivery.oqvideo.json'), 'Save project');
    await waitState('Project saved'); await close();
  }, { core: true });
}

async function proxyCacheAndCancellation() {
 await step('Reuse the verified proxy after reopening and cancel a proxy batch', async () => {
   const proxyDirectory = join(output, 'proxies');
   const entries = async () => (await readdir(proxyDirectory)).sort();
   const cached = (await entries()).filter(name => /^[a-f0-9]{64}\.mp4$/.test(name));
   assert.equal(cached.length, 1);
   await launch(join(output, 'proxy-vfr.oqvideo.json'));
   await waitState('clips 1');
   await client.click(await assetCard('source-time.mp4'));
   await click('Prepare proxy'); await waitState('Proxy ready');
   assert.deepEqual((await entries()).filter(name => /^[a-f0-9]{64}\.mp4$/.test(name)), cached);

   const red = generate('proxy-batch-red.mp4', 'red', 12, 30, '1920x1080');
   const blue = generate('proxy-batch-blue.mp4', 'blue', 12, 30, '1920x1080');
   await click('Import files');
   const input = await client.find({ label: 'Import file paths', role: 'TextInput' });
   await client.setValue(input, [red, blue].join('\n'));
   await click('Import selected files'); await waitState('Imported 2');
   await click('Prepare all proxies');
   await click('Cancel proxy'); await waitState('Proxy batch cancelled');
   const remaining = await entries();
   assert(cached.every(name => remaining.includes(name)), 'Cancellation removed a verified cached proxy');
   assert(!remaining.some(name => name.startsWith('.building')), 'Cancellation left a partial proxy file');
   await client.click(await assetCard('proxy-batch-blue.mp4'));
   await expectColor('blue');
   await fileAction('SAVE AS', join(output, 'proxy-cancel.oqvideo.json'), 'Save project');
   await waitState('Project saved');
   await close();
 });
}

async function runVideoCases() {
  const red = generate('red.mp4', 'red'), blue = generate('blue.mp4', 'blue');
 await step('Empty native window',async()=>{await launch();await waitState('assets 0; clips 0');await click('EXPORT');await waitState('Add media to the timeline before exporting');await snapshot('01-empty');}, { core: true });
 await step('Frameless window controls follow configuration and maximize/restore',async()=>{
  const labels=['Close window','Minimize window','Maximize window'];
  const controls=await Promise.all(labels.map(label=>client.find({label,role:'Button'})));
  const before=await client.windowProperties();
  const logicalWidth=before.size.width/(before.scaleFactor||1);
  const placement=process.env.OPEN_QUARTZ_WINDOW_CONTROLS||'platform';
  const left=placement==='left'||(placement==='platform'&&process.platform==='darwin');
  for(const control of controls)assert.equal(control.absolutePosition.x+control.size.width/2<logicalWidth/2,left);
  const ordered=controls.toSorted((a,b)=>a.absolutePosition.x-b.absolutePosition.x).map(control=>control.accessibleLabel);
  assert.deepEqual(ordered,process.platform==='darwin'?labels:[labels[1],labels[2],labels[0]]);
  await click('Maximize window');
  const maximized = await client.waitFor(async () => {
    const window = await client.windowProperties();
    const restore = await client.byLabel('Restore window', { role: 'Button' });
    return restore.length === 1 && (window.size.width > before.size.width || window.size.height > before.size.height) && window.size;
  }, { description: 'native window grows and exposes Restore' });
  // The OS window geometry can update before the animation finishes and is
  // ready for another command.
  await delay(350);
  await click('Restore window');
  await client.waitFor(async () => {
    const { size } = await client.windowProperties();
    const maximize = await client.byLabel('Maximize window', { role: 'Button' });
    return maximize.length === 1 && (size.width < maximized.width || size.height < maximized.height);
  }, { description: 'native window returns to its resizable size' });
  await snapshot('01-window-controls');
 }, { core: true });
 await step('Sequence settings',async()=>{await click('SEQUENCE');for(const [label,value]of [['Sequence width','640'],['Sequence height','360'],['FPS numerator','30'],['FPS denominator','1']])await client.setValue(await client.find({label,role:'TextInput'}),value);await click('Apply settings');await snapshot('02-sequence');}, { core: true });
 await step('Import red on hardware and place clip',async()=>{await fileAction('IMPORT',red,'Import media');await waitState('source PTS 0 ns · CPU image bytes 0');await snapshot('03-red-media','red');await click('ADD CLIP');await waitState('clips 1');}, { core: true });
 await step('Import blue and verify exact adjacent placement via media plus',async()=>{await fileAction('IMPORT',blue,'Import media');await waitState('assets 2');await waitState('source PTS 0 ns · CPU image bytes 0');await client.click(await assetCard('red.mp4'));await client.click(await assetCard('blue.mp4'));await client.waitFor(async()=>previewHasColor(await client.screenshot(),'blue'),{description:'selected blue media preview'});await snapshot('04-blue-media','blue');await click('Add blue.mp4 to timeline');await waitState('clips 2');for(let n=0;n<6;n++)await click('ZOOM OUT');assert((await timeline('blue.mp4'))[0].accessibleDescription.includes('start 90 frames; duration 90'));}, { core: true });
 await step('Move with one undo/redo; black gap',async()=>{const item=(await timeline('blue.mp4'))[0];await client.drag(item,{x:item.absolutePosition.x+item.size.width/2+40,y:item.absolutePosition.y+item.size.height/2});await client.waitFor(async()=> (await timeline('blue.mp4'))[0].accessibleDescription.includes('start 100 frames'),{description:'clip move'});await click('UNDO');assert((await timeline('blue.mp4'))[0].accessibleDescription.includes('start 90 frames'));await click('REDO');assert((await timeline('blue.mp4'))[0].accessibleDescription.includes('start 100 frames'));await waitState('source PTS gap ns');await snapshot('05-gap','black');}, { core: true });
 await step('Trim both edges with actual gestures',async()=>{for(const [edge,delta,expected]of [['start',40,'start 10 frames; duration 80'],['end',-40,'start 10 frames; duration 70']]){const item=await client.find({label:`Trim ${edge} of red.mp4`,role:'Button'});await client.drag(item,{x:item.absolutePosition.x+item.size.width/2+delta,y:item.absolutePosition.y+item.size.height/2});await client.waitFor(async()=> (await timeline('red.mp4'))[0].accessibleDescription.includes(expected),{description:`trim ${edge}`});}}, { core: true });
 await step('Split at frame45, lift and undo',async()=>{await client.click((await timeline('red.mp4'))[0]);for(let n=0;n<35;n++)await click('NEXT');await waitState('playhead 45 frames');await click('SPLIT');await waitState('clips 3');assert.deepEqual((await timeline('red.mp4')).map(i=>i.accessibleDescription),['Timeline start 10 frames; duration 35 frames','Timeline start 45 frames; duration 35 frames']);await click('DELETE');await waitState('clips 2');await click('UNDO');await waitState('clips 3');await snapshot('06-split');}, { core: true });
 await step('Inspector overlap rejection leaves clip unchanged',async()=>{await client.click((await timeline('blue.mp4'))[0]);await client.waitFor(async()=> (await client.find({label:'Timeline start',role:'TextInput'})).accessibleValue==='100',{description:'selected blue clip timing is published'});const input=await client.find({label:'Timeline start',role:'TextInput'});await client.click(input);await client.setValue(input,'20');await client.key('\n');await waitState('overlaps');assert((await timeline('blue.mp4'))[0].accessibleDescription.includes('start 100 frames'));});
 await step('Cancel active drag with Escape',async()=>{const item=(await timeline('blue.mp4'))[0];const moving=client.drag(item,{x:item.absolutePosition.x+item.size.width/2+160,y:item.absolutePosition.y+item.size.height/2});await delay(120);await client.key('\u001b');await moving;assert((await timeline('blue.mp4'))[0].accessibleDescription.includes('start 100 frames'));});
 await step('Play and pause real GPU video',async()=>{await client.click((await timeline('blue.mp4'))[0]);await click('PLAY');await client.waitFor(async()=>/playhead 1[1-7]\d frames/.test(await state()),{description:'video playback'});await click('PAUSE');await snapshot('07-playing-paused','blue');const paused=await state();await delay(200);assert.equal((await state()).match(/playhead (\d+)/)[1],paused.match(/playhead (\d+)/)[1]);}, { core: true });
 await step('Save project through UI and verify source timestamps',async()=>{await fileAction('SAVE AS',project,'Save project');await waitState('Project saved');const data=JSON.parse(await readFile(project,'utf8')).data;assert.equal(data.width,640);assert.equal(data.height,360);assert.deepEqual(data.tracks.flatMap(track => track.clips).map(c=>[c.timeline_start,c.duration]),[[10,35],[45,35],[100,90]]);const asset=data.assets.find(a=>a.name==='red.mp4');assert.equal(data.tracks.flatMap(track => track.clips)[0].source_in_pts,asset.start_pts+asset.time_base.denominator/asset.time_base.numerator/3);assert(!data.assets[0].path.startsWith('/'));await close();}, { core: true });
 await step('Reopen, detect missing media and reject wrong relink',async()=>{await rename(red,join(output,'red-relocated.mp4'));await launch(project);await waitState('clips 3');const item=(await client.byLabel('red.mp4',{role:'Button'})).find(i=>i.accessibleDescription?.includes('; offline'));assert(item);await client.click(item);await waitState('Media offline');await snapshot('08-offline');await fileAction('RELINK',blue,'Relink media');await waitState('does not match');assert((await client.byLabel('red.mp4',{role:'Button'})).some(i=>i.accessibleDescription?.includes('; offline')));}, { core: true });
 await step('Relink matching original and restore hardware image',async()=>{await fileAction('RELINK',join(output,'red-relocated.mp4'),'Relink media');await client.waitFor(async()=> (await buttons('red.mp4')).some(i=>i.accessibleDescription?.includes('; online')),{description:'matched relink'});const item=(await buttons('red.mp4')).find(i=>i.accessibleDescription?.includes('; online'));await client.click(item);await waitState('source PTS 0 ns · CPU image bytes 0');await snapshot('09-restored','red');await click('SAVE');await waitState('Project saved');await close();}, { core: true });
 if (previewOnly) {
   await linkedAudioPreview();
   await hdrPreviewRoundTrip();
   await maskTrackingRoundTrip();
   await proxyRoundTrip();
   await proxyCacheAndCancellation();
   return;
 }
 await step('Long media plus preserves full duration beyond GPU texture limits',async()=>{
  const longMedia=generate('long.mp4','red',30);
  const longProject=join(output,'long.oqvideo.json');
  await launch();
  await fileAction('IMPORT',longMedia,'Import media');
  await waitState('source PTS 0 ns · CPU image bytes 0');
  await click('Add long.mp4 to timeline');
  await waitState('clips 1');
  const placed=(await timeline('long.mp4'))[0];
  assert.equal(placed.accessibleDescription,'Timeline start 0 frames; duration 900 frames');
  assert.equal(placed.size.width,900*16);
  await snapshot('10-long-default-zoom','red');
  for(let n=0;n<16;n++)await click('ZOOM IN');
  assert.equal((await timeline('long.mp4'))[0].size.width,900*48);
  await snapshot('11-long-maximum-zoom','red');
  await click('UNDO');await waitState('clips 0');
  await click('REDO');await waitState('clips 1');
  await client.waitFor(async()=>previewHasColor(await client.screenshot(),'red'),{description:'long clip preview after redo'});
  await snapshot('12-long-redo','red');
  await click('FIT TIMELINE');
  const fitted=(await timeline('long.mp4'))[0];
  const window=await client.windowProperties();
  assert(fitted.absolutePosition.x>=84&&fitted.absolutePosition.x+fitted.size.width<=window.size.width/(window.scaleFactor||1),'Fit must show both clip boundaries');
  await click('ZOOM OUT');
  const overview=(await timeline('long.mp4'))[0];
  const pixelsPerFrame=overview.size.width/900;
  assert(pixelsPerFrame<1,'Overview must support fractional pixels per frame');
  const trimStart=await client.find({label:'Trim start of long.mp4',role:'Button'});
  const trimmedFrames=Math.round(20/pixelsPerFrame);
  await client.drag(trimStart,{x:trimStart.absolutePosition.x+trimStart.size.width/2+20,y:trimStart.absolutePosition.y+trimStart.size.height/2});
  await client.waitFor(async()=> (await timeline('long.mp4'))[0].accessibleDescription===`Timeline start ${trimmedFrames} frames; duration ${900-trimmedFrames} frames`,{description:'fractional zoom trim maps pixels to source frames'});
  await click('UNDO');
  await client.waitFor(async()=> (await timeline('long.mp4'))[0].accessibleDescription==='Timeline start 0 frames; duration 900 frames',{description:'undo overview trim restores exact duration'});
  await click('FIT TIMELINE');await snapshot('12-long-fit');
  await fileAction('SAVE AS',longProject,'Save project');await waitState('Project saved');
  assert.deepEqual(JSON.parse(await readFile(longProject,'utf8')).data.tracks.flatMap(track => track.clips).map(clip=>[clip.timeline_start,clip.duration]),[[0,900]]);
  await close();
  await launch(longProject);await waitState('clips 1');
  assert.equal((await timeline('long.mp4'))[0].accessibleDescription,'Timeline start 0 frames; duration 900 frames');
  await waitState('source PTS 0 ns · CPU image bytes 0');
  await snapshot('13-long-reopened','red');await close();
 });
 const multiProject = join(output, 'multitrack.oqvideo.json');
 const background = generate('background.mp4', 'blue', 5, 30, '1920x1080');
 const foreground = generate('foreground.mp4', 'red', 5, 24, '1920x1080');
 const item = async name => (await timeline(name))[0];
 await step('Drag media to the upper video track and render both layers', async () => {
   await launch();
   await fileAction('IMPORT', background, 'Import media'); await waitState('assets 1');
   await click('ADD CLIP'); await waitState('clips 1');
   await fileAction('IMPORT', foreground, 'Import media'); await waitState('assets 2');
   await click('FIT TIMELINE');
   await click('ADD TRACK');
   const track = await grip('Video 2');
   const base = await item('background.mp4');
   await client.drag(await assetCard('foreground.mp4'), {
     x: base.absolutePosition.x + 1,
     y: center(track).y + 24,
   });
   await waitState('clips 2');
   await waitState('source PTS Video 2 0, Video 1 0');
   assert((await item('foreground.mp4')).absolutePosition.y < base.absolutePosition.y);
   await expectColor('red');
 }, { core: true });
 await step('Viewer grips resize and drag the overlay with one undo per gesture', async () => {
   await click('TRANSFORM');
   const resize = await client.find({ label: 'Resize bottom right', role: 'Button' });
   const corner = center(resize);
   await client.drag(resize, { x: corner.x - 70, y: corner.y - 40 });
   const cornerPosition = async () => {
     const handles = await client.byLabel('Resize bottom right', { role: 'Button' });
     return handles[0] ? center(handles[0]) : undefined;
   };
   await client.waitFor(async () => (await cornerPosition())?.x < corner.x - 45,
     { description: 'bottom-right handle follows resized layer' });
   const smaller = await cornerPosition();
   const monitor = await client.find({ label: 'Video preview', role: 'Image' });
   const middle = center(monitor);
   await client.drag(monitor, { x: middle.x + 25, y: middle.y + 12 });
   await client.waitFor(async () => (await cornerPosition())?.x >= smaller.x + 18,
     { description: 'monitor drag moves the selected image' });
   await click('UNDO');
   await client.waitFor(async () => Math.abs((await cornerPosition())?.x - smaller.x) < 2,
     { description: 'undo restores the position without losing resize' });
   await click('REDO');
   await client.waitFor(async () => (await cornerPosition())?.x >= smaller.x + 18,
     { description: 'redo restores the preview move' });
   await click('TRANSFORM');
   await snapshot('14-picture-in-picture');
 }, { core: true });
 await step('Drag a track header to reorder, hide and undo without moving clip timing', async () => {
   const top = await grip('Video 2');
   const lower = await grip('Video 1');
   await client.drag(top, { x: center(top).x, y: center(lower).y + 52 });
   await expectColor('blue');
   assert((await item('foreground.mp4')).absolutePosition.y > (await item('background.mp4')).absolutePosition.y);
   await click('Hide Video 1'); await expectColor('red');
   await click('UNDO'); await expectColor('blue');
   await click('UNDO'); await expectColor('red');
   assert((await item('foreground.mp4')).accessibleDescription.includes('Timeline start 0 frames'));
 }, { core: true });
 await step('Cross-track drag keeps clip identity and a locked target rejects drops', async () => {
   await click('ADD TRACK');
   const upper = await grip('Video 3');
   const source = await item('foreground.mp4');
   const [x, y] = [center(source).x, center(upper).y + 24];
   await click('Lock Video 3');
   await client.drag(source, { x, y }); await waitState('Video 3 is locked');
   assert.equal((await item('foreground.mp4')).absolutePosition.y, source.absolutePosition.y);
   await click('Unlock Video 3');
   await client.drag(await item('foreground.mp4'), { x, y });
   await client.waitFor(async () => (await item('foreground.mp4')).absolutePosition.y < source.absolutePosition.y,
     { description: 'foreground clip moves into the top lane' });
   await fileAction('SAVE AS', multiProject, 'Save project'); await waitState('Project saved');
   const { data } = JSON.parse(await readFile(multiProject, 'utf8'));
   assert.equal(data.schema_version, 27);
   assert.deepEqual(data.tracks.map(track => [track.name, track.clips.length]), [['Video 3', 1], ['Video 2', 0], ['Video 1', 1]]);
   assert.equal(data.tracks[0].clips[0].timeline_start, 0);
   assert(data.tracks[0].clips[0].transform.scale < 1);
   await close();
   await launch(multiProject); await waitState('clips 2');
   await expectColor('red');
   await click('PLAY');
   await waitState('Frame 149 · source PTS Video 3 4958333333, Video 1 4966666667 ns');
   await expectColor('red');
   await snapshot('15-multitrack-reopened');
   await close();
 }, { core: true });
 const exportProject = join(output, 'export.oqvideo.json');
 const exported = join(output, 'sequence.mp4');
 await step('Export the sequence and decode every frame across gaps, cuts and track overlaps', async () => {
   const fixture = JSON.parse(await readFile(multiProject, 'utf8'));
   const [foregroundTrack, , backgroundTrack] = fixture.data.tracks;
   const [overlay] = foregroundTrack.clips;
   Object.assign(fixture.data, { width: 320, height: 180 });
   Object.assign(backgroundTrack.clips[0], { duration: 9 });
   Object.assign(overlay, { timeline_start: 3, duration: 3 });
   Object.assign(overlay.transform, { x: 0, y: 0, scale: 0.5 });
   foregroundTrack.clips.push({
     ...overlay, id: 'export-cut', timeline_start: 12, duration: 6,
     transform: { ...overlay.transform, scale: 1 },
   });
   await writeFile(exportProject, JSON.stringify(fixture));
   await launch(exportProject);
   await waitState('clips 3');
   await fileAction('EXPORT', exported, 'Export MP4');
   await waitState('Export complete');
   const ffmpegEnv = { ...process.env, DYLD_LIBRARY_PATH: '', DYLD_FALLBACK_LIBRARY_PATH: '' };
   const metadata = spawnSync(ffmpeg, ['-hide_banner', '-i', exported, '-map', '0:v:0', '-f', 'null', '-'],
     { env: ffmpegEnv, encoding: 'utf8' });
   assert.equal(metadata.status, 0, metadata.stderr);
   assert.match(metadata.stderr, /Video: h264[^\n]*320x180[^\n]*30 fps/);
   const width = 320, height = 180, stride = width * height * 3;
   const pixels = execFileSync(ffmpeg, ['-hide_banner', '-loglevel', 'error', '-i', exported,
     '-map', '0:v:0', '-vsync', '0', '-pix_fmt', 'rgb24', '-f', 'rawvideo', '-'],
     { env: ffmpegEnv, maxBuffer: stride * 24 });
   assert.equal(pixels.length, stride * 18, 'every sequence frame encoded once at canvas size');
   const colors = { red: [255, 0, 0], blue: [0, 0, 255], black: [0, 0, 0] };
   function check(frame, x, y, color) {
     const start = frame * stride + (y * width + x) * 3;
     const actual = [...pixels.subarray(start, start + 3)];
     assert(actual.every((channel, index) => Math.abs(channel - colors[color][index]) < 26),
       `frame ${frame} at ${x},${y} expected ${color}, got ${actual}`);
   }
   for (let frame = 0; frame < 18; frame++) {
     check(frame, 160, 90, frame >= 12 || (frame >= 3 && frame < 6) ? 'red' : frame >= 9 ? 'black' : 'blue');
     check(frame, 24, 24, frame >= 12 ? 'red' : frame >= 9 ? 'black' : 'blue');
   }
   await snapshot('16-export-complete');
   await close();
 }, { core: true });
 await step('Export validates paths and preserves existing destinations', async () => {
   await launch(exportProject);
   await waitState('clips 3');
   await fileAction('EXPORT', 'relative.mov', 'Export MP4');
   await waitState('Enter an absolute output path ending in .mp4');
   await click('Cancel');
   assert.equal((await client.byLabel('Export MP4', { role: 'Button' })).length, 0);
   const original = await readFile(exported);
   await fileAction('EXPORT', exported, 'Export MP4');
   await waitState('Export target already exists');
   assert.deepEqual(await readFile(exported), original);
   await close();
 });
 await step('Cancel an active export and wait for its output cleanup', async () => {
   await launch(multiProject);
   await waitState('clips 2');
   const cancelled = join(output, 'cancelled.mp4');
   const filesBefore = (await readdir(output)).sort();
   await fileAction('EXPORT', cancelled, 'Export MP4');
   await client.waitFor(async () => /Exporting MP4 · [1-9]\d* \/ 150 frames/.test(await state()),
     { description: 'encoder has started accepting the sequence frames' });
   await click('Close window');
   await waitState('Wait for the export to finish or cancel it before closing');
   await click('Cancel export');
   await waitState('Export cancelled');
   assert.deepEqual((await readdir(output)).sort(), filesBefore, 'no partial destination or temporary file survives');
   await close();
 });
 await step('Export rejects offline media in a later region and leaves no output', async () => {
   const missing = join(output, 'missing-export-source.mp4');
   const offlineProject = join(output, 'offline-export.oqvideo.json');
   const fixture = JSON.parse(await readFile(exportProject, 'utf8'));
   const source = fixture.data.tracks[0].clips[0].asset_id;
   fixture.data.assets.find(asset => asset.id === source).path = missing;
   await writeFile(offlineProject, JSON.stringify(fixture));
   await launch(offlineProject);
   await waitState('clips 3');
   const destination = join(output, 'offline.mp4');
   const filesBefore = (await readdir(output)).sort();
   await fileAction('EXPORT', destination, 'Export MP4');
   await waitState('Export failed · Media offline');
   assert.deepEqual((await readdir(output)).sort(), filesBefore);
   await close();
 });
 const audioProject = join(output, 'audio.oqvideo.json');
 const audioOutput = join(output, 'audio-mix.mp4');
 const mediaEnv = { ...process.env, DYLD_LIBRARY_PATH: '', DYLD_FALLBACK_LIBRARY_PATH: '' };
 const mediaCommand = (args, options = {}) => execFileSync(ffmpeg, ['-hide_banner', '-loglevel', 'error', ...args], { env: mediaEnv, ...options });
 const pcm = path => {
   const bytes = mediaCommand(['-i', path, '-map', '0:a:0', '-f', 'f32le', '-ac', '2', '-ar', '48000', '-'],
     { maxBuffer: 4 * 1024 * 1024 });
   return new Float32Array(bytes.buffer, bytes.byteOffset, bytes.byteLength / 4);
 };
 const amplitude = (samples, seconds, frequency) => {
   const start = Math.round(seconds * 48000);
   const count = 4800;
   let real = 0, imaginary = 0;
   for (let i = 0; i < count; i++) {
     const phase = 2 * Math.PI * i * frequency / 48000;
     real += samples[(start + i) * 2] * Math.cos(phase);
     imaginary += samples[(start + i) * 2] * Math.sin(phase);
   }
   return 2 * Math.hypot(real, imaginary) / count;
 };
 const writeAudioFixture = () => {
   const linked = join(output, 'linked-tone.mp4'), independent = join(output, 'independent-tone.wav');
   const picture = generate('audio-picture.mp4', 'red');
   mediaCommand(['-i', picture, '-itsoffset', '0.25', '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000:duration=2.75',
     '-map', '0:v:0', '-map', '1:a:0', '-c:v', 'copy', '-c:a', 'aac', '-y', linked]);
   mediaCommand(['-f', 'lavfi', '-i', 'sine=frequency=880:sample_rate=44100:duration=3', '-c:a', 'pcm_s16le', '-y', independent]);
   return { linked, independent };
 };
 const audioClip = async filename => client.find({ label: `Audio clip ${filename}`, role: 'Button', visible: false });
 const waitAudio = async () => client.waitFor(async () => /Audio output [1-9][0-9]*%/.test(await state()),
   { description: 'the device is receiving non-silent mixed PCM' });
 await step('Linked audio and a resampled WAV play together and export as stereo AAC', async () => {
   const { linked, independent } = writeAudioFixture();
   await launch();
   await fileAction('IMPORT', linked, 'Import media'); await waitState('assets 1');
   await click('ADD CLIP'); await waitState('clips 1');
   await client.waitFor(async () => (await audioClip('linked-tone.mp4')).accessibleDescription.endsWith('linked video'),
     { description: 'linked sound is present on the timeline' });
   await fileAction('IMPORT', independent, 'Import media'); await waitState('assets 2');
   await click('Add independent-tone.wav to timeline');
   await waitState('audio clips 2');
   const audioTimeline = await client.find({ id: 'EditorWindow::timeline_scroll' });
   await client.value('dispatch_pointer_scroll', { windowHandle: client.windowId, deltaX: 0, deltaY: -170,
     position: { x: audioTimeline.absolutePosition.x + 100, y: audioTimeline.absolutePosition.y + 50 } });
   assert((await audioClip('independent-tone.wav')).accessibleDescription.startsWith('0 samples to 144000'));
   await click('PLAY');
   await waitAudio();
   await client.waitFor(async () => /playhead (?:[2-9]|[1-9]\d) frames/.test(await state()),
     { description: 'video playhead follows audio device playback' });
   await click('PAUSE');
   await client.waitFor(async () => !(await state()).includes('Audio output'), { description: 'pause stops audio output' });
   await fileAction('SAVE AS', audioProject, 'Save project'); await waitState('Project saved');
   await fileAction('EXPORT', audioOutput, 'Export MP4'); await waitState('Export complete');
   const streams = spawnSync(ffmpeg, ['-hide_banner', '-i', audioOutput, '-map', '0:v:0', '-t', '0', '-f', 'null', '-'],
     { env: mediaEnv, encoding: 'utf8' });
   assert.equal(streams.status, 0, streams.stderr);
   assert.match(streams.stderr, /Audio: aac[^\n]*48000 Hz, stereo/);
   const samples = pcm(audioOutput);
   assert(samples.length / 2 >= 144000 && samples.length / 2 < 145024, 'stereo audio duration matches 90 video frames with at most an AAC tail block');
   assert(amplitude(samples, 0.04, 880) > 0.04, 'WAV is present before linked audio starts');
   assert(amplitude(samples, 0.04, 440) < 0.006, 'delayed linked audio starts at its media timestamp');
   assert(amplitude(samples, 0.45, 880) > 0.04, 'resampled WAV remains audible during overlap');
   assert(amplitude(samples, 0.45, 440) > 0.04, 'linked AAC is audible during overlap');
   await snapshot('17-audio-mix');
   await close();
 }, { core: true });
 const audioGestureProject = join(output, 'audio-gesture.oqvideo.json');
 const audioMediaName = 'independent-tone.wav';
 const audioMoveHandle = () => client.waitFor(async () => {
   try { return (await client.byLabel('Audio clip ' + audioMediaName, { role: 'Button' }))[0]; }
   catch (error) { if (!/destroyed|stale/i.test(error.message)) throw error; }
 }, { description: 'independent audio timeline item' });
 const audioLaneScroll = async () => {
   const view = await client.find({ id: 'EditorWindow::timeline_scroll' });
   await client.value('dispatch_pointer_scroll', { windowHandle: client.windowId, deltaX: 0, deltaY: -170, position: center(view) });
 };
 const dragAudio = async (element, pixels) => {
   const from = center(element);
   await client.drag(element, { x: from.x + pixels, y: from.y });
 };
 const audioRange = async (start, end) => client.waitFor(async () => {
   const description = (await audioMoveHandle()).accessibleDescription;
   return description === `${start} samples to ${end}`;
 }, { description: `audio range ${start} through ${end}` });
 await step('Edit the waveform on the audio timeline and export the same fades', async () => {
   const fixture = JSON.parse(await readFile(audioProject, 'utf8'));
   fixture.data.width = 320;
   fixture.data.height = 180;
   fixture.data.audio_tracks[0].muted = true;
   fixture.data.audio_tracks[1].clips[0].placement.duration = 48000;
   const seed = join(output, 'audio-gesture-seed.oqvideo.json');
   await writeFile(seed, JSON.stringify(fixture));
   await launch(seed); await waitState('audio clips 2');
   await audioLaneScroll();
   await client.waitFor(async () => (await client.byLabel('Waveform ' + audioMediaName, { role: 'Image' })).length === 1,
     { description: 'decoded waveform is visible on the audio clip' });
   await snapshot('17-audio-waveform');
   const px = (await audioMoveHandle()).size.width / 30;
   await dragAudio(await audioMoveHandle(), 3 * px); await audioRange(4800, 52800);
   const trim = edge => client.find({ label: `Trim audio ${edge} of ${audioMediaName}`, role: 'Button' });
   await dragAudio(await trim('start'), 3 * px); await audioRange(9600, 52800);
   await dragAudio(await trim('end'), -3 * px); await audioRange(9600, 48000);

   const fade = side => client.find({ label: `Audio fade ${side} ${audioMediaName}`, role: 'Button' });
   await dragAudio(await fade('in'), 6 * px);
   const field = label => client.find({ label, role: 'TextInput', visible: false });
   await client.waitFor(async () => (await field('Audio fade in (samples)')).accessibleValue === '9600',
     { description: 'fade-in handle writes a 9600-sample ramp' });
   await dragAudio(await fade('out'), -6 * px);
   await client.waitFor(async () => (await field('Audio fade out (samples)')).accessibleValue === '9600',
     { description: 'fade-out handle writes a 9600-sample ramp' });
   await click('UNDO');
   await client.waitFor(async () => (await field('Audio fade out (samples)')).accessibleValue === '0',
     { description: 'undo removes one fade gesture' });
   await click('REDO');
   await client.waitFor(async () => (await field('Audio fade out (samples)')).accessibleValue === '9600',
     { description: 'redo preserves the fade duration' });

   await fileAction('SAVE AS', audioGestureProject, 'Save project'); await waitState('Project saved');
   const saved = JSON.parse(await readFile(audioGestureProject, 'utf8')).data.audio_tracks[1].clips[0];
   assert.deepEqual(saved.placement, { kind: 'independent', start: 9600, duration: 38400, source: 4800 });
   assert.deepEqual([saved.fade_in, saved.fade_out], [9600, 9600]);
   const path = join(output, 'audio-waveform-edits.mp4');
   await fileAction('EXPORT', path, 'Export MP4'); await waitState('Export complete');
   const samples = pcm(path);
   const middle = amplitude(samples, .5, 880);
   assert(middle > .07, 'the middle of the clip plays at full gain');
   assert(amplitude(samples, .06, 880) < .002, 'silence before the moved and trimmed start');
   assert(amplitude(samples, .2, 880) < middle * .45, 'fade in reduces the opening amplitude');
   assert(amplitude(samples, .894, 880) < middle * .45, 'fade out reduces the closing amplitude');
   assert(amplitude(samples, 1.04, 880) < .002, 'silence after the trimmed end');
   await close();
 }, { core: true });
 await step('Split an independent audio clip and protect linked and locked clips from gestures', async () => {
   await launch(audioGestureProject); await waitState('audio clips 2');
   await audioLaneScroll();
   const clip = await audioMoveHandle();
   const [next] = await buttons('NEXT');
   await client.click(clip);
   const numericFade = await client.waitFor(async () => {
     try { return (await client.byLabel('Audio fade in (samples)', { role: 'TextInput' }))[0]; }
     catch (error) { if (!/destroyed|stale/i.test(error.message)) throw error; }
   }, { description: 'audio properties after selection' });
   await client.click(numericFade); await client.setValue(numericFade, '8000'); await client.key('\n');
   await client.waitFor(async () => (await client.properties(numericFade.handle)).accessibleValue === '8000',
     { description: 'sample-precise fade field' });
   await click('UNDO');
   await client.waitFor(async () => (await client.properties(numericFade.handle)).accessibleValue === '9600',
     { description: 'undo restores the typed fade' });
   const zoom = clip.size.width / 24;
   for (let i = 0; i < 15; i++) await client.click(next);
   await waitState('playhead 15 frames');
   await click('SPLIT'); await waitState('audio clips 3');
   const rows = await buttons('Audio clip ' + audioMediaName);
   assert.deepEqual(rows.map(row => row.accessibleDescription), ['9600 samples to 24000', '24000 samples to 48000']);
   await click('UNDO'); await audioRange(9600, 48000);
   await click('Lock Audio 2');
   await dragAudio(await audioMoveHandle(), zoom * 4); await audioRange(9600, 48000);
   assert.equal((await buttons('Trim audio start of ' + audioMediaName)).length, 0);
   await click('Unlock Audio 2');
   const linked = await audioClip('linked-tone.mp4');
   await dragAudio(linked, zoom * 4);
   assert((await audioClip('linked-tone.mp4')).accessibleDescription.startsWith('0 samples to 144000'));
   assert.equal((await buttons('Trim audio start of linked-tone.mp4')).length, 0);
   await close();
 });
 const rangeProject = join(output, 'range-seed.oqvideo.json');
 const rangeStart = () => client.find({ label: 'Export start frame', role: 'TextInput' });
 const rangeEnd = () => client.find({ label: 'Export end frame (exclusive)', role: 'TextInput' });
 const exportRange = async (path, start, end) => {
   await click('EXPORT');
   await client.setValue(await rangeStart(), String(start));
   await client.setValue(await rangeEnd(), String(end));
   await client.setValue(await client.find({ label: 'File path', role: 'TextInput' }), path);
   await click('Export MP4');
 };
 await step('Export a nonzero frame range with matching AAC and a zero-based output clock', async () => {
   const fixture = JSON.parse(await readFile(audioProject, 'utf8'));
   const videoFixture = JSON.parse(await readFile(exportProject, 'utf8')).data;
   const blueAsset = videoFixture.assets.find(asset => asset.name === 'background.mp4');
   const blueTrack = videoFixture.tracks.find(track => track.clips.some(clip => clip.asset_id === blueAsset.id));
   const blueClip = blueTrack.clips[0];
   const [base] = fixture.data.tracks;
   fixture.data.assets.push(blueAsset);
   fixture.data.tracks = [
     { ...blueTrack, id: 'range-upper', clips: [{ ...blueClip, timeline_start: 18, duration: 4 }] },
     base,
   ];
   fixture.data.width = 320;
   fixture.data.height = 180;
   const wav = fixture.data.audio_tracks.flatMap(track => track.clips)
     .find(clip => clip.placement.kind === 'independent');
   wav.placement.start = 32000;
   await writeFile(rangeProject, JSON.stringify(fixture));

   await launch(rangeProject); await waitState('clips 2');
   const path = join(output, 'sequence-range.mp4');
   await exportRange(path, 12, 26); await waitState('Export complete');
   assert((await state()).includes('playhead 0 frames'), 'export range did not seek or trim the project');
   const stride = 320 * 180 * 3;
   const pixels = mediaCommand(['-i', path, '-map', '0:v:0', '-vsync', '0',
     '-pix_fmt', 'rgb24', '-f', 'rawvideo', '-'], { maxBuffer: stride * 18 });
   assert.equal(pixels.length, stride * 14, 'the exclusive end produces exactly fourteen frames');
   for (let index = 0; index < 14; index++) {
     const pixel = index * stride + (90 * 320 + 160) * 3;
     const [r,g,b] = pixels.subarray(pixel, pixel + 3);
     const blue = index >= 6 && index < 10;
     assert(blue ? b > 225 && g < 25 && r < 25 : r > 225 && g < 25 && b < 25,
       `frame ${index} samples absolute sequence frame ${12 + index}, got RGB ${r},${g},${b}`);
   }
   const timestamps = spawnSync(ffmpeg, ['-hide_banner', '-i', path, '-an', '-vf', 'showinfo', '-f', 'null', '-'],
     { env: mediaEnv, encoding: 'utf8' });
   assert.equal(timestamps.status, 0, timestamps.stderr);
   assert.match(timestamps.stderr, /n:\s*0\s+pts:\s*0\s+pts_time:0(?:\s|$)/);
   const samples = pcm(path);
   assert(samples.length / 2 >= 22400 && samples.length / 2 < 23424, 'AAC matches the export range with at most an AAC tail block');
   assert(amplitude(samples, 0.03, 440) > 0.04, 'the linked tone is already playing at the range start');
   assert(amplitude(samples, 0.03, 880) < 0.006, 'the independent WAV does not start before its range-relative onset');
   assert(amplitude(samples, 0.34, 880) > 0.04, 'the independent WAV starts at the correct offset in the output');
   await close();
 }, { core: true });
 await step('Reject invalid export frames and preserve drafts until cancellation', async () => {
   await launch(rangeProject); await waitState('clips 2');
   await click('EXPORT');
   assert.equal((await rangeStart()).accessibleValue, '0');
   const duration = Number((await rangeEnd()).accessibleValue);
   assert.equal(duration, 110);
   const path = join(output, 'invalid-range.mp4');
   const filesBefore = (await readdir(output)).sort();
   await client.setValue(await client.find({ label: 'File path', role: 'TextInput' }), path);
   await client.setValue(await rangeStart(), 'bad');
   await click('Export MP4'); await waitState('Export frame values must be non-negative integers');
   assert.equal((await rangeStart()).accessibleValue, 'bad');
   await client.setValue(await rangeStart(), '22');
   await client.setValue(await rangeEnd(), '22');
   await click('Export MP4'); await waitState('Export range must satisfy');
   assert.equal((await rangeEnd()).accessibleValue, '22');
   await client.setValue(await rangeEnd(), String(duration + 1));
   await click('Export MP4'); await waitState('Export range must satisfy');
   assert.equal((await rangeEnd()).accessibleValue, String(duration + 1));
   assert.deepEqual((await readdir(output)).sort(), filesBefore);
   await click('Cancel');
   await click('EXPORT');
   assert.equal((await rangeStart()).accessibleValue, '0');
   assert.equal((await rangeEnd()).accessibleValue, String(duration));
   await click('Cancel');
   await close();
 });
 await step('Ignore audio sources wholly outside the export range', async () => {
   const fixture = JSON.parse(await readFile(rangeProject, 'utf8'));
   fixture.data.assets.find(asset => asset.name === 'independent-tone.wav').path = 'missing-scheduled-audio.wav';
   const seed = join(output, 'range-offline-audio.oqvideo.json');
   await writeFile(seed, JSON.stringify(fixture));
   await launch(seed); await waitState('clips 2');
   const cutoff = join(output, 'range-before-missing-audio.mp4');
   await exportRange(cutoff, 12, 20); await waitState('Export complete');
   const samples = pcm(cutoff);
   assert(amplitude(samples, 0.03, 440) > 0.04);
   assert(amplitude(samples, 0.03, 880) < 0.006);
   const filesBefore = (await readdir(output)).sort();
   await exportRange(join(output, 'range-missing-audio.mp4'), 12, 21);
   await waitState('Export failed · Audio media offline');
   assert.deepEqual((await readdir(output)).sort(), filesBefore);
   await close();
 });
 const findInput = label => client.find({ label, role: 'TextInput' });
 const timingProject = join(output, 'timing.oqvideo.json');
 const outgoing = 'timing-blue.mp4';
 const incoming = 'linked-tone.mp4';
 const positions = async (name) => (await timeline(name)).map(clip => {
   const [, start, duration] = /Timeline start (\d+) frames; duration (\d+) frames/.exec(clip.accessibleDescription);
   return [Number(start), Number(duration)];
 }).sort(([start], [other]) => start - other);
 const sourceSetup = async (name, locked) => {
   const seed = JSON.parse(await readFile(audioProject, 'utf8'));
   const [video] = seed.data.tracks;
   seed.data.tracks.unshift({ ...video, id: 'source-target-v2', name: 'Source Video', clips: [] });
   Object.assign(seed.data.audio_tracks[1], { locked, clips: [] });
   const path = join(output, name);
   await writeFile(path, JSON.stringify(seed));
   await launch(path); await waitState('clips 1');
   await click('Reorder Source Video');
   await client.click(await assetCard('linked-tone.mp4'));
   await waitState('source PTS 0 ns');
 };
 const sourceFrame = () => client.find({ label: 'Source monitor frame', role: 'TextInput' });
 const sourceJump = async (frame, pts) => {
   await enter(await sourceFrame(), frame);
   await waitState(`source PTS ${pts} ns`);
 };
 const sourceAudioTrack = async (steps, name) => {
   const box = () => client.find({ label: 'Source audio destination', role: 'Combobox' });
   await client.click(await box());
   for (let step = 0; step < steps; step++) await client.key('\uf701');
   await client.key('\n');
   assert.equal((await box()).accessibleValue, name);
 };
 await step('Mark a source range, insert it to selected audio and video tracks, then overwrite video only', async () => {
   await sourceSetup('source-in-out.oqvideo.json', false);
   await sourceJump(5, 166666667);
   await click('Mark In');
   await click('Next source frame'); await waitState('source PTS 200000000 ns');
   await click('Previous source frame'); await waitState('source PTS 166666667 ns');
   await sourceJump(14, 466666667);
   await click('Mark Out');
   assert.equal((await sourceFrame()).accessibleValue, '14');
   await sourceAudioTrack(3, 'Audio 2');
   await click('NEXT'); await waitState('playhead 1 frames');
   await click('Insert'); await waitState('clips 2');
   const path = join(output, 'source-track-target.oqvideo.json');
   await fileAction('SAVE AS', path, 'Save project'); await waitState('Project saved');
   const inserted = JSON.parse(await readFile(path, 'utf8')).data;
   const original = inserted.assets.find(asset => asset.name === 'linked-tone.mp4');
   const [clip] = inserted.tracks.find(track => track.id === 'source-target-v2').clips;
   assert.equal(clip.timeline_start, 1);
   assert.equal(clip.duration, 10);
   assert.equal(clip.source_in_pts, original.start_pts + original.time_base.denominator / 6);
   assert.deepEqual(inserted.audio_tracks[1].clips.map(audio => audio.placement.video_id), [clip.id]);
   await client.click(await assetCard('linked-tone.mp4'));
   await waitState('source PTS 466666667 ns');
   const audioBox = await client.find({ label: 'Source audio destination', role: 'Combobox' });
   await client.click(audioBox);
   await client.key('\uf700'); await client.key('\uf700'); await client.key('\n');
   await click('Overwrite'); await waitState('audio clips 1');
   await click('SAVE'); await waitState('Project saved');
   const replaced = JSON.parse(await readFile(path, 'utf8')).data;
   assert.equal(replaced.tracks.find(track => track.id === 'source-target-v2').clips[0].duration, 10);
   assert.deepEqual(replaced.audio_tracks[1].clips, []);
   await close();
 }, { core: true });
 await step('Reject reversed source marks and an insertion into a locked audio target', async () => {
   await sourceSetup('source-locked.oqvideo.json', true);
   await sourceJump(8, 266666667); await click('Mark In');
   await sourceJump(2, 66666667); await click('Mark Out');
   await waitState('Source In must precede the exclusive Out');
   await click('Clear marks');
   await sourceAudioTrack(3, 'Audio 2');
   await click('Insert');
   await waitState('Cannot place audio on Audio 2');
   assert((await state()).includes('clips 1'));
   await close();
 });
 const syncFixture = async name => {
   const fixture = JSON.parse(await readFile(audioProject, 'utf8'));
   const videoFixture = JSON.parse(await readFile(exportProject, 'utf8'));
   const [primary] = fixture.data.tracks;
   const [original] = primary.clips;
   for (const [assetName, id, trackName] of [
     ['background.mp4', 'sync-picture', 'Sync Picture'],
     ['foreground.mp4', 'standalone-picture', 'Standalone Picture'],
   ]) {
     const asset = videoFixture.data.assets.find(item => item.name === assetName);
     fixture.data.assets.push(asset);
     fixture.data.tracks.push({ ...primary, id, name: trackName,
       clips: [{ ...original, id: id + '-clip', asset_id: asset.id, source_in_pts: asset.start_pts }],
     });
   }
   Object.assign(fixture.data.audio_tracks[1].clips[0].placement, { start: 0, duration: 48000, source: 0 });
   const path = join(output, name);
   await writeFile(path, JSON.stringify(fixture));
   return path;
 };
 const syncButton = name => client.find({ label: `Ripple sync ${name}`, role: 'Button' });
 const scrollRippleTracks = async deltaY => {
   const view = await client.find({ id: 'EditorWindow::timeline_scroll' });
   await client.value('dispatch_pointer_scroll', { windowHandle: client.windowId,
     deltaX: 0, deltaY, position: center(view) });
 };
 await step('Ripple insert and delete split the opted-in picture and independent audio together', async () => {
   const seed = await syncFixture('ripple-sync.oqvideo.json');
   await launch(seed); await waitState('clips 3');
   await click('FIT TIMELINE');
   assert.equal((await syncButton('Sync Picture')).accessibleDescription, 'Ripple sync disabled');
   await client.click(await syncButton('Sync Picture'));
   assert.equal((await syncButton('Sync Picture')).accessibleDescription, 'Ripple sync enabled');
   await scrollRippleTracks(-380);
   await client.click(await syncButton('Audio 2'));
   assert.equal((await syncButton('Audio 2')).accessibleDescription, 'Ripple sync enabled');
   await scrollRippleTracks(380);
   for (let i = 0; i < 10; i++) await click('NEXT');
   await waitState('playhead 10 frames');
   await client.click(await assetCard('linked-tone.mp4'));
   await waitState('source PTS 0 ns');
   await sourceJump(2, 66666667); await click('Mark Out');
   await sourceAudioTrack(1, 'Video only');
   await click('Insert'); await waitState('clips 6');
   assert.deepEqual(await positions('linked-tone.mp4'), [[0, 10], [10, 3], [13, 80]]);
   assert.deepEqual(await positions('background.mp4'), [[0, 10], [13, 80]]);
   await click('SAVE'); await waitState('Project saved');
   const inserted = JSON.parse(await readFile(seed, 'utf8')).data;
   const [untouched] = inserted.tracks.find(track => track.id === 'standalone-picture').clips;
   assert.deepEqual([untouched.timeline_start, untouched.duration], [0, 90]);
   const audioRegions = async () => {
     const data = JSON.parse(await readFile(seed, 'utf8')).data;
     return data.audio_tracks[1].clips.map(clip => clip.placement);
   };
   assert.deepEqual(await audioRegions(), [
     { kind: 'independent', start: 0, duration: 16000, source: 0 },
     { kind: 'independent', start: 20800, duration: 32000, source: 16000 },
   ]);
   await click('Ripple delete'); await waitState('clips 5');
   assert.deepEqual(await positions('background.mp4'), [[0, 10], [10, 80]]);
   await click('SAVE'); await waitState('Project saved');
   assert.equal((await audioRegions())[1].start, 16000);
   await click('UNDO'); await waitState('clips 6');
   assert.deepEqual(await positions('background.mp4'), [[0, 10], [13, 80]]);
   await click('SAVE'); await waitState('Project saved');
   await close();
   await launch(seed); await waitState('clips 6');
   assert.equal((await syncButton('Sync Picture')).accessibleDescription, 'Ripple sync enabled');
   await scrollRippleTracks(-380);
   assert.equal((await syncButton('Audio 2')).accessibleDescription, 'Ripple sync enabled');
   await close();
 }, { core: true });
 await step('A locked participating track rejects ripple deletion as one edit', async () => {
   const path = await syncFixture('ripple-sync-locked.oqvideo.json');
   await launch(path); await waitState('clips 3');
   await click('FIT TIMELINE');
   await client.click(await syncButton('Sync Picture'));
   await click('Lock Sync Picture');
   await client.click(await item('linked-tone.mp4'));
   await click('Ripple delete'); await waitState('Sync Picture is locked');
   assert.deepEqual(await positions('linked-tone.mp4'), [[0, 90]]);
   assert.deepEqual(await positions('background.mp4'), [[0, 90]]);
   await click('SAVE'); await waitState('Project saved');
   await close();
 });
 await step('Insert, ripple delete, overwrite and roll the cut on the native timeline', async () => {
   const short = generate(outgoing, 'blue', 0.5);
   await launch(audioProject);
   await waitState('audio clips 2');
   await fileAction('IMPORT', short, 'Import media');
   await waitState('assets 3');
   await click('NEXT'); await waitState('playhead 1 frames');
   await click('Insert');
   await waitState('assets 3; clips 3');
   await click('FIT TIMELINE');
   assert.deepEqual(await positions(incoming), [[0, 1], [16, 89]]);
   assert.deepEqual(await positions(outgoing), [[1, 15]]);
   await waitState('audio clips 3');
   await expectColor('blue');

   await click('Ripple delete'); await waitState('assets 3; clips 2');
   assert.deepEqual(await positions(incoming), [[0, 1], [1, 89]]);
   await expectColor('red');
   await click('UNDO'); await waitState('assets 3; clips 3');
   await click('UNDO'); await waitState('assets 3; clips 1');
   assert.deepEqual(await positions(incoming), [[0, 90]]);

   await click('Overwrite'); await waitState('assets 3; clips 3');
   assert.deepEqual(await positions(incoming), [[0, 1], [16, 74]]);
   assert.deepEqual(await positions(outgoing), [[1, 15]]);
   await enter(await findInput('Roll cut (frame)'), 15);
   assert.deepEqual(await positions(outgoing), [[1, 14]]);
   assert.deepEqual(await positions(incoming), [[0, 1], [15, 75]]);

   await fileAction('SAVE AS', timingProject, 'Save project'); await waitState('Project saved');
   const { data } = JSON.parse(await readFile(timingProject, 'utf8'));
   const asset = data.assets.find(asset => asset.name === incoming);
   const tail = data.tracks[0].clips.at(-1);
   assert.equal(tail.source_in_pts, asset.start_pts + asset.time_base.denominator / asset.time_base.numerator / 2);
   assert.deepEqual(data.audio_tracks[0].clips.map(clip => clip.placement.video_id).sort(),
     [data.tracks[0].clips[0].id, tail.id].sort());
   assert.deepEqual(data.audio_tracks[1].clips[0].placement,
     { kind: 'independent', start: 0, duration: 144000, source: 0 });
   await close();
 }, { core: true });
 await step('Roll rejects locked linked audio and insufficient source handles atomically', async () => {
   await launch(timingProject); await waitState('assets 3; clips 3');
   await click('FIT TIMELINE');
   await click('Lock Audio 1');
   await client.click(await item(outgoing));
   const roll = () => client.waitFor(async () => (await client.byLabel('Roll cut (frame)', { role: 'TextInput' }))[0],
     { description: 'selected outgoing clip exposes its roll cut' });
   await enter(await roll(), 16);
   await waitState('Audio 1 is locked');
   assert.deepEqual(await positions(outgoing), [[1, 14]]);
   assert.deepEqual(await positions(incoming), [[0, 1], [15, 75]]);
   await click('Unlock Audio 1');
   await enter(await roll(), 17);
   await waitState('Clip source range extends outside media presentation duration');
   assert.deepEqual(await positions(outgoing), [[1, 14]]);
   assert.deepEqual(await positions(incoming), [[0, 1], [15, 75]]);
   await click('SAVE'); await waitState('Project saved');
   await close();
 });
 await step('Audio gain, mute and linked trim persist and affect the rendered mix', async () => {
   await launch(audioProject); await waitState('clips 1');
   await click('Lock Audio 1');
   await client.click(await item('linked-tone.mp4'));
   await enter(await findInput('Timeline end'), 45);
   await waitState('Audio 1 is locked');
   assert((await item('linked-tone.mp4')).accessibleDescription.includes('duration 90 frames'));
   await click('Unlock Audio 1');
   await enter(await findInput('Timeline end'), 60);
   assert((await audioClip('linked-tone.mp4')).accessibleDescription.startsWith('0 samples to 96000'));
   await click('UNDO');
   assert((await audioClip('linked-tone.mp4')).accessibleDescription.startsWith('0 samples to 144000'));
   await click('NEXT'); await waitState('playhead 1 frames');
   await click('SPLIT'); await waitState('audio clips 3');
   const linkedIntervals = (await client.byLabel('Audio clip linked-tone.mp4', { role: 'Button' }))
     .map(item => item.accessibleDescription).sort();
   assert.deepEqual(linkedIntervals, ['0 samples to 1600 · linked video', '1600 samples to 144000 · linked video']);
   await click('UNDO'); await waitState('audio clips 2');
   await enter(await findInput('Audio 1 gain (%)'), 25);
   await fileAction('EXPORT', join(output, 'audio-gain.mp4'), 'Export MP4'); await waitState('Export complete');
   const quieter = pcm(join(output, 'audio-gain.mp4'));
   assert(amplitude(quieter, 0.45, 440) < amplitude(pcm(audioOutput), 0.45, 440) * 0.35);
   assert(amplitude(quieter, 0.45, 880) > 0.04);
   await click('Mute Audio 1');
   await client.find({ label: 'Unmute Audio 1', role: 'Button' });
   await fileAction('EXPORT', join(output, 'audio-muted.mp4'), 'Export MP4'); await waitState('audio-muted.mp4');
   const muted = pcm(join(output, 'audio-muted.mp4'));
   assert(amplitude(muted, 0.45, 440) < 0.006);
   assert(amplitude(muted, 0.45, 880) > 0.04);
   await click('Unmute Audio 1');
   await click('SAVE'); await waitState('Project saved');
   await close();
   await launch(audioProject); await waitState('clips 1');
   assert.equal((await findInput('Audio 1 gain (%)')).accessibleValue, '25');
   assert((await audioClip('linked-tone.mp4')).accessibleDescription.endsWith('linked video'));
   await click('PLAY');
   await waitAudio();
   await client.waitFor(async () => {
     const status = await state();
     return status.includes('playhead 89 frames') && !status.includes('Audio output');
   }, { description: 'the audio device drains and playback stops on the final frame' });
   await close();
 });
 await step('Unlink and edit source, position, trim and gain at sample precision', async () => {
   await launch(audioProject); await waitState('clips 1');
   await client.click(await audioClip('linked-tone.mp4'));
   assert.equal(Boolean((await findInput('Audio start')).accessibleEnabled), false);
   await click('Unlink audio');
   const length = 144000;
   await enter(await findInput('Audio start'), 123);
   await enter(await findInput('Audio end'), 48123);
   await enter(await findInput('Audio source in'), 24000);
   await enter(await findInput('Audio clip gain (%)'), 50);
   assert((await audioClip('linked-tone.mp4')).accessibleDescription.startsWith('123 samples to 48123'));
   await click('Delete audio clip');
   assert.equal((await client.byLabel('Audio clip linked-tone.mp4', { role: 'Button', visible: false })).length, 0);
   await click('UNDO');
   assert((await audioClip('linked-tone.mp4')).accessibleDescription.startsWith('123 samples to 48123'));
   await fileAction('SAVE AS', join(output, 'unlinked.oqvideo.json'), 'Save project'); await waitState('Project saved');
   const data = JSON.parse(await readFile(join(output, 'unlinked.oqvideo.json'), 'utf8')).data;
   const clip = data.audio_tracks[0].clips[0];
   assert.deepEqual(clip.placement, { kind: 'independent', start: 123, duration: 48000, source: 24000 });
   assert.equal(clip.gain, 0.5);
   assert.equal(data.tracks[0].clips[0].duration * 1600, length);
   await close();
 });
 await step('Solo a track, observe separated meters, and export only that track', async () => {
   await launch(audioProject); await waitState('audio clips 2');
   await audioLaneScroll();
   const solo = async () => (await buttons('Solo Audio 2'))[0];
   assert.equal((await solo()).accessibleDescription, 'Solo disabled');
   await client.click(await solo());
   assert.equal((await solo()).accessibleDescription, 'Solo enabled');
   await click('PLAY'); await waitAudio();
   const meter = name => client.find({ label: `${name} level`, visible: false });
   await client.waitFor(async () => Number((await meter('Audio 2')).accessibleDescription.replace('%', '')) > 0,
     { description: 'soloed lane shows an active level' });
   assert.equal((await meter('Audio 1')).accessibleDescription, '0%');
   await click('PAUSE');
   const path = join(output, 'audio-solo.mp4');
   await fileAction('EXPORT', path, 'Export MP4'); await waitState('Export complete');
   const samples = pcm(path);
   assert(amplitude(samples, 0.45, 440) < 0.006);
   assert(amplitude(samples, 0.45, 880) > 0.04);
   await click('UNDO');
   assert.equal((await solo()).accessibleDescription, 'Solo disabled');
   await close();
 }, { core: true });
 await step('Report clipping from the mixed signal before it is clamped', async () => {
   const tone = join(output, 'clipping.wav');
   mediaCommand(['-f', 'lavfi', '-i', 'aevalsrc=0.56*sin(2*PI*440*t)|0.56*sin(2*PI*440*t):s=48000:d=1.2:c=stereo',
     '-c:a', 'pcm_s16le', '-y', tone]);
   await launch();
   await fileAction('IMPORT', tone, 'Import media'); await waitState('assets 1');
   await click('Add clipping.wav to timeline'); await waitState('audio clips 1');
   const gain = await client.find({ label: 'Audio 1 gain (%)', role: 'TextInput' });
   await enter(gain, '200');
   await click('PLAY'); await waitState('Audio output 100% · clipping');
   await click('PAUSE');
   await fileAction('SAVE AS', join(output, 'clipping.oqvideo.json'), 'Save project'); await waitState('Project saved');
   await close();
 });
 await step('Crossfade adjacent audio clips and hear both source tones in the export', async () => {
   const fixture = JSON.parse(await readFile(audioProject, 'utf8'));
   const [linkedTrack, independentTrack] = fixture.data.audio_tracks;
   const [left] = independentTrack.clips;
   const [right] = linkedTrack.clips;
   left.placement = { kind: 'independent', start: 0, duration: 48000, source: 0 };
   right.placement = { kind: 'independent', start: 48000, duration: 48000, source: 48000 };
   independentTrack.clips = [left, right];
   fixture.data.audio_tracks = [independentTrack];
   const seed = join(output, 'audio-crossfade.oqvideo.json');
   await writeFile(seed, JSON.stringify(fixture));
   await launch(seed); await waitState('audio clips 2');
   await click('FIT TIMELINE'); await audioLaneScroll();
   await client.click(await audioClip('independent-tone.wav'));
   const setCrossfade = async value => enter(await clipControl('Audio crossfade (samples)', 'TextInput', 'audio_properties'), value);
   await setCrossfade(96000);
   await waitState('Crossfade needs more source audio before the incoming clip');
   await setCrossfade(9600);
   await click('SAVE'); await waitState('Project saved');
   const stored = JSON.parse(await readFile(seed, 'utf8')).data.audio_tracks[0];
   assert.deepEqual(stored.crossfades, [{ outgoing: left.id, incoming: right.id, samples: 9600 }]);
   const path = join(output, 'audio-crossfade.mp4');
   await fileAction('EXPORT', path, 'Export MP4'); await waitState('Export complete');
   const samples = pcm(path);
   assert(amplitude(samples, 0.75, 880) > 0.04);
   assert(amplitude(samples, 0.75, 440) < 0.006);
   assert(amplitude(samples, 0.95, 880) > 0.035);
   assert(amplitude(samples, 0.95, 440) > 0.035);
   assert(amplitude(samples, 1.2, 440) > 0.04);
   assert(amplitude(samples, 1.2, 880) < 0.006);
   await setCrossfade(0);
   await click('SAVE'); await waitState('Project saved');
   assert.equal(JSON.parse(await readFile(seed, 'utf8')).data.audio_tracks[0].crossfades, undefined);
   await close();
 }, { core: true });
 const keyProject = join(output, 'opacity.oqvideo.json');
 await step('Animate opacity with the same interpolation in scrubbing, playback and MP4', async () => {
   const fixture = JSON.parse(await readFile(multiProject, 'utf8'));
   Object.assign(fixture.data, { width: 320, height: 180 });
   for (const clip of fixture.data.tracks.flatMap(track => track.clips)) {
     const asset = fixture.data.assets.find(asset => asset.id === clip.asset_id);
     Object.assign(clip, { timeline_start: 0, duration: 3, source_in_pts: asset.start_pts,
       source_subtick_numerator: 0, source_subtick_denominator: 1 });
     Object.assign(clip.transform, { x: 0, y: 0, scale: 1, rotation: 0, opacity: 1, exposure: 0 });
   }
   const seed = join(output, 'opacity-seed.oqvideo.json');
   await writeFile(seed, JSON.stringify(fixture));
   await launch(seed); await waitState('clips 2');
   await click('FIT TIMELINE');
   await client.click(await item('foreground.mp4'));
   await enter(await clipControl('Opacity (%)'), 0);
   await expectColor('blue');
   await client.click(await clipControl('Add opacity keyframe', 'Button'));
   await clipControl('Remove opacity keyframe', 'Button');
   await click('NEXT'); await click('NEXT'); await waitState('playhead 2 frames');
   await enter(await clipControl('Opacity (%)'), 100);
   await expectColor('red');
   await click('PREVIOUS'); await waitState('playhead 1 frames');
   await expectColor('blend');
   assert.equal(Number((await clipControl('Opacity (%)')).accessibleValue), 50);
   await click('PREVIOUS'); await waitState('playhead 0 frames');
   await click('PLAY'); await waitState('playhead 2 frames');
   await expectColor('red');
   await fileAction('SAVE AS', keyProject, 'Save project'); await waitState('Project saved');
   const { data } = JSON.parse(await readFile(keyProject, 'utf8'));
   assert.deepEqual(data.tracks[0].clips[0].opacity_keys, [
     { time: 0, value: 0 }, { time: 66666666, value: 1 },
   ]);
   const exportPath = join(output, 'opacity.mp4');
   await fileAction('EXPORT', exportPath, 'Export MP4'); await waitState('Export complete');
   const pixels = mediaCommand(['-i', exportPath, '-map', '0:v:0', '-vsync', '0', '-pix_fmt', 'rgb24', '-f', 'rawvideo', '-']);
   const stride = 320 * 180 * 3;
   assert.equal(pixels.length, stride * 3);
   for (const [frame, color] of [[0, [0, 0, 255]], [1, [188, 0, 188]], [2, [255, 0, 0]]]) {
     const offset = frame * stride + (90 * 320 + 160) * 3;
     assert(color.every((expected, channel) => Math.abs(pixels[offset + channel] - expected) < 22));
   }
   await close();
 }, { core: true });
 await step('Remove an opacity keyframe, undo and preserve the curve across a split', async () => {
   await launch(keyProject); await waitState('clips 2');
   await click('FIT TIMELINE');
   await client.click(await item('foreground.mp4'));
   await expectColor('blue');
   await client.click(await clipControl('Remove opacity keyframe', 'Button'));
   await expectColor('red');
   await click('UNDO'); await expectColor('blue');
   await click('NEXT'); await waitState('playhead 1 frames');
   await click('SPLIT'); await waitState('clips 3');
   await expectColor('blend');
   await click('SAVE'); await waitState('Project saved');
   await close();
 });
 const speedName = 'retime-source.mp4';
 const speedProject = join(output, 'retime.oqvideo.json');
 const goToSpeed = async value => enter(await clipControl('Speed (%)'), value);
 const expectRateDuration = async duration => client.waitFor(async () =>
   (await positions(speedName)).some(([start, length]) => start === 0 && length === duration),
 { description: `first retimed clip has ${duration} frames` });
 await step('Change clip speed and keep retimed video and linked audio synchronized in MP4', async () => {
   const source = join(output, speedName);
   mediaCommand(['-f', 'lavfi', '-i', 'color=c=red:s=320x180:r=30:d=1',
     '-f', 'lavfi', '-i', 'color=c=blue:s=320x180:r=30:d=1',
     '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000:duration=2',
     '-filter_complex', '[0:v][1:v]concat=n=2:v=1:a=0,scale=in_color_matrix=bt601:out_color_matrix=bt709[v]',
     '-map', '[v]', '-map', '2:a:0', '-c:v', 'libx264', '-pix_fmt', 'yuv420p',
     '-color_primaries', 'bt709', '-color_trc', 'bt709', '-colorspace', 'bt709', '-c:a', 'aac', '-y', source]);
   await launch(); await waitState('assets 0; clips 0');
   await click('SEQUENCE');
   await client.setValue(await findInput('Sequence width'), '320');
   await client.setValue(await findInput('Sequence height'), '180');
   await click('Apply settings');
   await fileAction('IMPORT', source, 'Import media'); await waitState('assets 1');
   await click('ADD CLIP'); await waitState('clips 1');
   await goToSpeed(200); await expectRateDuration(30);
   assert((await audioClip(speedName)).accessibleDescription.startsWith('0 samples to 48000'));
   await click('UNDO'); await expectRateDuration(60);
   await click('REDO'); await expectRateDuration(30);
   await expectColor('red');
   await click('PLAY'); await waitAudio();
   await waitState('playhead 29 frames');
   await expectColor('blue');
   await client.waitFor(async () => !(await state()).includes('Audio output'), { description: 'retimed audio reaches the end' });
   await fileAction('SAVE AS', speedProject, 'Save project'); await waitState('Project saved');
   const { data } = JSON.parse(await readFile(speedProject, 'utf8'));
   assert.equal(data.tracks[0].clips[0].speed_percent, 200);
   const exported = join(output, 'retimed-fast.mp4');
   await fileAction('EXPORT', exported, 'Export MP4'); await waitState('Export complete');
   const audio = pcm(exported);
   assert(audio.length / 2 >= 48000 && audio.length / 2 < 49024);
   assert(amplitude(audio, 0.1, 880) > 0.04);
   assert(amplitude(audio, 0.1, 440) < 0.008);
   const video = mediaCommand(['-i', exported, '-map', '0:v:0', '-vsync', '0', '-pix_fmt', 'rgb24', '-f', 'rawvideo', '-'],
     { maxBuffer: 8 * 1024 * 1024 });
   const stride = 320 * 180 * 3;
   assert.equal(video.length, stride * 30);
   for (let frame = 0; frame < 30; frame++) {
     const offset = frame * stride + (90 * 320 + 160) * 3;
     assert(video[offset + (frame < 15 ? 0 : 2)] > 220, `wrong source frame at ${frame}`);
     assert(video[offset + (frame < 15 ? 2 : 0)] < 30);
   }
   await close();
 }, { core: true });
 await step('Retime rejects overlap and a locked audio link, and unlink preserves slowed audio', async () => {
   await launch(speedProject); await waitState('clips 1; playhead 0 frames');
   await click('ADD CLIP'); await waitState('assets 1; clips 2');
   await click('FIT TIMELINE');
   await client.click((await timeline(speedName))[0]);
   await goToSpeed(50);
   await waitState('Video track clips cannot overlap');
   assert.deepEqual(await positions(speedName), [[0, 30], [30, 60]]);
   await click('Lock Audio 1');
   await goToSpeed(400);
   await waitState('Audio 1 is locked');
   assert.deepEqual(await positions(speedName), [[0, 30], [30, 60]]);
   await click('Unlock Audio 1');
   await client.click((await timeline(speedName))[1]);
   await waitState('playhead 30 frames');
   await click('DELETE'); await waitState('assets 1; clips 1');
   await client.click(await item(speedName));
   await goToSpeed(400); await expectRateDuration(15);
   await click('PLAY'); await waitState('playhead 14 frames');
   await expectColor('blue');
   await client.waitFor(async () => !(await state()).includes('Audio output'), { description: '4x playback drains' });
   await goToSpeed(50); await expectRateDuration(120);
   assert((await audioClip(speedName)).accessibleDescription.startsWith('0 samples to 192000'));
   await client.click(await audioClip(speedName));
   await client.waitFor(async () => (await client.byLabel('Unlink audio', { role: 'Button' }))[0],
     { description: 'selected audio clip exposes Unlink' });
   await click('Unlink audio');
   await client.click(await item(speedName));
   await client.waitFor(async () => (await client.query({ id: 'EditorWindow::clip_properties' }))[0],
     { description: 'video inspector is selected again after unlinking' });
   await goToSpeed(100); await expectRateDuration(60);
   assert.equal((await audioClip(speedName)).accessibleDescription, '0 samples to 192000 · 50% speed');
   const exported = join(output, 'retimed-unlinked.mp4');
   await fileAction('EXPORT', exported, 'Export MP4'); await waitState('Export complete');
   const audio = pcm(exported);
   assert(audio.length / 2 >= 192000 && audio.length / 2 < 193024);
   assert(amplitude(audio, 2.5, 220) > 0.04);
   assert(amplitude(audio, 2.5, 440) < 0.008);
   await fileAction('SAVE AS', join(output, 'retimed-unlinked.oqvideo.json'), 'Save project'); await waitState('Project saved');
   await close();
 });
 await step('Opacity keyframes continue to follow the source after changing clip speed', async () => {
   const seed = join(output, 'retimed-opacity.oqvideo.json');
   await writeFile(seed, await readFile(keyProject));
   await launch(seed);
   await waitState('clips 3');
   await click('FIT TIMELINE');
   const tail = (await timeline('foreground.mp4')).find(node => node.accessibleDescription.includes('start 1 frames'));
   assert(tail);
   await client.click(tail);
   await goToSpeed(50);
   await client.waitFor(async () => (await positions('foreground.mp4')).some(([start, length]) => start === 1 && length === 4),
     { description: 'second animated clip now spans four sequence frames' });
   await expectColor('blend');
   await click('NEXT'); await waitState('playhead 2 frames');
   await expectColor('three-quarter');
   await click('NEXT'); await waitState('playhead 3 frames');
   await expectColor('red');
   await click('SAVE'); await waitState('Project saved');
   await close();
 });
 const dissolveProject = join(output, 'dissolve.oqvideo.json');
 const dissolveField = async frames => enter(await clipControl('Dissolve (frames)'), frames);
 const dissolveMarkers = () => client.byLabel('Dissolve', { role: 'Groupbox' });
 const expectDissolve = async (first, last) => client.waitFor(async () =>
   (await dissolveMarkers()).some(marker => marker.accessibleDescription.endsWith(`frames ${first}–${last}`)),
 { description: `dissolve marker covers frames ${first}–${last}` });
 await step('Dissolve real adjacent media in scrub, playback and independently decoded MP4', async () => {
   const fixture = JSON.parse(await readFile(multiProject, 'utf8'));
   const outgoing = fixture.data.tracks[0].clips[0];
   const incoming = fixture.data.tracks.at(-1).clips[0];
   const outgoingAsset = fixture.data.assets.find(asset => asset.id === outgoing.asset_id);
   const incomingAsset = fixture.data.assets.find(asset => asset.id === incoming.asset_id);
   const second = incomingAsset.time_base.denominator / incomingAsset.time_base.numerator;
   Object.assign(outgoing, { timeline_start: 0, duration: 6, source_in_pts: outgoingAsset.start_pts });
   Object.assign(incoming, { timeline_start: 6, duration: 6, source_in_pts: incomingAsset.start_pts + second / 5 });
   for (const clip of [outgoing, incoming]) {
     Object.assign(clip, { source_subtick_numerator: 0, source_subtick_denominator: 1 });
     Object.assign(clip.transform, { x: 0, y: 0, scale: 1, rotation: 0, opacity: 1, exposure: 0 });
   }
   Object.assign(fixture.data, {
     width: 320, height: 180,
     tracks: [{ ...fixture.data.tracks[0], name: 'Dissolve track', clips: [outgoing, incoming] }],
   });
   const seed = join(output, 'dissolve-seed.oqvideo.json');
   await writeFile(seed, JSON.stringify(fixture));
   await launch(seed); await waitState('clips 2');
   await click('FIT TIMELINE');
   await client.click(await item('foreground.mp4'));
   await dissolveField(4);
   await expectDissolve(4, 7);
   assert.deepEqual(await positions('foreground.mp4'), [[0, 6]]);
   assert.deepEqual(await positions('background.mp4'), [[6, 6]]);
   for (let i = 0; i < 5; i++) await click('NEXT');
   await waitState('playhead 5 frames');
   await expectColor('three-quarter');
   await click('NEXT'); await waitState('playhead 6 frames');
   await expectColor('blend');
   await snapshot('18-dissolve-midpoint');
   for (let i = 0; i < 6; i++) await click('PREVIOUS');
   await waitState('playhead 0 frames');
   await click('PLAY'); await waitState('playhead 11 frames');
   await expectColor('blue');
   await fileAction('SAVE AS', dissolveProject, 'Save project'); await waitState('Project saved');
   const saved = JSON.parse(await readFile(dissolveProject, 'utf8')).data;
   assert.deepEqual(saved.tracks[0].dissolves, [{ outgoing: outgoing.id, incoming: incoming.id, frames: 4 }]);
   const exported = join(output, 'dissolve.mp4');
   await fileAction('EXPORT', exported, 'Export MP4'); await waitState('Export complete');
   const pixels = mediaCommand(['-i', exported, '-map', '0:v:0', '-vsync', '0', '-pix_fmt', 'rgb24', '-f', 'rawvideo', '-'],
     { maxBuffer: 4 * 1024 * 1024 });
   const stride = 320 * 180 * 3;
   assert.equal(pixels.length, stride * 12);
   const colors = [[255, 0, 0], [225, 0, 137], [188, 0, 188], [137, 0, 225], [0, 0, 255]];
   for (let frame = 0; frame < 12; frame++) {
     const color = colors[Math.min(4, Math.max(0, frame - 4))];
     const offset = frame * stride + (90 * 320 + 160) * 3;
     assert(color.every((expected, channel) => Math.abs(pixels[offset + channel] - expected) < 22),
       `dissolve frame ${frame} has RGB ${[...pixels.subarray(offset, offset + 3)]}`);
   }
   await close();
 }, { core: true });
 await step('Resize and remove a saved dissolve and reject missing incoming footage', async () => {
   await launch(dissolveProject); await waitState('clips 2');
   await click('FIT TIMELINE');
   await expectDissolve(4, 7);
   await client.click(await item('foreground.mp4'));
   await dissolveField(2); await expectDissolve(5, 6);
   await click('UNDO'); await expectDissolve(4, 7);
   await dissolveField(0);
   await client.waitFor(async () => (await dissolveMarkers()).length === 0, { description: 'remove the dissolve marker' });
   await client.click(await item('background.mp4'));
   const { data } = JSON.parse(await readFile(dissolveProject, 'utf8'));
   const asset = data.assets.find(asset => asset.name === 'background.mp4');
   await enter(await clipControl('Source in'), asset.start_pts);
   await client.click(await item('foreground.mp4'));
   await dissolveField(4); await waitState('Not enough source before the incoming clip');
   assert.equal((await dissolveMarkers()).length, 0);
   assert.deepEqual(await positions('foreground.mp4'), [[0, 6]]);
   assert.deepEqual(await positions('background.mp4'), [[6, 6]]);
   await click('SAVE'); await waitState('Project saved');
   await close();
 });
 await step('Only require visual inputs used by the range, including the incoming side of a dissolve', async () => {
   const fixture = JSON.parse(await readFile(join(output, 'dissolve-seed.oqvideo.json'), 'utf8'));
   const [track] = fixture.data.tracks;
   const [left, right] = track.clips;
   track.dissolves = [{ outgoing: left.id, incoming: right.id, frames: 4 }];
   const transition = join(output, 'range-dissolve-seed.oqvideo.json');
   await writeFile(transition, JSON.stringify(fixture));
   await launch(transition); await waitState('clips 2');
   const clipped = join(output, 'range-dissolve.mp4');
   await exportRange(clipped, 5, 7); await waitState('Export complete');
   const frames = path => mediaCommand(['-i', path, '-map', '0:v:0', '-vsync', '0', '-pix_fmt', 'rgb24', '-f', 'rawvideo', '-']);
   const stride = 320 * 180 * 3;
   const blended = frames(clipped);
   assert.equal(blended.length, 2 * stride);
   for (const [i, expected] of [[0, [225, 0, 137]], [1, [188, 0, 188]]]) {
     const index = i * stride + (90 * 320 + 160) * 3;
     const pixel = blended.subarray(index, index + 3);
     assert(expected.every((channel, j) => Math.abs(pixel[j] - channel) < 22),
       `frame ${5 + i} retains its dissolve phase, got ${[...pixel]}`);
   }
   await close();
   fixture.data.assets.find(asset => asset.id === right.asset_id).path = 'missing-dissolve-incoming.mp4';
   const seed = join(output, 'range-offline-dissolve.oqvideo.json');
   await writeFile(seed, JSON.stringify(fixture));
   await launch(seed); await waitState('clips 2');
   const cutoff = join(output, 'range-before-dissolve.mp4');
   await exportRange(cutoff, 1, 4); await waitState('Export complete');
   assert.equal(frames(cutoff).length, 3 * stride);
   const filesBefore = (await readdir(output)).sort();
   await exportRange(join(output, 'range-in-dissolve.mp4'), 4, 5);
   await waitState('Export failed · Media offline');
   assert.deepEqual((await readdir(output)).sort(), filesBefore);
   await close();
 });
 const titleProject = join(output, 'title.oqvideo.json');
 const titleText = 'Quartz 标题';
 const titleField = async (label, value) => enter(await clipControl(label), value);
 const whitePixels = (pixels, width, channels, left, top, right, bottom, threshold = 225) => {
   let white = 0;
   for (let y = top; y < bottom; y++) {
     for (let x = left; x < right; x++) {
       const offset = (y * width + x) * channels;
       if (pixels[offset] > threshold && pixels[offset + 1] > threshold && pixels[offset + 2] > threshold) white++;
     }
   }
   return white;
 };
 const previewHasTitle = async () => {
   const preview = await client.find({ label: 'Video preview', role: 'Image' });
   const window = await client.windowProperties();
   const png = decodePng(await client.screenshot());
   const scaleX = png.width * (window.scaleFactor || 1) / window.size.width;
   const scaleY = png.height * (window.scaleFactor || 1) / window.size.height;
   const { x, y } = center(preview);
   return whitePixels(png.pixels, png.width, png.channels,
     Math.floor((x - preview.size.width * 0.35) * scaleX),
     Math.floor((y - preview.size.height * 0.18) * scaleY),
     Math.ceil((x + preview.size.width * 0.35) * scaleX),
     Math.ceil((y + preview.size.height * 0.18) * scaleY)) > 200;
 };
 await step('Add and edit a title over video, reopen it and verify exact MP4 frames', async () => {
   const fixture = JSON.parse(await readFile(multiProject, 'utf8'));
   const track = fixture.data.tracks.find(track => track.clips.some(clip =>
     fixture.data.assets.find(asset => asset.id === clip.asset_id)?.name === 'background.mp4'));
   const [base] = track.clips;
   const media = fixture.data.assets.find(asset => asset.id === base.asset_id);
   Object.assign(base, { timeline_start: 0, duration: 6, source_in_pts: media.start_pts,
     source_subtick_numerator: 0, source_subtick_denominator: 1, opacity_keys: [] });
   Object.assign(base.transform, { x: 0, y: 0, scale: 1, rotation: 0, opacity: 1, exposure: 0 });
   Object.assign(fixture.data, { width: 320, height: 180, tracks: [{ ...track, visible: true, clips: [base] }] });
   const seed = join(output, 'title-seed.oqvideo.json');
   await writeFile(seed, JSON.stringify(fixture));
   await launch(seed); await waitState('clips 1');
   await click('NEXT'); await click('NEXT'); await waitState('playhead 2 frames');
   await expectColor('blue');
   await click('Add title'); await waitState('clips 2');
   assert.equal((await positions('Title'))[0][0], 2);
   await titleField('Title text', titleText);
   assert.equal((await positions(titleText))[0][0], 2);
   await titleField('Title size (%)', 22);
   await titleField('Timeline end', 4);
   assert.deepEqual(await positions(titleText), [[2, 2]]);
   await client.waitFor(previewHasTitle, { description: 'white title glyphs over the program preview' });
   await snapshot('19-title-overlay');
   await fileAction('SAVE AS', titleProject, 'Save project'); await waitState('Project saved');
   const saved = JSON.parse(await readFile(titleProject, 'utf8')).data;
   const [titleTrack] = saved.tracks;
   assert.equal(titleTrack.name, 'Titles');
   assert.deepEqual(titleTrack.clips[0].title, { text: titleText, size_percent: 22 });
   assert.equal(titleTrack.clips[0].asset_id, undefined);
   await close();
   await launch(titleProject); await waitState('clips 2');
   await client.click(await item(titleText));
   assert.equal((await clipControl('Title text')).accessibleValue, titleText);
   assert.equal((await clipControl('Title size (%)')).accessibleValue, '22');
   await client.waitFor(previewHasTitle, { description: 'saved title is visible after reopening' });
   await click('NEXT'); await click('NEXT'); await waitState('playhead 4 frames');
   await expectColor('blue');
   const exported = join(output, 'title.mp4');
   await fileAction('EXPORT', exported, 'Export MP4'); await waitState('Export complete');
   const pixels = mediaCommand(['-i', exported, '-map', '0:v:0', '-vsync', '0', '-pix_fmt', 'rgb24', '-f', 'rawvideo', '-'],
     { maxBuffer: 4 * 1024 * 1024 });
   const stride = 320 * 180 * 3;
   assert.equal(pixels.length, stride * 6);
   for (let frame = 0; frame < 6; frame++) {
     const image = pixels.subarray(frame * stride, (frame + 1) * stride);
     const white = whitePixels(image, 320, 3, 0, 0, 320, 180);
     assert(frame >= 2 && frame < 4 ? white > 350 : white === 0,
       `frame ${frame} has ${white} white title pixels`);
     const [r, g, b] = image.subarray((20 * 320 + 20) * 3, (20 * 320 + 20) * 3 + 3);
     assert(r < 25 && g < 25 && b > 220, `frame ${frame} retains blue below the title`);
   }
   await close();
 }, { core: true });
 await step('Titles work without media and title validation preserves undoable edits', async () => {
   await launch(); await waitState('assets 0; clips 0');
   await click('Add title'); await waitState('assets 0; clips 1');
   await client.waitFor(previewHasTitle, { description: 'title alone renders on the program canvas' });
   assert.deepEqual(await positions('Title'), [[0, 90]]);
   await titleField('Title size (%)', 31);
   await waitState('Title size must be a percentage from 1 to 30');
   await titleField('Title text', '   ');
   await waitState('Title text must contain 1 to 256 characters');
   assert.deepEqual(await positions('Title'), [[0, 90]]);
   await titleField('Scale', 0.75);
   const transform = await client.find({ label: 'TRANSFORM', role: 'Button' });
   await client.click(transform);
   const resizeGrip = async () => {
     try {
       return (await client.byLabel('Resize bottom right', { role: 'Button' }))[0];
     } catch (error) {
       if (!/destroyed|stale/i.test(error.message)) throw error;
     }
   };
   const handle = await client.waitFor(resizeGrip, { description: 'resize grip on the selected title' });
   const before = center(handle);
   await client.drag(handle, { x: before.x - 60, y: before.y - 35 });
   await client.waitFor(async () => {
     const grip = await resizeGrip();
     return grip && center(grip).x < before.x - 25;
   }, { description: 'resize grip scales the generated title layer' });
   await client.click(transform);
   await click('UNDO'); await click('UNDO');
   assert.equal((await clipControl('Scale')).accessibleValue, '1.000');
   await click('NEXT'); await click('SPLIT'); await waitState('clips 2');
   assert.deepEqual(await positions('Title'), [[0, 1], [1, 89]]);
   await click('UNDO'); await waitState('clips 1');
   assert.deepEqual(await positions('Title'), [[0, 90]]);
   await click('UNDO'); await waitState('assets 0; clips 0');
   assert.equal((await client.byLabel('Reorder Titles', { role: 'Button' })).length, 0);
   await click('REDO'); await waitState('assets 0; clips 1');
   assert.deepEqual(await positions('Title'), [[0, 90]]);
   await client.click(await item('Title'));
   await titleField('Timeline end', 3);
   await fileAction('SAVE AS', join(output, 'title-only.oqvideo.json'), 'Save project');
   await waitState('Project saved');
   const exported = join(output, 'title-only.mp4');
   await fileAction('EXPORT', exported, 'Export MP4'); await waitState('Export complete');
   const image = mediaCommand(['-i', exported, '-frames:v', '1', '-vf', 'scale=320:180',
     '-pix_fmt', 'rgb24', '-f', 'rawvideo', '-']);
   assert.equal(image.length, 320 * 180 * 3);
   assert(whitePixels(image, 320, 3, 0, 0, 320, 180) > 40);
   assert(image.subarray(0, 3).every(channel => channel < 5));
   await close();
 });
 const subtitleProject = join(output, 'subtitles.oqvideo.json');
 const importSrt = path => fileAction('Import SRT', path, 'Import subtitles');
 const expectSubtitle = async () => client.waitFor(async () => {
   const preview = await client.find({ label: 'Video preview', role: 'Image' });
   const window = await client.windowProperties();
   const png = decodePng(await client.screenshot());
   const sx = png.width * (window.scaleFactor || 1) / window.size.width;
   const sy = png.height * (window.scaleFactor || 1) / window.size.height;
   const fit = Math.min(preview.size.width / 320, preview.size.height / 180);
   const { x, y } = center(preview);
   return whitePixels(png.pixels, png.width, png.channels,
     Math.floor((x - 100 * fit) * sx), Math.floor((y + 45 * fit) * sy),
     Math.ceil((x + 100 * fit) * sx), Math.ceil((y + 79 * fit) * sy)) > 35;
 }, { description: 'white subtitle glyphs near the bottom of the program preview' });
 await step('Author and import multiline subtitles and burn in only their scheduled MP4 frames', async () => {
   const fixture = JSON.parse(await readFile(join(output, 'title-seed.oqvideo.json'), 'utf8'));
   fixture.data.tracks[0].clips[0].duration = 10;
   const seed = join(output, 'subtitle-seed.oqvideo.json');
   await writeFile(seed, JSON.stringify(fixture));
   await launch(seed); await waitState('clips 1');
   await click('NEXT'); await click('NEXT'); await waitState('playhead 2 frames');
   await click('Add subtitle'); await waitState('clips 2');
   assert.deepEqual(await positions('Subtitle'), [[2, 60]]);
   const manual = 'Handwritten cue\n第二行';
   await client.setValue(await clipControl('Subtitle text'), manual);
   await client.click(await clipControl('Apply subtitle text', 'Button'));
   await client.waitFor(async () => (await positions('Handwritten cue')).length === 1,
     { description: 'applied text appears as the cue label' });
   await titleField('Timeline end', 4);
   assert.deepEqual(await positions('Handwritten cue'), [[2, 2]]);
   await expectSubtitle();
   await snapshot('20-subtitle-overlay');
   await client.click(await client.find({ label: 'Video preview', role: 'Image' }));
   assert.equal((await clipControl('Timeline start')).accessibleValue, '0');
   assert.equal((await clipControl('Timeline end')).accessibleValue, '10');

   const source = join(output, 'subtitles.srt');
   const firstImported = 'First SRT cue\n另一个段落';
   await writeFile(source, '\uFEFF' + [
     '1', '00:00:00,155 --> 00:00:00,221', ...firstImported.split('\n'), '',
     '2', '00:00:00,221 --> 00:00:00,276', 'Second SRT cue', '',
   ].join('\r\n'));
   await importSrt(source); await waitState('Imported 2 subtitle cues');
   assert.deepEqual(await positions('First SRT cue'), [[5, 2]]);
   assert.deepEqual(await positions('Second SRT cue'), [[7, 2]]);
   await click('UNDO'); await waitState('clips 2');
   assert.deepEqual(await positions('First SRT cue'), []);
   assert.deepEqual(await positions('Handwritten cue'), [[2, 2]]);
   await click('REDO'); await waitState('clips 4');
   assert.deepEqual(await positions('Second SRT cue'), [[7, 2]]);

   await fileAction('SAVE AS', subtitleProject, 'Save project'); await waitState('Project saved');
   const saved = JSON.parse(await readFile(subtitleProject, 'utf8')).data;
   const [track] = saved.tracks.filter(track => track.subtitles);
   assert.equal(track.name, 'Subtitles');
   assert.deepEqual(track.clips.map(clip => [clip.timeline_start, clip.duration, clip.title.text]),
     [[2, 2, manual], [5, 2, firstImported], [7, 2, 'Second SRT cue']]);
   await close();
   await launch(subtitleProject); await waitState('clips 4');
   await client.click(await item('First SRT cue'));
   assert.equal((await clipControl('Subtitle text')).accessibleValue, firstImported);
   await expectSubtitle();

   const exported = join(output, 'burned-subtitles.mp4');
   await fileAction('EXPORT', exported, 'Export MP4'); await waitState('Export complete');
   const pixels = mediaCommand(['-i', exported, '-map', '0:v:0', '-vsync', '0', '-pix_fmt', 'rgb24', '-f', 'rawvideo', '-'],
     { maxBuffer: 4 * 1024 * 1024 });
   const stride = 320 * 180 * 3;
   assert.equal(pixels.length, stride * 10);
   for (let frame = 0; frame < 10; frame++) {
     const image = pixels.subarray(frame * stride, (frame + 1) * stride);
     const visible = [2, 3, 5, 6, 7, 8].includes(frame);
     const white = whitePixels(image, 320, 3, 70, 135, 250, 172, 185);
     assert(visible ? white > 60 : white === 0, `frame ${frame} has ${white} subtitle pixels`);
     let darkened = 0;
     for (let y = 135; y < 172; y++) {
       for (let x = 70; x < 250; x++) {
         const [r, g, b] = image.subarray((y * 320 + x) * 3, (y * 320 + x) * 3 + 3);
         if (r < 25 && g < 25 && b > 85 && b < 175) darkened++;
       }
     }
     assert(visible ? darkened > 250 : darkened === 0,
       `frame ${frame} has ${darkened} pixels darkened by the subtitle backing`);
     const [r, g, b] = image.subarray((90 * 320 + 160) * 3, (90 * 320 + 160) * 3 + 3);
     assert(r < 25 && g < 25 && b > 220, `frame ${frame} preserves the video above the subtitle`);
   }
   await close();
 }, { core: true });
 await step('Reject invalid or conflicting subtitles without changing the edit', async () => {
   await launch(subtitleProject); await waitState('clips 4');
   const conflicting = join(output, 'conflicting.srt');
   await writeFile(conflicting, [
     '1', '00:00:00,034 --> 00:00:00,122', 'Conflict', '',
     '2', '00:00:00,500 --> 00:00:00,700', 'Would otherwise fit',
   ].join('\n'));
   await importSrt(conflicting); await waitState('Imported subtitles overlap');
   assert.deepEqual(await positions('Would otherwise fit'), []);
   assert.deepEqual(await positions('Handwritten cue'), [[2, 2]]);
   const malformed = join(output, 'malformed.srt');
   await writeFile(malformed, '1\n00:60:00,000 --> 00:60:03,000\nMalformed\n');
   await importSrt(malformed); await waitState('Invalid timestamp or index in SRT cue 1');
   assert.deepEqual(await positions('Malformed'), []);

   const addSubtitle = await client.find({ label: 'Add subtitle', role: 'Button' });
   await client.click(await item('Handwritten cue'));
   assert.equal((await clipControl('Timeline start')).accessibleValue, '2');
   await client.click(addSubtitle); await waitState('A subtitle already covers the playhead');
   await client.setValue(await clipControl('Subtitle text'), '   ');
   await client.click(await clipControl('Apply subtitle text', 'Button'));
   await waitState('Subtitle text must contain 1 to 256 characters');
   assert.deepEqual(await positions('Handwritten cue'), [[2, 2]]);
   await click('Lock Subtitles');
   await client.click(addSubtitle); await waitState('Subtitles is locked');
   assert.deepEqual(await positions('Handwritten cue'), [[2, 2]]);
   await click('UNDO');
   await close();
 });
 const stillProject = join(output, 'stills.oqvideo.json');
 const importMedia = path => fileAction('IMPORT', path, 'Import media');
 const overlay = join(output, 'overlay.png');
 const makeOverlay = () => {
   const rgba = Buffer.alloc(160 * 90 * 4);
   for (let y = 29; y < 61; y++) {
     for (let x = 45; x < 145; x++) {
       const at = (y * 160 + x) * 4;
       if (x < 115 || x >= 120) {
         rgba[at] = 255;
         rgba[at + 3] = x < 115 ? 255 : 128;
       }
     }
   }
   mediaCommand(['-f', 'rawvideo', '-pix_fmt', 'rgba', '-s', '160x90', '-i', '-', '-frames:v', '1',
     '-c:v', 'png', '-threads', '1', '-y', overlay], { input: rgba });
 };
 await step('Import and trim a transparent PNG over video, reopen it and decode the exact output', async () => {
   const seed = join(output, 'subtitle-seed.oqvideo.json');
   makeOverlay();
   await launch(seed); await waitState('clips 1');
   await importMedia(overlay); await waitState('assets 3');
   const card = await assetCard('overlay.png');
   assert(card.accessibleDescription.includes('160 × 90 · Still image'));
   await expectColor('red');
   await click('ADD TRACK');
   await click('Add overlay.png to timeline'); await waitState('clips 2');
   assert.deepEqual(await positions('overlay.png'), [[0, 90]]);
   await titleField('Timeline start', 2);
   await titleField('Timeline end', 6);
   assert.deepEqual(await positions('overlay.png'), [[2, 4]]);
   assert.equal((await client.byLabel('Speed (%)', { role: 'TextInput' })).length, 0);
   await client.click(await item('overlay.png')); await expectColor('red');
   await snapshot('21-still-overlay');
   await titleField('Opacity (%)', 0);
   await expectColor('blue');
   await click('UNDO'); await expectColor('red');
   await fileAction('SAVE AS', stillProject, 'Save project'); await waitState('Project saved');
   const saved = JSON.parse(await readFile(stillProject, 'utf8')).data;
   const still = saved.assets.find(asset => asset.name === 'overlay.png');
   assert.equal(still.still, true);
   assert.equal(still.duration_pts, null);
   assert.equal(still.path, 'overlay.png');
   assert.equal(saved.audio_tracks, undefined);
   await close();

   await launch(stillProject); await waitState('clips 2');
   await expectColor('blue');
   await client.click(await item('overlay.png')); await expectColor('red');
   await click('PREVIOUS'); await waitState('playhead 1 frames');
   await click('PLAY'); await waitState('playhead 9 frames');
   await expectColor('blue');
   const exported = join(output, 'still-overlay.mp4');
   await fileAction('EXPORT', exported, 'Export MP4'); await waitState('Export complete');
   const pixels = mediaCommand(['-i', exported, '-map', '0:v:0', '-vsync', '0', '-pix_fmt', 'rgb24',
     '-f', 'rawvideo', '-'], { maxBuffer: 4 * 1024 * 1024 });
   const stride = 320 * 180 * 3;
   assert.equal(pixels.length, stride * 10);
   const read = (frame, x, y) => pixels.subarray(frame * stride + (y * 320 + x) * 3,
     frame * stride + (y * 320 + x) * 3 + 3);
   const compare = (pixel, expected) => expected.every((value, channel) => Math.abs(pixel[channel] - value) < 23);
   for (let frame = 0; frame < 10; frame++) {
     const visible = frame >= 2 && frame < 6;
     const middle = read(frame, 160, 90);
     const translucent = read(frame, 264, 90);
     assert(compare(middle, visible ? [255, 0, 0] : [0, 0, 255]),
       `frame ${frame} center RGB is ${[...middle]}`);
     assert(compare(translucent, visible ? [188, 0, 187] : [0, 0, 255]),
       `frame ${frame} partially transparent patch RGB is ${[...translucent]}`);
     assert(compare(read(frame, 22, 22), [0, 0, 255]), `frame ${frame} preserves the lower layer`);
   }
   await close();
 }, { core: true });
 await step('Reject a corrupt still and relink an image by its original content', async () => {
   const moved = join(output, 'moved-overlay.asset');
   await rename(overlay, moved);
   await launch(stillProject); await waitState('clips 2');
   assert.equal((await client.byLabel('Input color', { role: 'Combobox' })).length, 1);
   const card = await assetCard('overlay.png');
   assert(card.accessibleDescription.endsWith('; offline'));
   await client.click(card);
   await waitState('Media offline');
   assert.equal((await client.byLabel('Input color', { role: 'Combobox' })).length, 0);
   await fileAction('RELINK', moved, 'Relink media');
   await client.waitFor(async () => (await assetCard('overlay.png')).accessibleDescription.endsWith('; online'),
     { description: 'the matching relocated image is online' });
   await client.click(await assetCard('overlay.png'));
   await expectColor('red');
   const broken = join(output, 'broken.png');
   await writeFile(broken, 'damaged bytes');
   await importMedia(broken); await waitState('Cannot decode still image');
   assert.equal((await client.byLabel('broken.png', { role: 'Button' })).length, 0);
   await fileAction('SAVE AS', join(output, 'relinked-still.oqvideo.json'), 'Save project');
   await waitState('Project saved');
   await close();
   await writeFile(overlay, await readFile(moved));
 });
 await step('Use a rotated JPEG for overwrite and dissolve between two stills', async () => {
   const jpeg = join(output, 'rotated.jpg');
   const encoded = mediaCommand(['-f', 'lavfi', '-i', 'color=c=blue:s=160x90', '-frames:v', '1',
     '-c:v', 'mjpeg', '-threads', '1', '-f', 'mjpeg', '-']);
   const exif = Buffer.from('ffe1002245786966000049492a0008000000010012010300010000000600000000000000', 'hex');
   await writeFile(jpeg, Buffer.concat([encoded.subarray(0, 2), exif, encoded.subarray(2)]));
   await launch();
   await importMedia(jpeg); await waitState('assets 1');
   assert((await assetCard('rotated.jpg')).accessibleDescription.includes('90 × 160 · Still image'));
   await expectColor('blue');
   await click('Insert'); await waitState('clips 1');
   await titleField('Timeline end', 8);
   await client.click(await item('rotated.jpg'));
   await click('NEXT'); await click('NEXT'); await click('NEXT'); await click('NEXT');
   await waitState('playhead 4 frames');
   await importMedia(overlay); await waitState('assets 2');
   await expectColor('red');
   await click('Overwrite'); await waitState('clips 2');
   assert.deepEqual(await positions('rotated.jpg'), [[0, 4]]);
   assert.deepEqual(await positions('overlay.png'), [[4, 90]]);
   await titleField('Timeline end', 8);
   await client.click(await item('rotated.jpg'));
   await dissolveField(4); await expectDissolve(2, 5);
   for (let i = 0; i < 4; i++) await click('NEXT');
   await waitState('playhead 4 frames');
   await expectColor('blend');
   await fileAction('SAVE AS', join(output, 'still-dissolve.oqvideo.json'), 'Save project');
   await waitState('Project saved');
   await close();
 });
 const batchAssetsProject = join(output, 'batch-media.oqvideo.json');
 await step('Batch import deduplicates files and prepares one verified proxy per asset', async () => {
   const invalid = join(output, 'batch-invalid.mp4');
   await writeFile(invalid, 'Not a video');
   await launch(); await waitState('assets 0');
   await click('Import files');
   const input = await client.find({ label: 'Import file paths', role: 'TextInput' });
   await client.setValue(input, [background, foreground, background, invalid].join('\n'));
   await click('Import selected files');
   await waitState('Imported 2 · 1 duplicates · 1 failed');
   await waitState('assets 2');
   await click('UNDO'); await waitState('assets 0');
   await click('REDO'); await waitState('assets 2');
   await click('Prepare all proxies');
   await waitState('Proxy batch complete · 2 of 2 ready');
   const files = await readdir(join(output, 'proxies'));
   assert(files.filter(name => /^[a-f0-9]{64}\.mp4$/.test(name)).length >= 2);
   await fileAction('SAVE AS', batchAssetsProject, 'Save project'); await waitState('Project saved');
   await close();
 }, { core: true });
 await step('Batch relink finds renamed originals only by content, with one undo step', async () => {
   const project = JSON.parse(await readFile(exportProject, 'utf8'));
   const folder = join(output, 'relinked-media');
   const nested = join(folder, 'camera-a');
   await mkdir(nested, { recursive: true });
   for (const [index, asset] of project.data.assets.entries()) {
     const content = await readFile(resolve(output, asset.path));
     await writeFile(join(nested, `take-${index}.mp4`), content);
     await writeFile(join(folder, asset.name), Buffer.alloc(content.length, 17));
     asset.path = `absent/${asset.name}`;
   }
   const path = join(output, 'relink-folder-seed.oqvideo.json');
   await writeFile(path, JSON.stringify(project));
   await launch(path); await waitState('assets 2');
   assert((await assetCard('background.mp4')).accessibleDescription.includes('offline'));
   await fileAction('Relink folder', folder, 'Search folder');
   await waitState('Relinked 2 originals · 0 still offline');
   assert((await assetCard('background.mp4')).accessibleDescription.includes('online'));
   assert((await assetCard('foreground.mp4')).accessibleDescription.includes('online'));
   await click('UNDO');
   assert((await assetCard('background.mp4')).accessibleDescription.includes('offline'));
   assert((await assetCard('foreground.mp4')).accessibleDescription.includes('offline'));
   await click('REDO');
   await click('SAVE'); await waitState('Project saved');
   const verified = JSON.parse(await readFile(path, 'utf8')).data;
   assert.deepEqual(verified.assets.map(asset => asset.path),
     ['relinked-media/camera-a/take-0.mp4', 'relinked-media/camera-a/take-1.mp4']);
   await close();
 }, { core: true });
 await step('Cancel a proxy batch while preserving the usable originals and cached work', async () => {
   const fixture = JSON.parse(await readFile(batchAssetsProject, 'utf8'));
   const folder = join(output, 'new-batch');
   await mkdir(folder, { recursive: true });
   for (const asset of fixture.data.assets) {
     const path = join(folder, asset.name);
     const fresh = generate(`cancel-${asset.name}`, asset.name === 'background.mp4' ? 'blue' : 'red', 12, 30, '1920x1080');
     await writeFile(path, await readFile(fresh));
   }
   await launch(); await waitState('assets 0');
   await click('Import files');
   const input = await client.find({ label: 'Import file paths', role: 'TextInput' });
   await client.setValue(input, fixture.data.assets.map(asset => join(folder, asset.name)).join('\n'));
   await click('Import selected files'); await waitState('Imported 2');
   await click('Prepare all proxies');
   await click('Cancel proxy'); await waitState('Proxy batch cancelled');
   assert(!(await readdir(join(output, 'proxies'))).some(name => name.startsWith('.building')));
   await client.click(await assetCard('background.mp4'));
   await expectColor('blue');
   await fileAction('SAVE AS', join(output, 'cancel-proxy-batch.oqvideo.json'), 'Save project'); await waitState('Project saved');
   await close();
 });
 const proxyProject = join(output, 'proxy-seed.oqvideo.json');
 const cacheFiles = async () => (await readdir(join(output, 'proxies'))).filter(name => /^[a-f0-9]{64}\.mp4$/.test(name));
 await step('Preview from a verified 360p proxy while export requires the original media', async () => {
   const fixture = JSON.parse(await readFile(exportProject, 'utf8'));
   const video = fixture.data.assets.find(asset => asset.name === 'background.mp4');
   fixture.data.tracks = fixture.data.tracks.filter(track => track.clips.some(clip => clip.asset_id === video.id));
   await writeFile(proxyProject, JSON.stringify(fixture));
   await launch(proxyProject); await waitState('clips 1');
   await client.click(await assetCard(video.name));
   await expectColor('blue');
   await click('Prepare proxy');
   await waitState('Proxy ready');
   const [proxy] = await cacheFiles();
   assert(proxy);
   const proxyPath = join(output, 'proxies', proxy);
   const info = spawnSync(ffmpeg, ['-hide_banner', '-i', proxyPath, '-t', '0', '-f', 'null', '-'],
     { env: mediaEnv, encoding: 'utf8' });
   assert.equal(info.status, 0, info.stderr);
   assert.match(info.stderr, /Video: h264[^\n]*640x360/);
   await click('Use proxies'); await waitState('Proxy preview enabled');
   await expectColor('blue');
   const original = resolve(output, video.path);
   const moved = original + '.offline';
   await rename(original, moved);
   try {
     await click('NEXT'); await waitState('Frame 1 · source PTS 33333333 ns');
     await click('PLAY'); await waitState('Frame 8 · source PTS 266666667 ns');
     await expectColor('blue');
     const failed = join(output, 'proxy-missing-original.mp4');
     await fileAction('EXPORT', failed, 'Export MP4'); await waitState('Export failed');
     assert(!(await readdir(output)).includes('proxy-missing-original.mp4'));
   } finally {
     await rename(moved, original);
   }
   const fromOriginal = join(output, 'original-with-proxy-enabled.mp4');
   await exportRange(fromOriginal, 1, 4); await waitState('Export complete');
   assert((await readdir(output)).includes('original-with-proxy-enabled.mp4'));
   await click('Use originals'); await waitState('Original preview enabled');
   await expectColor('blue');
   await close();
 }, { core: true });
 await step('Reuse a cached proxy after reopening and cancel an unrelated preparation', async () => {
   await launch(proxyProject); await waitState('clips 1');
   await client.click(await assetCard('background.mp4'));
   const previous = await cacheFiles();
   await click('Prepare proxy'); await waitState('Proxy ready');
   assert.deepEqual(await cacheFiles(), previous);
   const uncached = generate('single-cancel.mp4', 'red', 14, 24, '1920x1080');
   await fileAction('IMPORT', uncached, 'Import media'); await waitState('assets 3');
   await click('Prepare proxy');
   await click('Cancel proxy'); await waitState('Proxy cancelled');
   assert.deepEqual(await cacheFiles(), previous);
   assert(!(await readdir(join(output, 'proxies'))).some(name => name.startsWith('.building')));
   await fileAction('SAVE AS', join(output, 'proxy-single-cancel.oqvideo.json'), 'Save project');
   await waitState('Project saved');
   await close();
 });
 await step('Seek and play through the last cut of an eight-minute proxy project', async () => {
   const path = join(output, 'proxy-many-cuts.oqvideo.json');
   const fixture = JSON.parse(await readFile(proxyProject, 'utf8'));
   const [track] = fixture.data.tracks;
   const [clip] = track.clips;
   track.clips = Array.from({ length: 160 }, (_, index) => ({
     ...clip, id: `proxy-stress-${index}`, timeline_start: index * 90, duration: 90,
   }));
   fixture.data.markers = [{ id: 'proxy-last', frame: 14397, name: 'Last proxy frames' }];
   await writeFile(path, JSON.stringify(fixture));
   await launch(path); await waitState('clips 160');
   await client.click(await assetCard('background.mp4'));
   await click('Prepare proxy'); await waitState('Proxy ready');
   await click('Use proxies'); await waitState('Proxy preview enabled');
   await click('FIT TIMELINE');
   await click('Go to Last proxy frames'); await waitState('Frame 14397 · source PTS 2900000000 ns');
   await click('PLAY'); await waitState('Frame 14399 · source PTS 2966666667 ns');
   await expectColor('blue');
   await close();
 });
 await proxyRoundTrip();
 const chooseExportOption = async (label, steps, value) => {
   const box = await client.find({ label, role: 'Combobox' });
   await client.click(box);
   for (let i = 0; i < steps; i++) await client.key('\uf701');
   await client.key('\n');
   await client.waitFor(async () => (await client.properties(box.handle)).accessibleValue === value,
     { description: `export option ${value}` });
 };
 await step('Export with a scaled resolution, a different cadence and a bitrate preset', async () => {
   await launch(exportProject); await waitState('clips 3');
   const path = join(output, '720p-export.mp4');
   await click('EXPORT');
   await client.setValue(await rangeStart(), '1');
   await client.setValue(await rangeEnd(), '11');
   await chooseExportOption('Export resolution', 1, 'Fit 720p');
   await chooseExportOption('Export frame rate', 1, '24 fps');
   await chooseExportOption('Export bitrate', 2, 'High');
   await client.setValue(await client.find({ label: 'File path', role: 'TextInput' }), path);
   await click('Export MP4'); await waitState('Export complete');
   const metadata = spawnSync(ffmpeg, ['-hide_banner', '-i', path, '-map', '0:v:0', '-t', '0', '-f', 'null', '-'],
     { env: mediaEnv, encoding: 'utf8' });
   assert.equal(metadata.status, 0, metadata.stderr);
   assert.match(metadata.stderr, /Video: h264[^\n]*1280x720[^\n]*24 fps/);
   const frames = mediaCommand(['-i', path, '-map', '0:v:0', '-vsync', '0', '-vf', 'scale=32:18',
     '-pix_fmt', 'rgb24', '-f', 'rawvideo', '-']);
   const stride = 32 * 18 * 3;
   assert.equal(frames.length, 8 * stride);
   const expected = ['blue', 'blue', 'red', 'red', 'blue', 'blue', 'blue', 'black'];
   for (let i = 0; i < expected.length; i++) {
     const pixel = (9 * 32 + 16) * 3 + i * stride;
     const [r, , b] = frames.subarray(pixel, pixel + 3);
     assert.equal(r > 220 ? 'red' : b > 220 ? 'blue' : 'black', expected[i],
       `sample ${i} reflects the 24 fps export cadence`);
   }
   assert.equal((await client.byLabel('Export resolution', { role: 'Combobox' })).length, 0);
   await close();
 }, { core: true });
 await step('Export a rebased SRT sidecar and protect the existing destination', async () => {
   await launch(subtitleProject); await waitState('clips 4');
   const path = join(output, 'exported-cues.srt');
   const exportCues = async () => {
     await click('Export SRT');
     await client.setValue(await rangeStart(), '3');
     await client.setValue(await rangeEnd(), '8');
     await client.setValue(await client.find({ label: 'File path', role: 'TextInput' }), path);
     await click('Save SRT');
   };
   await exportCues(); await waitState('Subtitle file exported');
   const saved = await readFile(path, 'utf8');
   assert.equal(saved, [
     '1', '00:00:00,000 --> 00:00:00,033', 'Handwritten cue', '第二行', '',
     '2', '00:00:00,066 --> 00:00:00,133', 'First SRT cue', '另一个段落', '',
     '3', '00:00:00,133 --> 00:00:00,166', 'Second SRT cue', '', '',
   ].join('\n'));
   await exportCues(); await waitState('SRT export failed');
   assert.equal(await readFile(path, 'utf8'), saved);
   await close();
 }, { core: true });
 const spatialProject = join(output, 'spatial.oqvideo.json');
 const transformControl = async (label, value) => enter(await clipControl(label), value);
 const spatialButton = async label => client.click(await clipControl(label, 'Button'));
 await step('Select, drag, copy, paste and delete a group of timeline keyframes', async () => {
   const fixture = JSON.parse(await readFile(exportProject, 'utf8'));
   const track = fixture.data.tracks.find(track => track.clips.some(clip =>
     fixture.data.assets.find(asset => asset.id === clip.asset_id)?.name === 'background.mp4'));
   const [clip] = track.clips;
   clip.duration = 12;
   clip.crop_keys = [
     { time: 0, value: { left: 0, top: 0, right: 0, bottom: 0 } },
     { time: 3, value: { left: 0.8, top: 0, right: 0, bottom: 0 } },
   ];
   fixture.data.tracks = [track];
   const seed = join(output, 'keyframe-track.oqvideo.json');
   await writeFile(seed, JSON.stringify(fixture));
   await launch(seed); await waitState('clips 1');
   await client.click(await item('background.mp4'));
   await click('Show keyframes');
   const channel = await client.find({ label: 'Animation property', role: 'Combobox' });
   await client.click(channel);
   await client.key('\uf701'); await client.key('\uf701'); await client.key('\n');
   await client.waitFor(async () => (await client.properties(channel.handle)).accessibleValue === 'Crop',
     { description: 'the crop parameter track is active' });
   const key = async frame => (await buttons(`Keyframe ${frame}`))[0];
   await client.click(await key(0));
   await client.click(await key(3), { modifiers: { shift: true } });
   assert.equal((await key(0)).accessibleDescription, 'Selected');
   assert.equal((await key(3)).accessibleDescription, 'Selected');
   await click('Copy keyframes');
   const rail = await client.find({ label: 'Clip keyframe track' });
   const first = await key(0);
   const origin = center(first);
   await client.drag(first, { x: origin.x + 2 * (rail.size.width - 24) / 11, y: origin.y });
   assert(await key(2)); assert(await key(5));
   await click('UNDO');
   assert(await key(0)); assert(await key(3));
   for (let i = 0; i < 6; i++) await click('NEXT');
   await click('Paste keyframes');
   assert(await key(6)); assert(await key(9));
   await client.click(await key(6));
   await client.click(await key(9), { modifiers: { shift: true } });
   await click('Delete keyframes');
   assert(!(await key(6))); assert(!(await key(9)));
   await click('UNDO');
   assert(await key(6)); assert(await key(9));
   await click('SAVE'); await waitState('Project saved');
   const stored = JSON.parse(await readFile(seed, 'utf8')).data.tracks[0].clips[0].crop_keys;
   assert.deepEqual(stored.map(key => key.time), [0, 3, 6, 9]);
   await click('Hide keyframes');
   assert.equal((await client.byLabel('Clip keyframe track')).length, 0);
   await close();
 }, { core: true });
 await step('Set Hold and Bezier and drag both tangent handles with matching export pixels', async () => {
   const fixture = JSON.parse(await readFile(exportProject, 'utf8'));
   const [upper, , lower] = fixture.data.tracks;
   upper.clips = upper.clips.slice(0, 1);
   Object.assign(upper.clips[0], { timeline_start: 0, duration: 5 });
   upper.clips[0].transform.scale = 1;
   upper.clips[0].crop_keys = [
     { time: 0, value: { left: 0, top: 0, right: 0, bottom: 0 } },
     { time: 4, value: { left: 0.95, top: 0, right: 0, bottom: 0 } },
   ];
   lower.clips[0].duration = 5;
   fixture.data.tracks = [upper, lower];
   const seed = join(output, 'bezier-curve.oqvideo.json');
   await writeFile(seed, JSON.stringify(fixture));
   await launch(seed); await waitState('clips 2');
   await client.click(await item('foreground.mp4'));
   await click('Show keyframes');
   const choose = async (label, keys, value) => {
     const box = () => client.find({ label, role: 'Combobox' });
     await client.click(await box());
     for (const key of keys) await client.key(key);
     await client.key('\n');
     await client.waitFor(async () => (await box()).accessibleValue === value,
       { description: `interpolation choice ${value}` });
   };
   await choose('Animation property', ['\uf701', '\uf701'], 'Crop');
   await click('Keyframe 0');
   await choose('Keyframe interpolation', ['\uf700'], 'Hold');
   for (let i = 0; i < 3; i++) await click('NEXT');
   await expectColor('red');
   await click('NEXT'); await expectColor('blue');
   await choose('Keyframe interpolation', ['\uf701'], 'Linear');
   await click('PREVIOUS'); await expectColor('blue');
   await click('PREVIOUS'); await expectColor('red');
   await choose('Keyframe interpolation', ['\uf701', '\uf701'], 'Bezier');
   const plot = await client.find({ label: 'Bezier curve' });
   const graph = plot.absolutePosition;
   const dragHandle = async (label, x, y) => client.drag(await client.find({ label, role: 'Button' }), {
     x: graph.x + x * plot.size.width,
     y: graph.y + (1 - y) * plot.size.height,
   });
   await dragHandle('First Bezier handle', 0.10, 0.90);
   await dragHandle('Second Bezier handle', 0.35, 0.95);
   await click('PREVIOUS'); await waitState('playhead 1 frames'); await expectColor('blue');
   const path = join(output, 'bezier-curve.mp4');
   await fileAction('EXPORT', path, 'Export MP4'); await waitState('Export complete');
   const pixels = mediaCommand(['-i', path, '-map', '0:v:0', '-vsync', '0', '-vf', 'scale=32:18',
     '-pix_fmt', 'rgb24', '-f', 'rawvideo', '-']);
   const center = (32 * 18 + 9 * 32 + 16) * 3;
   assert(pixels[center + 2] > 200 && pixels[center] < 25, 'the second output frame is blue');
   await click('SAVE'); await waitState('Project saved');
   const key = JSON.parse(await readFile(seed, 'utf8')).data.tracks[0].clips[0].crop_keys[0];
   for (const [property, expected] of Object.entries({ x1: 0.10, y1: 0.90, x2: 0.35, y2: 0.95 })) {
     assert(Math.abs(key.curve.bezier[property] - expected) < 0.04, property);
   }
   await close();
 }, { core: true });
 await step('Queue range exports, preserve snapshots, cancel and retry individual jobs', async () => {
   const seed = join(output, 'queue-edit.oqvideo.json');
   await writeFile(seed, await readFile(exportProject));
   const existing = join(output, 'queue-existing.mp4');
   const snapshotOutput = join(output, 'queue-snapshot.mp4');
   const cancelled = join(output, 'queue-cancelled.mp4');
   await writeFile(existing, 'Keep this existing delivery');
   await launch(seed); await waitState('clips 3');
   const add = async (path, start, end) => {
     await click('EXPORT');
     await client.setValue(await client.find({ label: 'Export start frame', role: 'TextInput' }), String(start));
     await client.setValue(await client.find({ label: 'Export end frame (exclusive)', role: 'TextInput' }), String(end));
     await client.setValue(await client.find({ label: 'File path', role: 'TextInput' }), path);
     await click('Add to queue');
   };
   await add(existing, 0, 2); await waitState('Delivery added to render queue');
   await add(snapshotOutput, 3, 6);
   await add(snapshotOutput, 0, 3); await waitState('already uses that output path');
   assert.equal((await client.find({ label: 'File path', role: 'TextInput' })).accessibleValue, snapshotOutput);
   await click('Cancel');
   await add(cancelled, 0, 2);
   const job = name => client.find({ label: new RegExp(`Delivery .*queue-${name}[.]mp4$`), role: 'Groupbox' });
   const phase = (name, prefix) => client.waitFor(async () =>
     (await job(name)).accessibleDescription.startsWith(prefix),
     { description: `${name} job ${prefix}` });
   await click('Render queue');
   await phase('snapshot', 'Pending');
   await click('Cancel job', true); await phase('cancelled', 'Cancelled');
   await click('Close queue');
   await click('SEQUENCE'); await click('New sequence'); await waitState('clips 0'); await click('Cancel');
   await click('Render queue'); await click('Start render queue');
   await phase('existing', 'Failed'); await phase('snapshot', 'Complete');
   assert.equal(await readFile(existing, 'utf8'), 'Keep this existing delivery');
   assert.equal((await readdir(output)).includes('queue-cancelled.mp4'), false);
   const stride = 320 * 180 * 3;
   const frames = mediaCommand(['-i', snapshotOutput, '-an', '-pix_fmt', 'rgb24', '-f', 'rawvideo', '-'], { maxBuffer: stride * 4 });
   assert.equal(frames.length, stride * 3);
   const [r, g, b] = frames.subarray((90 * 320 + 160) * 3);
   assert(r > 225 && g < 25 && b < 25);
   await rename(existing, join(output, 'queue-preserved.mp4'));
   await click('Retry job'); await phase('existing', 'Pending');
   await click('Start render queue'); await phase('existing', 'Complete');
   assert((await readFile(existing)).length > 512);
   await click('Close queue');
   await click('SAVE'); await waitState('Project saved');
   await close();
 }, { core: true });
 await step('Cancel a running queued delivery and keep its partial output unpublished', async () => {
   const model = JSON.parse(await readFile(exportProject, 'utf8'));
   const base = model.data.tracks.at(-1);
   const clip = base.clips[0];
   model.data.tracks = [{ ...base, clips: Array.from({ length: 20 }, (_, index) => ({
     ...clip, id: `queue-long-${index}`, timeline_start: index * 80, duration: 80,
   })) }];
   const seed = join(output, 'queue-cancel-edit.oqvideo.json');
   const path = join(output, 'queue-cancel-active.mp4');
   await writeFile(seed, JSON.stringify(model));
   await launch(seed); await waitState('clips 20');
   await click('EXPORT');
   await client.setValue(await client.find({ label: 'File path', role: 'TextInput' }), path);
   await click('Add to queue'); await waitState('Delivery added to render queue');
   await click('Render queue'); await click('Start render queue');
   const row = () => client.find({ label: /Delivery .*queue-cancel-active[.]mp4$/, role: 'Groupbox' });
   await client.waitFor(async () => (await row()).accessibleDescription.startsWith('Rendering'),
     { description: 'the queued task is rendering' });
   await click('Cancel job');
   await client.waitFor(async () => (await row()).accessibleDescription.startsWith('Cancelled'),
     { description: 'the running queued task was cancelled' });
   assert(!(await readdir(output)).includes('queue-cancel-active.mp4'));
   await click('Close queue');
   await close();
 });
 await step('Scroll and edit a late clip in an 1,800-cut timeline without mounting every clip', async () => {
   const seed = JSON.parse(await readFile(exportProject, 'utf8'));
   const blue = seed.data.assets.find(asset => asset.name === 'background.mp4');
   const track = seed.data.tracks.find(track => track.clips.some(clip => clip.asset_id === blue.id));
   const source = track.clips[0];
   track.clips = Array.from({ length: 1800 }, (_, index) => ({
     ...source, id: `long-cut-${index}`, timeline_start: 10 * index, duration: 4,
   }));
   seed.data.tracks = [track];
   const fixture = join(output, 'long-timeline.oqvideo.json');
   await writeFile(fixture, JSON.stringify(seed));
   const launched = performance.now();
   await launch(fixture); await waitState('clips 1800');
   const availableMs = performance.now() - launched;
   const mounted = async visible => (await client.byLabel('background.mp4', { role: 'Button', visible }))
     .filter(item => item.accessibleDescription?.startsWith('Timeline start'));
   const instances = await client.waitFor(async () => {
     const match = /timeline window (\d+) video/.exec(await state());
     return match && Number(match[1]) > 0 && Number(match[1]);
   }, { description: 'hydrated timeline window' });
   report.longTimeline = { totalClips: 1800, mountedAtStart: instances, availableMs };
   assert(instances > 0 && instances < 40, 'initial UI contains a bounded clip window');
   await expectColor('blue');
   const viewport = await client.find({ id: 'EditorWindow::timeline_scroll' });
   await client.value('dispatch_pointer_scroll', {
     windowHandle: client.windowId, position: center(viewport), deltaX: -64000, deltaY: 0,
   });
   const later = await client.waitFor(async () => {
     const rows = await mounted(true);
     return rows.filter(row => Number(/^Timeline start (\d+)/.exec(row.accessibleDescription)?.[1]) > 2500).at(3);
   }, { description: 'a distant clip mounts after real timeline pan' });
   const start = Number(/^Timeline start (\d+)/.exec(later.accessibleDescription)[1]);
   await client.click(later);
   await waitState(`playhead ${start} frames`);
   await expectColor('blue');
   await click('Toggle timeline snapping');
   const selected = (await mounted(true)).find(row => row.accessibleDescription.startsWith(`Timeline start ${start} frames`));
   const point = center(selected);
   await client.drag(selected, { x: point.x + 12, y: point.y });
   await client.waitFor(async () => !(await mounted(true)).some(row => row.accessibleDescription.startsWith(`Timeline start ${start} frames`)),
     { description: 'dragged clip moved from its former position' });
   await click('UNDO');
   await client.waitFor(async () => (await mounted(true)).some(row => row.accessibleDescription.startsWith(`Timeline start ${start} frames`)),
     { description: 'undo restores a clip outside the first viewport' });
   await close();
 }, { core: true });
 await step('Read live histogram, waveform, RGB parade and vectorscope from the program monitor', async () => {
   await launch(exportProject); await waitState('clips 3'); await expectColor('blue');
   const scope = () => client.find({ label: 'Color scope', role: 'Combobox' });
   const plot = () => client.find({ label: 'Color scope plot', role: 'Image' });
   const choose = async (key, count) => {
     await client.click(await scope());
     for (let i = 0; i < count; i++) await client.key(key);
     await client.key('\n');
   };
   await choose('\uf701', 1);
   const channel = async color => client.waitFor(async () => {
     const info = (await plot()).accessibleDescription;
     const codes = /RGB peak levels · R (\d+) · G (\d+) · B (\d+)/.exec(info);
     if (!codes) return false;
     const [r, g, b] = codes.slice(1).map(Number);
     return color === 'blue' ? b > 220 && r < 25 && g < 25 : r > 220 && b < 25 && g < 25;
   }, { description: `histogram of actual ${color} pixels` });
   await channel('blue');
   for (let i = 0; i < 3; i++) await click('NEXT');
   // The red inset sits on blue, so use a frame without any blue backdrop.
   for (let i = 3; i < 12; i++) await click('NEXT');
   await channel('red');
   await snapshot('rgb-histogram');
   await choose('\uf701', 1);
   await client.waitFor(async () => {
     const codes = /Luma levels · (\d+)–(\d+)/.exec((await plot()).accessibleDescription);
     return codes && Number(codes[1]) >= 45 && Number(codes[2]) <= 65;
   }, { description: 'luma waveform of the red program frame' });
   await snapshot('luma-waveform');
   await choose('\uf701', 1);
   await client.waitFor(async () => {
     const levels = /RGB parade levels · R (\d+)–(\d+) · G (\d+)–(\d+) · B (\d+)–(\d+)/.exec((await plot()).accessibleDescription);
     return levels && Number(levels[2]) > 220 && Number(levels[4]) < 25 && Number(levels[6]) < 25;
   }, { description: 'red program frame in independent RGB parade bands' });
   await snapshot('rgb-parade');
   await choose('\uf701', 1);
   await client.waitFor(async () => {
     const levels = /Vectorscope · Cb (-?\d+)–(-?\d+) · Cr (-?\d+)–(-?\d+)/.exec((await plot()).accessibleDescription);
     return levels && Number(levels[1]) < -20 && Number(levels[4]) > 100;
   }, { description: 'red program frame in the correct vectorscope quadrant' });
   await snapshot('vectorscope');
   await choose('\uf700', 4);
   assert.equal((await scope()).accessibleValue, 'Scopes off');
   assert.equal((await client.byLabel('Color scope plot', { role: 'Image' })).length, 0);
   await close();
 }, { core: true });
 await step('Shape and feather a clip mask and export the same alpha composite', async () => {
   const fixture = JSON.parse(await readFile(exportProject, 'utf8'));
   fixture.data.tracks = fixture.data.tracks.filter(track => track.clips.length > 0);
   fixture.data.tracks[0].clips[0].transform.scale = 1;
   const seed = join(output, 'clip-mask.oqvideo.json');
   await writeFile(seed, JSON.stringify(fixture));
   await launch(seed); await waitState('clips 3');
   for (let i = 0; i < 3; i++) await click('NEXT');
   await client.click(await item('foreground.mp4'));
   const pick = async (keys, label) => {
     await client.click(await clipControl('Mask shape', 'Combobox'));
     for (const key of keys) await client.key(key);
     await client.key('\n');
     await client.waitFor(async () =>
       (await client.find({ label: 'Mask shape', role: 'Combobox', visible: false })).accessibleValue === label,
       { description: `mask shape ${label}` });
   };
   await pick(['\uf701'], 'Rectangle');
   for (const [label, value] of [['center X', 45], ['center Y', 45], ['width', 60], ['height', 60], ['feather', 25]]) {
     await enter(await clipControl(`Mask ${label} (%)`), value);
   }
   await enter(await clipControl('Mask width (%)'), 0);
   await waitState('width and height must be 1–100%');
   await pick(['\uf701'], 'Ellipse'); await expectColor('red');
   const path = join(output, 'ellipse-mask.mp4');
   await fileAction('EXPORT', path, 'Export MP4'); await waitState('Export complete');
   const stride = 320 * 180 * 3;
   const bytes = mediaCommand(['-i', path, '-an', '-pix_fmt', 'rgb24', '-f', 'rawvideo', '-'], { maxBuffer: stride * 20 });
   const pixel = (x, y) => bytes.subarray(3 * stride + (y * 320 + x) * 3, 3 * stride + (y * 320 + x) * 3 + 3);
   assert(pixel(160, 90)[0] > 225, 'ellipse interior stays opaque');
   assert(pixel(215, 119)[2] > 225 && pixel(215, 119)[0] < 25, 'ellipse removes the rectangle corner');
   const [r, g, b] = pixel(227, 81);
   assert(r > 90 && b > 90 && g < 30, `feather mixes red and blue, got ${r}, ${g}, ${b}`);
   await click('SAVE'); await waitState('Project saved');
   const stored = JSON.parse(await readFile(seed, 'utf8')).data.tracks[0].clips[0].mask;
   for (const [key, value] of Object.entries({ x: 0.45, y: 0.45, width: 0.6, height: 0.6, feather: 0.25 })) {
     assert(Math.abs(stored[key] - value) < 0.0001);
   }
   await close();
   await launch(seed);
   await client.click(await item('foreground.mp4'));
   assert.equal((await clipControl('Mask shape', 'Combobox')).accessibleValue, 'Ellipse');
   await pick(['\uf700'], 'Rectangle');
   await pick(['\uf700'], 'None');
   await click('SAVE'); await waitState('Project saved');
   assert.equal(JSON.parse(await readFile(seed, 'utf8')).data.tracks[0].clips[0].mask, undefined);
   await click('UNDO');
   assert.equal((await clipControl('Mask shape', 'Combobox')).accessibleValue, 'Rectangle');
   await click('UNDO');
   assert.equal((await clipControl('Mask shape', 'Combobox')).accessibleValue, 'Ellipse');
   await click('SAVE'); await waitState('Project saved');
   await close();
 }, { core: true });
 await maskTrackingRoundTrip(mediaCommand);
 await step('Choose track blend modes and match composite preview to decoded MP4', async () => {
   const fixture = JSON.parse(await readFile(exportProject, 'utf8'));
   fixture.data.tracks = fixture.data.tracks.filter(track => track.clips.length > 0);
   fixture.data.tracks[0].clips[0].transform.scale = 1;
   const seed = join(output, 'blend-modes.oqvideo.json');
   await writeFile(seed, JSON.stringify(fixture));
   await launch(seed); await waitState('clips 3');
   for (let i = 0; i < 3; i++) await click('NEXT');
   await expectColor('red');
   const mode = () => client.find({ label: 'Blend Video 3', role: 'Combobox' });
   const pickBlend = async (keys, name) => {
     await client.click(await mode());
     for (const key of keys) await client.key(key);
     await client.key('\n');
     await client.waitFor(async () => (await mode()).accessibleValue === name,
       { description: `selected blend mode ${name}` });
   };
   await pickBlend(['\uf701'], 'Multiply'); await expectColor('black');
   await pickBlend(['\uf701'], 'Screen'); await expectColor('magenta');
   const path = join(output, 'blend-screen.mp4');
   await fileAction('EXPORT', path, 'Export MP4'); await waitState('Export complete');
   const stride = 320 * 180 * 3;
   const bytes = mediaCommand(['-i', path, '-an', '-pix_fmt', 'rgb24', '-f', 'rawvideo', '-'], { maxBuffer: stride * 20 });
   const at = 3 * stride + (90 * 320 + 160) * 3;
   const [r, g, b] = bytes.subarray(at, at + 3);
   assert(r > 225 && g < 25 && b > 225, `screen gives magenta, got ${r}, ${g}, ${b}`);
   await pickBlend(['\uf701'], 'Overlay'); await expectColor('blue');
   await click('UNDO'); await expectColor('magenta');
   await click('SAVE'); await waitState('Project saved'); await close();
   await launch(seed);
   assert.equal((await mode()).accessibleValue, 'Screen');
   await click('Lock Video 3');
   assert.equal(Boolean((await mode()).accessibleEnabled), false);
   await click('UNDO');
   await close();
 }, { core: true });
 await step('Nest a live sequence at the playhead and export its internal cuts and audio', async () => {
   const fixture = JSON.parse(await readFile(exportProject, 'utf8'));
   const sound = JSON.parse(await readFile(audioProject, 'utf8')).data;
   const independent = sound.audio_tracks.find(track => track.clips.some(clip => clip.placement.kind === 'independent'));
   const tone = independent.clips.find(clip => clip.placement.kind === 'independent');
   const toneAsset = sound.assets.find(asset => asset.id === tone.asset_id);
   fixture.data.assets.push(toneAsset);
   fixture.data.audio_tracks = [{ ...independent, clips: [{
     ...tone, placement: { ...tone.placement, start: 0, duration: 28800 },
   }] }];
   fixture.data.tracks = fixture.data.tracks.filter(track => track.clips.length > 0);
   // The outgoing solid changes pose at child frame 4. The parent must use the child's animation clock.
   fixture.schema_version = fixture.data.schema_version = Math.max(fixture.schema_version, 27);
   fixture.data.tracks[0].clips[0].spatial_keys = [
     { time: 0, value: { x: 0, y: 0, scale: 0.5, rotation: 0 } },
     { time: 1, value: { x: 0.45, y: 0, scale: 0.2, rotation: 0 } },
   ];
   fixture.data.sequence_name = 'Picture and sound';
   const seed = join(output, 'nested-sequence.oqvideo.json');
   await writeFile(seed, JSON.stringify(fixture));
   await launch(seed); await waitState('assets 3; clips 3');
   await click('SEQUENCE'); await click('New sequence'); await waitState('clips 0');
   await click('Nest at playhead'); await waitState('clips 1');
   await expectColor('blue');
   for (let i = 0; i < 3; i++) await click('NEXT');
   await expectColor('red');
   await click('NEXT'); await expectColor('blue');
   await click('PLAY'); await waitState('playhead 17 frames'); await expectColor('red');
   const path = join(output, 'nested-sequence.mp4');
   await fileAction('EXPORT', path, 'Export MP4'); await waitState('Export complete');
   const stride = 320 * 180 * 3;
   const bytes = mediaCommand(['-i', path, '-an', '-pix_fmt', 'rgb24', '-f', 'rawvideo', '-'], { maxBuffer: stride * 20 });
   assert.equal(bytes.length, 18 * stride);
   for (const [frame, channel] of [[0, 2], [3, 0], [4, 2], [7, 2], [10, -1], [12, 0]]) {
     const at = frame * stride + (90 * 320 + 160) * 3;
     for (let rgb = 0; rgb < 3; rgb++) {
       assert(rgb === channel ? bytes[at + rgb] > 225 : bytes[at + rgb] < 25, `nested frame ${frame} channel ${rgb}`);
     }
   }
   assert(amplitude(pcm(path), 0.1, 880) > 0.02);
   await click('SAVE'); await waitState('Project saved'); await close();
   await launch(seed); await waitState('clips 1'); await expectColor('blue');
   const selectNest = async () => client.click(await item('Picture and sound'));
   const resizeGrip = () => client.waitFor(async () => (await buttons('Resize top left'))[0],
     { description: 'the nested clip transform gripper' });
   await selectNest(); await click('TRANSFORM');
   const topBefore = (await resizeGrip()).absolutePosition.y;
   const chooseChild = async (label, key) => {
     const combo = () => client.find({ label: 'Active sequence', role: 'Combobox' });
     await client.click(await combo()); await client.key(key); await client.key('\n');
     await client.waitFor(async () => (await combo()).accessibleValue === label,
       { description: `active sequence ${label}` });
   };
   await click('SEQUENCE'); await chooseChild('Picture and sound', '\uf700'); await waitState('clips 3');
   await click('Nest at playhead'); await waitState('cannot contain itself');
   await client.setValue(await client.find({ label: 'Sequence width', role: 'TextInput' }), '640');
   await click('Apply settings');
   await waitState('Frame 0');
   await client.click(await item('background.mp4')); await click('DELETE'); await waitState('clips 2');
   await click('SEQUENCE'); await chooseChild('Sequence 1', '\uf701'); await click('Cancel');
   await waitState('clips 1'); await expectColor('black');
   await selectNest();
   assert((await resizeGrip()).absolutePosition.y > topBefore + 20, 'grips follow the resized child canvas');
   await click('SAVE'); await waitState('Project saved');
   await close();
 }, { core: true });
 await step('Create, rename, duplicate and reopen independent sequences sharing one media library', async () => {
   const seed = join(output, 'sequence-switch.oqvideo.json');
   await writeFile(seed, await readFile(exportProject));
   await launch(seed); await waitState('assets 2; clips 3');
   await click('SEQUENCE');
   await click('New sequence'); await waitState('assets 2; clips 0');
   await enter(await findInput('Sequence name'), 'Delivery');
   await client.setValue(await findInput('Sequence width'), '640');
   await client.setValue(await findInput('Sequence height'), '360');
   await click('Apply settings');
   const chooseSequence = async (steps, label) => {
     const box = () => client.find({ label: 'Active sequence', role: 'Combobox' });
     await client.click(await box());
     for (let i = 0; i < Math.abs(steps); i++) await client.key(steps < 0 ? '\uf700' : '\uf701');
     await client.key('\n');
     await client.waitFor(async () => (await box()).accessibleValue === label,
       { description: `active sequence ${label}` });
   };
   await click('SEQUENCE');
   await chooseSequence(-1, 'Sequence 1');
   await waitState('assets 2; clips 3');
   assert.equal((await findInput('Sequence width')).accessibleValue, '320');
   await click('Duplicate sequence');
   assert.equal((await findInput('Sequence name')).accessibleValue, 'Sequence 1 copy');
   await click('Cancel');
   await client.click(await item('foreground.mp4'));
   await click('DELETE'); await waitState('clips 2');
   await click('SEQUENCE');
   await chooseSequence(-2, 'Sequence 1');
   await waitState('assets 2; clips 3');
   await click('Cancel');
   await click('SAVE'); await waitState('Project saved');
   const stored = JSON.parse(await readFile(seed, 'utf8')).data;
   assert.equal(stored.sequence_name, 'Sequence 1');
   assert.equal(stored.other_sequences.find(slot => slot.name === 'Delivery').width, 640);
   assert.equal(stored.other_sequences.find(slot => slot.name === 'Sequence 1 copy').tracks.flatMap(track => track.clips).length, 2);
   await close();
   await launch(seed); await waitState('assets 2; clips 3');
   await click('SEQUENCE');
   await chooseSequence(1, 'Delivery');
   await waitState('assets 2; clips 0');
   assert.equal((await findInput('Sequence width')).accessibleValue, '640');
   await click('Cancel');
   await click('UNDO'); await waitState('assets 2; clips 3');
   await close();
 }, { core: true });
 await step('Animate a shader scalar through keyframes in playback and export', async () => {
   const fixture = JSON.parse(await readFile(exportProject, 'utf8'));
   const [upper] = fixture.data.tracks;
   const [clip] = upper.clips;
   upper.clips = [clip];
   clip.timeline_start = 0; clip.duration = 5; clip.transform.scale = 1;
   fixture.data.tracks = [upper];
   const graph = clip.pipeline.graph;
   const [direct] = graph.edges;
   graph.nodes.push({
     id: 'tint', type: 'shader', position: { x: 220, y: 80 },
     data: {
       type: 'shader', label: 'Animated tint',
       shaderCode: `
@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var imageSampler: sampler;
@group(0) @binding(2) var<uniform> strength: f32;
@fragment fn main(@location(0) uv: vec2f) -> @location(0) vec4f {
  let original = textureSample(image, imageSampler, uv);
  return vec4f(mix(vec3f(0.0, 0.0, 1.0), original.rgb, strength), original.a);
}`,
       inputs: [
         { id: 'image', label: 'image', dataType: 'sampler2D', direction: 'input' },
         { id: 'strength', label: 'strength', dataType: 'float', direction: 'input' },
       ],
       outputs: [{ id: 'fragColor', label: 'fragColor', dataType: 'vec4', direction: 'output' }],
       uniforms: { strength: 1 },
     },
   });
   graph.edges = [
     { ...direct, id: 'tint-source', target: 'tint', targetHandle: 'image' },
     { ...direct, id: 'tint-output', source: 'tint', sourceHandle: 'fragColor' },
   ];
   const seed = join(output, 'effect-animation.oqvideo.json');
   await writeFile(seed, JSON.stringify(fixture));
   await launch(seed); await waitState('clips 1');
   await client.click(await item('foreground.mp4')); await expectColor('red');
   await click('Show keyframes');
   const channel = await client.find({ label: 'Animation property', role: 'Combobox' });
   await client.click(channel);
   for (let i = 0; i < 3; i++) await client.key('\uf701');
   await client.key('\n');
   await click('Add effect keyframe');
   for (let i = 0; i < 4; i++) await click('NEXT');
   await enter(await client.find({ label: 'Effect keyframe value', role: 'TextInput' }), 0);
   await expectColor('blue');
   for (let i = 0; i < 4; i++) await click('PREVIOUS');
   await expectColor('red');
   await click('PLAY'); await waitState('playhead 4 frames'); await expectColor('blue');
   const path = join(output, 'effect-animated.mp4');
   await fileAction('EXPORT', path, 'Export MP4'); await waitState('Export complete');
   const pixels = mediaCommand(['-i', path, '-map', '0:v:0', '-vsync', '0', '-vf', 'scale=32:18',
     '-pix_fmt', 'rgb24', '-f', 'rawvideo', '-']);
   const center = frame => pixels.subarray((frame * 32 * 18 + 9 * 32 + 16) * 3,
     (frame * 32 * 18 + 9 * 32 + 16) * 3 + 3);
   assert(center(0)[0] > 200 && center(0)[2] < 25);
   assert(center(4)[2] > 200 && center(4)[0] < 25);
   assert(Math.abs(center(2)[0] - center(2)[2]) < 24 && center(2)[0] > 90);
   await click('SAVE'); await waitState('Project saved');
   const saved = JSON.parse(await readFile(seed, 'utf8')).data.tracks[0].clips[0];
   assert.deepEqual(saved.effect_keys, [{ node: 'tint', parameter: 'strength',
     keys: [{ time: 0, value: 1 }, { time: 4, value: 0 }] }]);
   await click('Remove effect keyframe'); await expectColor('red');
   await click('UNDO'); await expectColor('blue');
   await close();
 }, { core: true });
 await step('Migrate legacy animations and keyframe crop in the monitor and export', async () => {
   const fixture = JSON.parse(await readFile(exportProject, 'utf8'));
   fixture.schema_version = 15; fixture.data.schema_version = 15;
   const [upper, , lower] = fixture.data.tracks;
   upper.clips = upper.clips.slice(0, 1);
   upper.clips[0].timeline_start = 0;
   upper.clips[0].duration = 5;
   upper.clips[0].transform.scale = 1;
   upper.clips[0].spatial_keys = [
     { offset: 0, pose: { x: 0, y: 0, scale: 1, rotation: 0 }, ease: true },
     { offset: 4, pose: { x: 0, y: 0, scale: 1, rotation: 0 } },
   ];
   upper.clips[0].opacity_keys = [{ source_ns: 0, opacity: 1 }];
   lower.clips[0].duration = 5;
   fixture.data.tracks = [upper, lower];
   const seed = join(output, 'animated-crop.oqvideo.json');
   await writeFile(seed, JSON.stringify(fixture));
   await launch(seed); await waitState('clips 2');
   await client.click(await item('foreground.mp4')); await expectColor('red');
   const cropKey = action => clipControl(`${action} crop keyframe`, 'Button');
   await client.click(await cropKey('Add'));
   for (let i = 0; i < 4; i++) await click('NEXT');
   await enter(await clipControl('Crop left (%)'), 90);
   await expectColor('blue');
   await click('PREVIOUS'); await waitState('playhead 3 frames'); await expectColor('blue');
   await click('PREVIOUS'); await waitState('playhead 2 frames'); await expectColor('red');
   assert.equal((await clipControl('Crop left (%)')).accessibleValue, '45.000');
   const path = join(output, 'animated-crop.mp4');
   await fileAction('EXPORT', path, 'Export MP4'); await waitState('Export complete');
   const pixels = mediaCommand(['-i', path, '-map', '0:v:0', '-vsync', '0', '-vf', 'scale=32:18',
     '-pix_fmt', 'rgb24', '-f', 'rawvideo', '-']);
   const dominant = frame => {
     const offset = frame * 32 * 18 * 3 + (9 * 32 + 16) * 3;
     return pixels[offset] > pixels[offset + 2] ? 'red' : 'blue';
   };
   assert.deepEqual([0, 1, 2, 3, 4].map(dominant), ['red', 'red', 'red', 'blue', 'blue']);
   await click('SAVE'); await waitState('Project saved');
   const saved = JSON.parse(await readFile(seed, 'utf8'));
   assert.equal(saved.schema_version, 27);
   const [layer] = saved.data.tracks[0].clips;
   assert.deepEqual(layer.opacity_keys, [{ time: 0, value: 1 }]);
   assert.equal(layer.spatial_keys[0].curve, 'smooth');
   assert.deepEqual(layer.crop_keys.map(key => key.time), [0, 4]);
   await close();
   await launch(seed); await waitState('clips 2');
   await client.click(await item('foreground.mp4'));
   for (let i = 0; i < 4; i++) await click('NEXT');
   await client.click(await cropKey('Remove'));
   await expectColor('red');
   await click('UNDO'); await expectColor('blue');
   await close();
 }, { core: true });
 await step('Crop the layer and match eased transform keyframes in preview and exported pixels', async () => {
   const fixture = JSON.parse(await readFile(exportProject, 'utf8'));
   const [upper, , lower] = fixture.data.tracks;
   upper.clips = [upper.clips[0]];
   Object.assign(upper.clips[0], { timeline_start: 0, duration: 12 });
   Object.assign(lower.clips[0], { timeline_start: 0, duration: 12 });
   fixture.data.tracks = [upper, lower];
   const seed = join(output, 'spatial-seed.oqvideo.json');
   await writeFile(seed, JSON.stringify(fixture));
   await launch(seed); await waitState('clips 2');
   await client.click(await item('foreground.mp4'));
   for (const [label, percent] of [['Crop left (%)', 50], ['Crop top (%)', 20], ['Crop right (%)', 10], ['Crop bottom (%)', 20]]) {
     await transformControl(label, percent);
   }
   await spatialButton('Add transform keyframe');
   await spatialButton('Ease to next transform key');
   const [next] = await buttons('NEXT');
   for (let i = 0; i < 8; i++) await client.click(next);
   await waitState('playhead 8 frames');
   await transformControl('Position X (px)', 128);
   assert.equal((await clipControl('Position X (px)')).accessibleValue, '128.000');
   await expectColor('blue');
   for (let i = 0; i < 6; i++) await click('PREVIOUS');
   await waitState('playhead 2 frames');
   assert.equal((await clipControl('Position X (px)')).accessibleValue, '20.000');
   await snapshot('22-eased-crop');
   await fileAction('SAVE AS', spatialProject, 'Save project'); await waitState('Project saved');
   const clip = JSON.parse(await readFile(spatialProject, 'utf8')).data.tracks[0].clips[0];
   assert.deepEqual(clip.crop, { left: 0.5, top: 0.2, right: 0.1, bottom: 0.2 });
   assert.deepEqual(clip.spatial_keys.map(key => [key.time, key.value.x, key.curve === "smooth"]),
     [[0, 0, true], [8, 0.4, false]]);
   const path = join(output, 'eased-crop.mp4');
   await fileAction('EXPORT', path, 'Export MP4'); await waitState('Export complete');
   const pixels = mediaCommand(['-i', path, '-map', '0:v:0', '-vsync', '0', '-pix_fmt', 'rgb24', '-f', 'rawvideo', '-'], { maxBuffer: 4 * 1024 * 1024 });
   const width = 320, stride = width * 180 * 3;
   assert.equal(pixels.length, stride * 12);
   const color = (frame, x, y) => {
     const start = frame * stride + (y * width + x) * 3;
     const [r, g, b] = pixels.subarray(start, start + 3);
     assert(g < 25 && Math.max(r, b) > 220, `unexpected sample RGB ${r}/${g}/${b}`);
     return r > b ? 'red' : 'blue';
   };
   assert.equal(color(0, 130, 90), 'blue');
   assert.equal(color(0, 200, 90), 'red');
   assert.equal(color(0, 231, 90), 'blue');
   assert.equal(color(0, 200, 55), 'blue');
   assert.equal(color(0, 200, 124), 'blue');
   assert.equal(color(2, 186, 90), 'red', 'ease moved the cropped left edge 20 pixels, before the linear position');
   assert.equal(color(4, 220, 90), 'blue');
   assert.equal(color(4, 234, 90), 'red');
   assert.equal(color(8, 291, 90), 'red');
   assert.equal(color(11, 291, 90), 'red');
   await close();
 }, { core: true });
 await step('Reopen and split an animated clip, then remove and undo a keyframe', async () => {
   await launch(spatialProject); await waitState('clips 2');
   await client.click(await item('foreground.mp4'));
   for (let i = 0; i < 4; i++) await click('NEXT');
   await waitState('playhead 4 frames');
   assert.equal((await clipControl('Position X (px)')).accessibleValue, '64.000');
   await click('SPLIT'); await waitState('clips 3');
   assert.equal((await clipControl('Position X (px)')).accessibleValue, '64.000');
   await click('UNDO'); await waitState('clips 2');
   await client.click(await item('foreground.mp4'));
   const start = Number(/playhead (\d+) frames/.exec(await state())[1]);
   for (let frame = start; frame < 8; frame++) await click('NEXT');
   await waitState('playhead 8 frames');
   await spatialButton('Remove transform keyframe');
   assert.equal((await clipControl('Position X (px)')).accessibleValue, '0.000');
   await click('UNDO');
   assert.equal((await clipControl('Position X (px)')).accessibleValue, '128.000');
   for (let i = 0; i < 8; i++) await click('PREVIOUS');
   await waitState('playhead 0 frames');
   await spatialButton('Use linear transform interpolation');
   await click('NEXT'); await click('NEXT'); await waitState('playhead 2 frames');
   assert.equal((await clipControl('Position X (px)')).accessibleValue, '32.000');
   await click('UNDO');
   assert.equal((await clipControl('Position X (px)')).accessibleValue, '20.000');
   await close();
 });
 const timelineToolsProject = join(output, 'timeline-tools.oqvideo.json');
 const shiftSelect = async name => client.click(await item(name), { modifiers: { shift: true } });
 const snapProject = join(output, 'timeline-snap.oqvideo.json');
 const dragTimelineBy = async (element, frames) => {
   await client.drag(element, { x: center(element).x + 4 * frames, y: center(element).y });
 };
 const zoomForSnapping = async () => {
   for (let i = 0; i < 6; i++) await click('ZOOM OUT');
 };
 await step('Snap a timeline move and both trim edges, and toggle snapping off', async () => {
   const fixture = JSON.parse(await readFile(exportProject, 'utf8'));
   const [upper, , lower] = fixture.data.tracks;
   Object.assign(upper.clips[0], { timeline_start: 12, duration: 20 });
   Object.assign(upper.clips[1], { timeline_start: 90, duration: 16 });
   Object.assign(lower.clips[0], { timeline_start: 8, duration: 20 });
   fixture.data.tracks = [upper, lower];
   fixture.data.markers = [
     { id: 'snap-move', frame: 60, name: 'Movement reference' },
     { id: 'snap-trim', frame: 72, name: 'Trim reference' },
   ];
   await writeFile(snapProject, JSON.stringify(fixture));
   await launch(snapProject); await waitState('clips 3');
   await zoomForSnapping();
   const snap = () => client.find({ label: 'Toggle timeline snapping', role: 'Button' });
   assert.equal((await snap()).accessibleDescription, 'Snapping enabled');
   await dragTimelineBy(await item('foreground.mp4'), 47);
   assert.deepEqual(await positions('foreground.mp4'), [[60, 20], [90, 16]]);
   assert.equal((await client.byLabel('Snap guide')).length, 0);
   await click('UNDO');
   assert.deepEqual(await positions('foreground.mp4'), [[12, 20], [90, 16]]);
   await client.click(await snap());
   assert.equal((await snap()).accessibleDescription, 'Snapping disabled');
   await dragTimelineBy(await item('foreground.mp4'), 47);
   assert.deepEqual(await positions('foreground.mp4'), [[59, 20], [90, 16]]);
   await click('UNDO');
   await client.click(await snap());
   assert.equal((await snap()).accessibleDescription, 'Snapping enabled');
   await dragTimelineBy(await item('foreground.mp4'), 47);
   const left = (await buttons('Trim start of foreground.mp4'))[0];
   await dragTimelineBy(left, 11);
   assert.deepEqual(await positions('foreground.mp4'), [[72, 8], [90, 16]]);
   const right = (await buttons('Trim end of foreground.mp4'))[0];
   await dragTimelineBy(right, 9);
   assert.deepEqual(await positions('foreground.mp4'), [[72, 18], [90, 16]]);
   await click('UNDO'); await click('UNDO'); await click('UNDO');
   assert.deepEqual(await positions('foreground.mp4'), [[12, 20], [90, 16]]);
   await close();
 }, { core: true });
 await step('A selected clip on another track can anchor a snapped group move', async () => {
   await launch(snapProject); await waitState('clips 3');
   await zoomForSnapping();
   await client.click(await item('foreground.mp4'));
   await shiftSelect('background.mp4'); await waitState('2 selected');
   await dragTimelineBy(await item('foreground.mp4'), 51);
   assert.deepEqual(await positions('foreground.mp4'), [[64, 20], [90, 16]]);
   assert.deepEqual(await positions('background.mp4'), [[60, 20]]);
   await click('UNDO');
   assert.deepEqual(await positions('foreground.mp4'), [[12, 20], [90, 16]]);
   assert.deepEqual(await positions('background.mp4'), [[8, 20]]);
   await close();
 });
 await step('Move and delete a multi-track selection in one undo step, and navigate a named marker', async () => {
   const fixture = JSON.parse(await readFile(exportProject, 'utf8'));
   const [upper, , lower] = fixture.data.tracks;
   Object.assign(upper.clips[0], { timeline_start: 7, duration: 6 });
   Object.assign(upper.clips[1], { timeline_start: 16, duration: 6 });
   Object.assign(lower.clips[0], { timeline_start: 4, duration: 6 });
   fixture.data.tracks = [upper, lower];
   await writeFile(timelineToolsProject, JSON.stringify(fixture));
   await launch(timelineToolsProject); await waitState('clips 3');
   await client.click(await item('foreground.mp4')); await waitState('playhead 7 frames');
   await shiftSelect('background.mp4'); await waitState('2 selected');
   const first = await item('foreground.mp4');
   const px = first.size.width / 6;
   await client.drag(first, { x: center(first).x + 2 * px, y: center(first).y });
   assert.deepEqual(await positions('foreground.mp4'), [[9, 6], [16, 6]]);
   assert.deepEqual(await positions('background.mp4'), [[6, 6]]);
   await click('UNDO');
   assert.deepEqual(await positions('foreground.mp4'), [[7, 6], [16, 6]]);
   assert.deepEqual(await positions('background.mp4'), [[4, 6]]);
   await click('DELETE'); await waitState('clips 1');
   assert.deepEqual(await positions('foreground.mp4'), [[16, 6]]);
   assert.deepEqual(await positions('background.mp4'), []);
   await click('UNDO'); await waitState('clips 3');

   await client.click(await item('foreground.mp4')); await waitState('playhead 7 frames');
   await click('Add marker');
   await enter(await findInput('Marker name'), 'Cutaway');
   await client.waitFor(async () => (await buttons('Go to Cutaway')).length === 1,
     { description: 'ruler shows the renamed marker' });
   await click('PREVIOUS'); await waitState('playhead 6 frames');
   await click('Go to Cutaway'); await waitState('playhead 7 frames');
   await fileAction('SAVE AS', join(output, 'timeline-marker.oqvideo.json'), 'Save project');
   await waitState('Project saved');
   await close();
 }, { core: true });
 await step('Reject a conflicting group move and reopen and remove a saved timeline marker', async () => {
   await launch(join(output, 'timeline-marker.oqvideo.json')); await waitState('clips 3');
   await click('Go to Cutaway'); await waitState('playhead 7 frames');
   assert.equal((await findInput('Marker name')).accessibleValue, 'Cutaway');
   await click('Remove marker');
   assert.equal((await buttons('Go to Cutaway')).length, 0);
   await click('UNDO'); assert.equal((await buttons('Go to Cutaway')).length, 1);
   await client.click(await item('foreground.mp4')); await waitState('playhead 7 frames');
   await shiftSelect('background.mp4'); await waitState('2 selected');
   const first = await item('foreground.mp4');
   await client.drag(first, { x: center(first).x + first.size.width, y: center(first).y });
   await waitState('Video track clips cannot overlap');
   assert.deepEqual(await positions('foreground.mp4'), [[7, 6], [16, 6]]);
   assert.deepEqual(await positions('background.mp4'), [[4, 6]]);
   await close();
 });
 await step('Slip changes source samples and Slide shifts both neighboring cuts', async () => {
   const fixture = JSON.parse(await readFile(exportProject, 'utf8'));
   const track = fixture.data.tracks.find(track => track.clips.some(clip =>
     fixture.data.assets.find(asset => asset.id === clip.asset_id)?.name === 'background.mp4'));
   const reference = track.clips[0];
   const asset = fixture.data.assets.find(asset => asset.id === reference.asset_id);
   const second = asset.time_base.denominator / asset.time_base.numerator;
   track.clips = [0, 1, 2].map(index => ({
     ...reference, id: `slip-${index}`, timeline_start: index * 10, duration: 10,
     source_in_pts: asset.start_pts + index * second,
   }));
   fixture.data.tracks = [track];
   const seed = join(output, 'slip-slide.oqvideo.json');
   await writeFile(seed, JSON.stringify(fixture));
   await launch(seed); await waitState('clips 3');
   const middle = () => client.waitFor(async () => (await timeline('background.mp4'))[1],
     { description: 'middle source clip' });
   await client.click(await middle());
   await click('SLIP');
   let clip = await middle();
   const px = clip.size.width / 10;
   await client.drag(clip, { x: center(clip).x + 2 * px, y: center(clip).y });
   assert.deepEqual(await positions('background.mp4'), [[0, 10], [10, 10], [20, 10]]);
   assert.equal(Number((await clipControl('Source in')).accessibleValue), asset.start_pts + second + second * 2 / 30);
   await click('SLIDE');
   clip = await middle();
   await client.drag(clip, { x: center(clip).x + 2 * px, y: center(clip).y });
   assert.deepEqual(await positions('background.mp4'), [[0, 12], [12, 10], [22, 8]]);
   assert.equal(Number((await clipControl('Source in')).accessibleValue), asset.start_pts + second + second * 2 / 30);
   await click('UNDO');
   assert.deepEqual(await positions('background.mp4'), [[0, 10], [10, 10], [20, 10]]);
   await click('UNDO');
   assert.equal(Number((await clipControl('Source in')).accessibleValue), asset.start_pts + second);
   await close();
 }, { core: true });
 const recoveryFiles = async () => (await readdir(join(output, 'recovery')))
   .filter(name => name.endsWith('.recovery.oqvideo.json'));
 const abruptExit = async () => {
   child.kill('SIGKILL');
   assert.deepEqual(await exited, { code: null, signal: 'SIGKILL' });
   child = undefined;
 };
 await step('Recover an unsaved edit after killing the native editor', async () => {
   await launch(); await waitState('clips 0');
   await click('Add title'); await waitState('clips 1');
   await waitState('Recovery snapshot saved');
   assert.equal((await recoveryFiles()).length, 1);
   await abruptExit();

   await launch(); await waitState('clips 0');
   await click('Recover autosave');
   await waitState('Recovered an editable copy');
   assert((await state()).includes('clips 1'));
   assert.deepEqual(await positions('Title'), [[0, 90]]);
   const saved = join(output, 'recovered.oqvideo.json');
   await fileAction('SAVE AS', saved, 'Save project'); await waitState('Project saved');
   assert.deepEqual(await recoveryFiles(), []);
   await close();

   await launch(saved); await waitState('clips 1');
   assert.deepEqual(await positions('Title'), [[0, 90]]);
   await close();
 }, { core: true });
 await step('Open the previous saved version as a copy and discard a separate autosave', async () => {
   const path = join(output, 'recover-history.oqvideo.json');
   await launch(); await waitState('clips 0');
   await click('Add title'); await waitState('clips 1');
   await fileAction('SAVE AS', path, 'Save project'); await waitState('Project saved');
   await click('Add title'); await waitState('clips 2');
   await click('SAVE'); await waitState('Project saved');
   await click('Previous version'); await waitState('Recovered an editable copy');
   assert((await state()).includes('clips 1'));
   const saved = JSON.parse(await readFile(path, 'utf8'));
   assert.equal(saved.data.tracks.flatMap(track => track.clips).length, 2);
   await waitState('Recovery snapshot saved');
   await abruptExit();

   await launch(path); await waitState('clips 2');
   await click('Discard autosave'); await waitState('Recovery snapshot discarded');
   assert.deepEqual(await recoveryFiles(), []);
   assert.equal((await buttons('Recover autosave')).length, 0);
   assert((await state()).includes('clips 2'));
   await close();
 });
 await step('Track names, button reordering, deletion and explicit show update the editor', async () => {
   await launch(multiProject); await waitState('clips 2');
   await enter(await findInput('Name of Video 2'), 'Spare');
   await client.waitFor(async () => (await client.byLabel('Reorder Spare', { role: 'Button' })).length === 1,
     { description: 'renamed track head' });
   await click('Move Spare up');
   assert(center(await grip('Spare')).y < center(await grip('Video 3')).y);
   await click('Move Spare down');
   assert(center(await grip('Spare')).y > center(await grip('Video 3')).y);
   await client.click(await grip('Spare'));
   await client.key('\u0012', 'Press'); // Alt / Option
   await client.key('\u0010', 'Press'); // Shift
   try {
     await client.key('\uf700'); // Up
     assert(center(await grip('Spare')).y < center(await grip('Video 3')).y);
     await client.key('\uf701'); // Down
     assert(center(await grip('Spare')).y > center(await grip('Video 3')).y);
   } finally {
     await client.key('\u0010', 'Release');
     await client.key('\u0012', 'Release');
   }
   await click('Delete empty Video 3'); await waitState('remove clips before deleting');
   await click('Delete empty Spare');
   await client.waitFor(async () => (await client.byLabel('Reorder Spare', { role: 'Button' })).length === 0,
     { description: 'empty track removed' });
   await click('Hide Video 3'); await expectColor('blue');
   await click('Show Video 3'); await expectColor('red');
   await click('SAVE'); await waitState('Project saved');
   const { data } = JSON.parse(await readFile(multiProject, 'utf8'));
   assert.deepEqual(data.tracks.map(track => track.name), ['Video 3', 'Video 1']);
   await close();
 });
 await step('Monitor picking, rotation, transform properties, and reset reach actual output', async () => {
   await launch(multiProject); await waitState('clips 2'); await expectColor('red');
   const monitor = await client.find({ label: 'Video preview', role: 'Image' });
   await client.click(monitor);
   await client.waitFor(async () => (await client.byLabel('Timeline start', { role: 'TextInput' }))[0]?.accessibleValue === '0',
     { description: 'monitor picks the active foreground clip' });
   await click('TRANSFORM');
   const rotation = await client.find({ label: 'Rotate clip', role: 'Button' });
   const start = center(rotation);
   await client.drag(rotation, { x: start.x + 60, y: start.y + 9 });
   assert(Math.abs(Number((await clipControl('Rotation (°)')).accessibleValue)) > 5,
     'Rotating via the monitor updates the Inspector');
   await client.click(await clipControl('Reset', 'Button'));
   assert.equal(Number((await clipControl('Rotation (°)')).accessibleValue), 0);
   assert.equal(Number((await clipControl('Scale')).accessibleValue), 1);
   await enter(await clipControl('Opacity (%)'), 50);
   await expectColor('blend');
   for (const [label, value] of [['Position X (px)', 30], ['Position Y (px)', -20], ['Scale', 0.9], ['Rotation (°)', 12], ['Exposure (EV)', -0.5]]) {
     await enter(await clipControl(label), value);
     assert(Math.abs(Number((await clipControl(label)).accessibleValue) - value) < 0.01, `${label} committed`);
   }
   await click('SAVE'); await waitState('Project saved');
   const { data } = JSON.parse(await readFile(multiProject, 'utf8'));
   const transform = data.tracks[0].clips[0].transform;
   for (const [key, value] of Object.entries({ x: 30/1920, y: -20/1080, scale: 0.9, rotation: 12, opacity: 0.5, exposure: -0.5 })) {
     assert(Math.abs(transform[key] - value) < 1e-6, `${key} persisted on the picked foreground`);
   }
   await click('Lock Video 3');
   assert.equal(Boolean((await clipControl('Opacity (%)')).accessibleEnabled), false);
   assert.equal((await client.byLabel('Rotate clip', { role: 'Button' })).length, 0);
   await click('Unlock Video 3');
   assert.equal(Boolean((await clipControl('Opacity (%)')).accessibleEnabled), true);
   await click('SAVE'); await waitState('Project saved');
   await close();
 }, { core: true });
 await step('Project rename, library search, ruler seek and Inspector timing are observable', async () => {
   await launch(multiProject); await waitState('clips 2');
   await enter(await findInput('Project name'), 'Multitrack interaction coverage');
   await waitState('Project Multitrack interaction coverage');
   const search = await findInput('Search media');
   await client.click(search); await client.setValue(search, 'foreground');
   await client.waitFor(async () => (await client.byLabel('Add background.mp4 to timeline', { role: 'Button' })).length === 0,
     { description: 'nonmatching library asset is filtered' });
   assert.equal((await client.byLabel('Add foreground.mp4 to timeline', { role: 'Button' })).length, 1);
   await client.setValue(search, '');
   await client.waitFor(async () => (await client.byLabel('Add background.mp4 to timeline', { role: 'Button' })).length === 1,
     { description: 'clearing search restores library assets' });
   await click('FIT TIMELINE');
   await click('Seek timeline ruler');
   await client.waitFor(async () => /playhead [1-9]\d* frames/.test(await state()), { description: 'ruler updates the playhead' });
   const soughtFrame = Number(/playhead (\d+) frames/.exec(await state())[1]);
   await click('PREVIOUS');
   await waitState(`playhead ${soughtFrame - 1} frames`);
   await client.click(await item('foreground.mp4'));
   await enter(await clipControl('Timeline end'), 100);
   assert((await item('foreground.mp4')).accessibleDescription.includes('duration 100 frames'));
   await enter(await clipControl('Timeline start'), 5);
   assert((await item('foreground.mp4')).accessibleDescription.includes('start 5 frames'));
   const { data } = JSON.parse(await readFile(multiProject, 'utf8'));
   const clip = data.tracks[0].clips[0], asset = data.assets.find(asset => asset.id === clip.asset_id);
   const ticksPerSecond = asset.time_base.denominator / asset.time_base.numerator;
   await enter(await clipControl('Source in'), asset.start_pts + ticksPerSecond / 2);
   await enter(await clipControl('Source out'), asset.start_pts + ticksPerSecond * 2);
   assert((await item('foreground.mp4')).accessibleDescription.includes('duration 45 frames'));
   await click('SAVE'); await waitState('Project saved');
   const saved = JSON.parse(await readFile(multiProject, 'utf8')).data;
   assert.equal(saved.name, 'Multitrack interaction coverage');
   assert.equal(saved.tracks[0].clips[0].source_in_pts, asset.start_pts + ticksPerSecond / 2);
   await close();
 }, { core: true });
 await hdrPreviewRoundTrip();
 await step('Export PQ highlights as independently decoded HDR10 Main10 with mastering metadata', async () => {
   const input = join(output, 'pq-split.mp4');
   mediaCommand(['-f', 'lavfi', '-i',
     "nullsrc=s=320x180:r=30,format=yuv420p10le,geq=lum='if(lt(X,160),509,720)':cb=512:cr=512",
     '-frames:v', '24', '-c:v', 'libx265', '-x265-params', 'pools=1:frame-threads=1:log-level=error',
     '-pix_fmt', 'yuv420p10le', '-tag:v', 'hvc1', '-color_range', 'tv',
     '-colorspace', 'bt2020nc', '-color_primaries', 'bt2020', '-color_trc', 'smpte2084', '-y', input]);
   await launch(); await waitState('assets 0');
   await click('SEQUENCE');
   await client.setValue(await client.find({ label: 'Sequence width', role: 'TextInput' }), '320');
   await client.setValue(await client.find({ label: 'Sequence height', role: 'TextInput' }), '180');
   await click('Apply settings');
   await importMedia(input); await waitState('assets 1');
   await click('ADD CLIP'); await waitState('clips 1');
   const path = join(output, 'main10.mp4');
   await click('EXPORT');
   await client.click(await client.find({ label: 'Export color profile', role: 'Combobox' }));
   await client.key('\uf701'); await client.key('\n');
   await client.waitFor(async () =>
     (await client.find({ label: 'Export color profile', role: 'Combobox' })).accessibleValue.startsWith('HDR10'),
     { description: 'HDR10 export profile selected' });
   await client.setValue(await client.find({ label: 'Export end frame (exclusive)', role: 'TextInput' }), '5');
   await client.setValue(await client.find({ label: 'File path', role: 'TextInput' }), path);
   await click('Export MP4'); await waitState('Export complete');
   const probe = spawnSync(ffmpeg, ['-hide_banner', '-i', path, '-vf', 'showinfo', '-f', 'null', '-'], { env: mediaEnv, encoding: 'utf8' });
   assert.equal(probe.status, 0, probe.stderr);
   assert.match(probe.stderr, /hevc \(Main 10\).*yuv420p10le.*bt2020nc\/bt2020\/smpte2084/);
   assert.match(probe.stderr, /n:\s*0\s+pts:\s*0\s+pts_time:0/);
   const decoded = mediaCommand(['-i', path, '-an', '-pix_fmt', 'yuv420p10le', '-f', 'rawvideo', '-']);
   assert.equal(decoded.length, 320 * 180 * 3 * 5);
   const dark = decoded.readUInt16LE((90 * 320 + 100) * 2);
   const light = decoded.readUInt16LE((90 * 320 + 220) * 2);
   assert(Math.abs(dark - 509) <= 5 && Math.abs(light - 720) <= 5, `expected original PQ values, got ${dark} and ${light}`);
   const container = await readFile(path);
   const mdcv = container.indexOf(Buffer.from('mdcv'));
   assert(mdcv > 4 && container.readUInt32BE(mdcv - 4) === 32, 'explicit MP4 mastering display declaration');
   assert.equal(container.readUInt32BE(mdcv + 20), 10_000_000, '1000 nit target in mastering metadata');
   await fileAction('SAVE AS', join(output, 'hdr-export.oqvideo.json'), 'Save project');
   await waitState('Project saved'); await close();
 }, { core: true });
 await step('Composite two overlapping video tracks into a linear-light HDR10 export', async () => {
   const fixture = JSON.parse(await readFile(join(output, 'hdr-export.oqvideo.json'), 'utf8'));
   const base = fixture.data.tracks[0];
   const top = structuredClone(base);
   top.id += '-top'; top.name = 'Upper grade'; top.clips[0].id += '-top';
   top.clips[0].transform.opacity = 0.5;
   base.clips[0].transform.exposure = -1;
   fixture.data.tracks.unshift(top);
   const project = join(output, 'hdr-stack.oqvideo.json');
   await writeFile(project, JSON.stringify(fixture));
   await launch(project); await waitState('clips 2');
   const path = join(output, 'hdr-stack.mp4');
   await click('EXPORT');
   await client.click(await client.find({ label: 'Export color profile', role: 'Combobox' }));
   await client.key('\uf701'); await client.key('\n');
   await client.setValue(await client.find({ label: 'Export end frame (exclusive)', role: 'TextInput' }), '3');
   await client.setValue(await client.find({ label: 'File path', role: 'TextInput' }), path);
   await click('Export MP4'); await waitState('Export complete');
   const decoded = mediaCommand(['-i', path, '-an', '-pix_fmt', 'yuv420p10le', '-f', 'rawvideo', '-']);
   assert.equal(decoded.length, 320 * 180 * 3 * 3);
   const pq = (code, gain) => {
     const m1 = 2610 / 16384, m2 = 2523 / 32;
     const c1 = 3424 / 4096, c2 = 2413 / 128, c3 = 2392 / 128;
     const signal = ((code - 64) / 876) ** (1 / m2);
     const luminance = (Math.max(signal - c1, 0) / (c2 - c3 * signal)) ** (1 / m1) * gain;
     const power = luminance ** m1;
     return 64 + 876 * ((c1 + c2 * power) / (1 + c3 * power)) ** m2;
   };
   for (const [x, original] of [[100, 509], [220, 720]]) {
     const actual = decoded.readUInt16LE((90 * 320 + x) * 2);
     assert(Math.abs(actual - pq(original, 0.75)) <= 5,
       `expected linear-light 75% HDR blend at ${x}, got ${actual}`);
   }
   await close();
 }, { core: true });
 await step('Deliver BT.2020 HLG Main10 with independently verified signal levels', async () => {
   const input = join(output, 'hlg-split.mp4');
   mediaCommand(['-f', 'lavfi', '-i',
     "nullsrc=s=320x180:r=30,format=yuv420p10le,geq=lum='if(lt(X,160),509,720)':cb=512:cr=512",
     '-frames:v', '12', '-c:v', 'libx265', '-x265-params', 'pools=1:frame-threads=1:log-level=error',
     '-pix_fmt', 'yuv420p10le', '-tag:v', 'hvc1', '-color_range', 'tv',
     '-colorspace', 'bt2020nc', '-color_primaries', 'bt2020', '-color_trc', 'arib-std-b67', '-y', input]);
   await launch();
   await click('SEQUENCE');
   await client.setValue(await client.find({ label: 'Sequence width', role: 'TextInput' }), '320');
   await client.setValue(await client.find({ label: 'Sequence height', role: 'TextInput' }), '180');
   await click('Apply settings');
   await importMedia(input); await waitState('assets 1');
   await click('ADD CLIP'); await waitState('clips 1');
   const path = join(output, 'hlg-out.mp4');
   await click('EXPORT');
   const profile = () => client.find({ label: 'Export color profile', role: 'Combobox' });
   await client.click(await profile());
   await client.key('\uf701'); await client.key('\uf701'); await client.key('\n');
   assert.equal((await profile()).accessibleValue, 'HLG · BT.2020');
   await client.setValue(await client.find({ label: 'Export end frame (exclusive)', role: 'TextInput' }), '4');
   await client.setValue(await client.find({ label: 'File path', role: 'TextInput' }), path);
   await click('Export MP4'); await waitState('Export complete');
   const probe = spawnSync(ffmpeg, ['-hide_banner', '-i', path, '-vf', 'showinfo', '-f', 'null', '-'], { env: mediaEnv, encoding: 'utf8' });
   assert.equal(probe.status, 0, probe.stderr);
   assert.match(probe.stderr, /hevc \(Main 10\).*yuv420p10le.*bt2020nc\/bt2020\/arib-std-b67/);
   assert.match(probe.stderr, /n:\s*0\s+pts:\s*0\s+pts_time:0/);
   const decoded = mediaCommand(['-i', path, '-an', '-pix_fmt', 'yuv420p10le', '-f', 'rawvideo', '-']);
   assert.equal(decoded.length, 320 * 180 * 3 * 4);
   for (const [x, expected] of [[100, 509], [220, 720]]) {
     const actual = decoded.readUInt16LE((90 * 320 + x) * 2);
     assert(Math.abs(actual - expected) <= 5, `expected HLG ${expected} at ${x}, got ${actual}`);
   }
   assert.equal((await readFile(path)).indexOf(Buffer.from('mdcv')), -1);
   await fileAction('SAVE AS', join(output, 'hlg-export.oqvideo.json'), 'Save project');
   await waitState('Project saved'); await close();
 }, { core: true });
 await step('Switch a sequence to managed color and apply exposure before the SDR view', async () => {
   const fixture = JSON.parse(await readFile(join(output, 'hdr-export.oqvideo.json'), 'utf8'));
   fixture.data.tracks[0].clips[0].transform.exposure = -1;
   const project = join(output, 'managed-view.oqvideo.json');
   await writeFile(project, JSON.stringify(fixture));
   await launch(project); await waitState('clips 1');
   const check = (min, max) => client.waitFor(async () => {
     const pixels = decodePng(await client.screenshot());
     const viewer = await client.find({ label: 'Video preview', role: 'Image' });
     const win = await client.windowProperties();
     const scale = pixels.width / (win.size.width / (win.scaleFactor || 1));
     const x = Math.floor((viewer.absolutePosition.x + viewer.size.width * 0.43) * scale);
     const y = Math.floor((viewer.absolutePosition.y + viewer.size.height / 2) * scale);
     const level = pixels.pixels[(y * pixels.width + x) * pixels.channels];
     return level >= min && level <= max;
   }, { description: `monitor exposure level ${min}–${max}` });
   await check(68, 81);
   await click('SEQUENCE');
   const choice = () => client.find({ label: 'Managed color · ACEScg', role: 'Checkbox' });
   await client.click(await choice());
   await click('Cancel');
   await check(115, 133);
   await click('UNDO'); await check(68, 81);
   await click('REDO'); await check(115, 133);
   await click('SAVE'); await waitState('Project saved'); await close();
   assert.equal(JSON.parse(await readFile(project, 'utf8')).data.managed_color, true);
   await launch(project); await waitState('clips 1'); await check(115, 133); await close();
 }, { core: true });
 await step('Override Sony Log input, reset to Auto, and reopen the saved interpretation', async () => {
   const path = join(output, 'sony-log-reference.mp4');
   mediaCommand(['-f', 'lavfi', '-i',
     "nullsrc=s=320x180:r=30,format=yuv420p10le,geq=lum=420:cb=512:cr=512",
     '-frames:v', '12', '-c:v', 'libx265', '-x265-params', 'pools=1:frame-threads=1:log-level=error',
     '-pix_fmt', 'yuv420p10le', '-tag:v', 'hvc1', '-color_range', 'tv',
     '-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-y', path]);
   await launch(); await importMedia(path); await waitState('assets 1');
   const choice = () => client.find({ label: 'Input color', role: 'Combobox' });
   const check = (min, max) => client.waitFor(async () => {
     const pixels = decodePng(await client.screenshot());
     const viewer = await client.find({ label: 'Video preview', role: 'Image' });
     const win = await client.windowProperties();
     const scale = pixels.width / (win.size.width / (win.scaleFactor || 1));
     const x = Math.floor((viewer.absolutePosition.x + viewer.size.width / 2) * scale);
     const y = Math.floor((viewer.absolutePosition.y + viewer.size.height / 2) * scale);
     const level = pixels.pixels[(y * pixels.width + x) * pixels.channels];
     return level >= min && level <= max;
   }, { description: `Sony monitor level ${min}–${max}` });
   const select = async (keys, label) => {
     await client.click(await choice());
     for (const key of keys) await client.key(key);
     await client.key('\n');
     await client.waitFor(async () => (await choice()).accessibleValue === label,
       { description: `selected ${label}` });
   };
   assert.equal((await choice()).accessibleValue, 'Auto (metadata)');
   await check(114, 120);
   await select(['\uf701'], 'Sony S-Log3 / S-Gamut3.Cine'); await check(105, 112);
   await select(['\uf701'], 'Sony S-Log3 / S-Gamut3'); await check(105, 112);
   await select(['\uf701'], 'Sony S-Log2 / S-Gamut'); await check(129, 138);
   await select(['\uf700','\uf700','\uf700'], 'Auto (metadata)'); await check(114, 120);
   await select(['\uf701'], 'Sony S-Log3 / S-Gamut3.Cine');
   assert.match((await assetCard('sony-log-reference.mp4')).accessibleDescription, /S-Gamut3.Cine \(override\)/);
   const project = join(output, 'sony-input.oqvideo.json');
   await fileAction('SAVE AS', project, 'Save project'); await waitState('Project saved'); await close();
   await launch(project); await waitState('assets 1');
   await client.click(await assetCard('sony-log-reference.mp4')); await check(105, 112);
   assert.equal((await choice()).accessibleValue, 'Sony S-Log3 / S-Gamut3.Cine');
   await close();
 }, { core: true });
}

try {
  if (deliveryOnly) await hardwareDeliveryRoundTrip();
  else {
    if (audioOnly || (!smoke && !previewOnly)) await audioOnlyRoundTrip();
    if (!audioOnly) await runVideoCases();
  }
  report.passed=true;
}catch(error){report.error=error.stack;try{await client.screenshot(join(output,'failure.png'));await writeFile(join(output,'failure-tree.json'),JSON.stringify(await client.tree(),null,2));}catch{}throw error;
}finally{await writeFile(join(output,'result.json'),JSON.stringify(report,null,2));await writeHtmlReport(output,report,{...report,caseDefinitions:report.caseDefinitions});if(child)child.kill('SIGTERM');console.log(`[video-editor] Evidence: ${output}`);}
