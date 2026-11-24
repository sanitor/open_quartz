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
const positional = args.filter(arg => arg !== '--smoke');
assert(positional.length <= 1 && positional.every(arg => !arg.startsWith('--')), 'Expected [binary] [--smoke]');
const tier = smoke ? 'smoke' : 'full';
const output=resolve(process.env.VIDEO_EDITOR_TEST_OUTPUT || `test-results/phase6/ui-${tier}-${new Date().toISOString().replaceAll(/[:.]/g,'-')}`);
const binary=resolve(positional[0] || 'apps/open_quartz_video_editor/target/debug/open_quartz_video_editor');
const ffmpeg=resolve(process.env.OPEN_QUARTZ_FFMPEG_PATH || 'apps/open_quartz_slint/runtime/ffmpeg');
await mkdir(output,{recursive:true});
const report={passed:false,tier,steps:[],screenshots:[],launches:[],caseDefinitions:[]};
const server=createServer();await new Promise(r=>server.listen(0,'127.0.0.1',r));const port=server.address().port;await new Promise(r=>server.close(r));
let child,client,exited,stdout='',stderr='';
const project=join(output,'edit.oqvideo.json');
async function launch(path){
  stdout='';stderr='';child=spawn(binary,path?['--project',path]:[],{env:{...process.env,SLINT_MCP_PORT:String(port),OPEN_QUARTZ_UI_CLOSE_STDIN:'1'},stdio:['pipe','pipe','pipe']});
  child.stdout.on('data',data=>stdout+=data);child.stderr.on('data',data=>stderr+=data);
  exited=new Promise((accept,reject)=>{child.once('error',reject);child.once('close',(code,signal)=>accept({code,signal}));});
  client=new SlintUiClient(`http://127.0.0.1:${port}/mcp`);
  await client.waitFor(async()=>{try{await client.init();await client.useWindow();return true;}catch{return false;}},{description:'native editor window'});
}
async function close(){await click('Close window');const exit=await Promise.race([exited,delay(10000).then(()=>{throw Error('Window did not close normally');})]);assert.deepEqual(exit,{code:0,signal:null});report.launches.push(exit);await writeFile(join(output,`application-${report.launches.length}.log`),stdout+stderr);child=undefined;}
const state=async()=> (await client.properties(client.rootHandle)).accessibleDescription;
async function click(label,last=false){const items=await client.byLabel(label,{role:'Button'});assert(items.length,`Missing UI button ${label}`);await client.click(last?items.at(-1):items[0]);}
async function fileAction(action,path,submit){await click(action);await client.setValue(await client.find({label:'File path',role:'TextInput'}),path);await click(submit,true);}
const timeline=async name=>(await client.byLabel(name,{role:'Button'})).filter(item=>item.accessibleDescription?.startsWith('Timeline start'));
const assetCard=async name=>{const cards=(await client.byLabel(name,{role:'Button'})).filter(item=>item.typeNamesAndIds?.some(type=>type.typeName==='AssetCard'));assert.equal(cards.length,1,`Expected one media card ${name}`);return cards[0];};
async function waitState(fragment){return client.waitFor(async()=> (await state()).includes(fragment),{description:fragment});}
async function previewHasColor(png,color){const preview=await client.find({label:'Video preview',role:'Image'});const image=decodePng(png);const window=await client.windowProperties();const scale=window.scaleFactor||1;const sx=image.width/(window.size.width/scale),sy=image.height/(window.size.height/scale);const x=Math.floor(((preview.absolutePosition.x||0)+preview.size.width/2)*sx);const y=Math.floor(((preview.absolutePosition.y||0)+preview.size.height/2)*sy);let hits=0;for(let dy=-15;dy<15;dy++)for(let dx=-15;dx<15;dx++){const at=((y+dy)*image.width+x+dx)*image.channels;const [r,g,b]=image.pixels.subarray(at,at+3);if(color==='red'?r>220&&g<25&&b<25:color==='blue'?b>220&&r<25&&g<25:color==='blend'?r>=178&&r<=196&&g<25&&b>=178&&b<=196:r<5&&g<5&&b<5)hits++;}return hits>=890;}
async function snapshot(name,color){const path=join(output,`${name}.png`);const png=await client.screenshot(path);report.screenshots.push(path);if(color)assert(await previewHasColor(png,color),`${name}: wrong visible ${color} pixels`);return png;}
async function step(name, run, { core = false } = {}) {
  if (smoke && !core) return;
  report.caseDefinitions.push({ id: name, title: name, description: 'Native UI actions and observable results', screenshotPrefixes: [] });
  const startedAt=new Date().toISOString();const first=report.screenshots.length;try{await run();report.steps.push({name,passed:true,startedAt,finishedAt:new Date().toISOString(),screenshots:report.screenshots.slice(first)});console.log(`[video-editor] PASS ${name}`);}catch(error){report.steps.push({name,passed:false,startedAt,finishedAt:new Date().toISOString(),error:error.message,screenshots:report.screenshots.slice(first)});throw error;}}
