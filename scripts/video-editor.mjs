#!/usr/bin/env node
import { spawn } from 'node:child_process';
import { resolve, join, delimiter } from 'node:path';
import { fileURLToPath } from 'node:url';
import { mediaSdk, prepareMediaSdk } from './prepare-media-sdk.mjs';
import { packageVideoEditor } from './package-video-editor.mjs';

const root=resolve(fileURLToPath(new URL('..',import.meta.url)));
const [mode='dev',...args]=process.argv.slice(2);
if(!['dev','test','test-ui','test-ui-stills','test-ui-audio','test-coverage','build','package'].includes(mode))throw Error('Usage: video-editor.mjs dev [--project FILE] | test | test-ui [--smoke] | test-ui-stills | test-ui-audio | test-coverage | build | package');
if(mode==='test-ui' && (args.length>1 || args.some(arg=>arg!=='--smoke')))throw Error('Expected test-ui [--smoke]');
const windows = process.platform === 'win32' && process.arch === 'x64';
if (process.platform !== 'darwin' && !windows) throw Error('Video Editor supports macOS and Windows x64.');
if (windows && ['test-ui', 'test-coverage'].includes(mode)) {
  throw Error(`Video Editor ${mode} currently requires the macOS VideoToolbox media backend.`);
}
async function run(command,arguments_,env=process.env){
  await new Promise((accept,reject)=>{
    const child=spawn(command,arguments_,{cwd:root,stdio:'inherit',env});
    const forward=signal=>child.kill(signal);const sigint=()=>forward('SIGINT');const sigterm=()=>forward('SIGTERM');
    process.on('SIGINT',sigint);process.on('SIGTERM',sigterm);
    child.once('error',reject);child.once('exit',(code,signal)=>{process.removeListener('SIGINT',sigint);process.removeListener('SIGTERM',sigterm);if(code===0)accept();else reject(Error(`${command} failed (${signal||code})`));});
  });
}
await prepareMediaSdk();
if(mode!=='package' || windows)await run(process.execPath,['scripts/copy-ffmpeg.mjs']);
const sdk=join(mediaSdk,'lib');
const env = {
  ...process.env,
  OPEN_QUARTZ_FFMPEG_PATH: join(root, 'apps/open_quartz_slint/runtime', windows ? 'ffmpeg.exe' : 'ffmpeg'),
};
if (!windows) env.DYLD_LIBRARY_PATH = [sdk, process.env.DYLD_LIBRARY_PATH].filter(Boolean).join(delimiter);
if (windows) {
  env.FFMPEG_DIR = join(root, 'apps/open_quartz_slint/runtime/ffmpeg-shared');
  const pathKey = Object.keys(env).find(name => name.toLowerCase() === 'path') || 'PATH';
  env[pathKey] = [join(env.FFMPEG_DIR, 'bin'), env[pathKey]].filter(Boolean).join(delimiter);
}
const manifest=join(root,'apps/open_quartz_video_editor/Cargo.toml');
if(mode==='dev')await run('cargo',['run','--locked','--manifest-path',manifest,'--',...args],env);
if(mode==='test')await run('cargo',['test','--locked','--manifest-path',manifest,'--','--test-threads=1'],env);
if(mode==='build')await run('cargo',['build','--locked','--release','--manifest-path',manifest],env);
if(mode==='package'){
  const target = join(root,'apps/open_quartz_video_editor/target');
  if (windows) {
    const localEnv = { ...env };
    delete localEnv.CARGO_BUILD_TARGET;
    await run('cargo', ['build', '--locked', '--release', '--manifest-path', manifest, '--target-dir', target], localEnv);
    await packageVideoEditor(root, join(target, 'release', 'open_quartz_video_editor.exe'));
  } else {
    const localEnv = {...env, MACOSX_DEPLOYMENT_TARGET:'13.0'};
    delete localEnv.CARGO_BUILD_TARGET;
    await run('cargo',['rustc','--locked','--release','--manifest-path',manifest,'--target-dir',target,'--',
      '-C','link-arg=-Wl,-rpath,@executable_path/../Resources/runtime'],localEnv);
    await packageVideoEditor(root,join(target,'release','open_quartz_video_editor'));
  }
}

async function runEditorUi(binary, options, testEnv) {
  const failures = [];
  for (const script of ['test-video-editor-ui.mjs', 'test-video-editor-nodes-ui.mjs']) {
    try { await run(process.execPath, [`scripts/${script}`, binary, ...options], testEnv); }
    catch (error) { failures.push(error); }
  }
  if (failures.length) throw new AggregateError(failures, 'Video Editor UI failed');
}
if(['test-ui', 'test-ui-stills', 'test-ui-audio'].includes(mode)){
  await run('cargo',['build','--locked','--manifest-path',manifest,'--features','ui-automation'],env);
  const binary = join(root, 'apps/open_quartz_video_editor/target/debug', windows ? 'open_quartz_video_editor.exe' : 'open_quartz_video_editor');
  if (mode === 'test-ui-stills') {
    await run(process.execPath, ['scripts/test-video-editor-nodes-ui.mjs', binary, '--still-only'], env);
  } else if (mode === 'test-ui-audio') {
    await run(process.execPath, ['scripts/test-video-editor-ui.mjs', binary, '--audio-only'], env);
  } else {
    await runEditorUi(binary, args, env);
  }
}
if(mode==='test-coverage'){
  await run('cargo',['llvm-cov','--manifest-path',manifest,'--locked','--no-report','--','--test-threads=1'],env);
  await run('cargo',['llvm-cov','run','--manifest-path',manifest,'--locked','--features','ui-automation','--no-report','--','--help'],env);
  const target=join(root,'apps/open_quartz_video_editor/target/llvm-cov-target');
  const uiEnv={...env,LLVM_PROFILE_FILE:join(target,'open_quartz_video_editor-%p-%m.profraw')};
  const binary=join(target,'debug/open_quartz_video_editor');
  await runEditorUi(binary, [], uiEnv);
  await run('cargo',['llvm-cov','report','--manifest-path',manifest,'--json','--output-path',join(root,'test-results/phase6/editor-coverage.json')],env);
}