function generate(name,color,duration=3,fps=30,size='320x180'){const path=join(output,name);execFileSync(ffmpeg,['-hide_banner','-loglevel','error','-f','lavfi','-i',`color=c=${color}:s=${size}:r=${fps}`,'-vf','scale=in_color_matrix=bt601:out_color_matrix=bt709','-t',String(duration),'-c:v','libx264','-pix_fmt','yuv420p','-color_primaries','bt709','-color_trc','bt709','-colorspace','bt709','-y',path],{env:{...process.env,DYLD_LIBRARY_PATH:'',DYLD_FALLBACK_LIBRARY_PATH:''}});return path;}
const red=generate('red.mp4','red'),blue=generate('blue.mp4','blue');
try{
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
 await step('Relink matching original and restore hardware image',async()=>{await fileAction('RELINK',join(output,'red-relocated.mp4'),'Relink media');await client.waitFor(async()=> (await client.byLabel('red.mp4',{role:'Button'})).some(i=>i.accessibleDescription?.includes('; online')),{description:'matched relink'});const item=(await client.byLabel('red.mp4',{role:'Button'})).find(i=>i.accessibleDescription?.includes('; online'));await client.click(item);await waitState('source PTS 0 ns · CPU image bytes 0');await snapshot('09-restored','red');await click('SAVE');await waitState('Project saved');await close();}, { core: true });
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
 const expectColor = async color => {
   await client.waitFor(async () => previewHasColor(await client.screenshot(), color),
     { description: `composited ${color} pixels` });
 };
 const center = item => ({
   x: item.absolutePosition.x + item.size.width / 2,
   y: item.absolutePosition.y + item.size.height / 2,
 });
 const grip = name => client.find({ label: `Reorder ${name}`, role: 'Button' });
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
   assert.equal(data.schema_version, 5);
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
 const findInput = label => client.find({ label, role: 'TextInput' });
 const enter = async (input, value) => {
   await client.click(input);
   await client.setValue(input, String(value));
   await client.key('\n');
 };
 const timingProject = join(output, 'timing.oqvideo.json');
 const outgoing = 'timing-blue.mp4';
 const incoming = 'linked-tone.mp4';
 const positions = async (name) => (await timeline(name)).map(clip => {
   const [, start, duration] = /Timeline start (\d+) frames; duration (\d+) frames/.exec(clip.accessibleDescription);
   return [Number(start), Number(duration)];
 }).sort(([start], [other]) => start - other);
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
 const clipControl = async (label, role = 'TextInput') => {
   const viewport = await client.find({ id: 'EditorWindow::clip_properties' });
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
     { source_ns: 0, opacity: 0 }, { source_ns: 66666666, opacity: 1 },
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
 report.passed=true;
}catch(error){report.error=error.stack;try{await client.screenshot(join(output,'failure.png'));await writeFile(join(output,'failure-tree.json'),JSON.stringify(await client.tree(),null,2));}catch{}throw error;
}finally{await writeFile(join(output,'result.json'),JSON.stringify(report,null,2));await writeHtmlReport(output,report,{...report,caseDefinitions:report.caseDefinitions});if(child)child.kill('SIGTERM');console.log(`[video-editor] Evidence: ${output}`);}
