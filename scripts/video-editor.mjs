#!/usr/bin/env node
import { spawn } from 'node:child_process';
import { resolve, join, delimiter } from 'node:path';
import { fileURLToPath } from 'node:url';
import { mediaSdk, prepareMediaSdk } from './prepare-media-sdk.mjs';
import { packageVideoEditor } from './package-video-editor.mjs';

const root=resolve(fileURLToPath(new URL('..',import.meta.url)));
const [mode='dev',...args]=process.argv.slice(2);
if(!['dev','test','test-ui','test-coverage','build','package'].includes(mode))throw Error('Usage: video-editor.mjs dev [--project FILE] | test | test-ui [--smoke] | test-coverage | build | package');
if(mode==='test-ui' && (args.length>1 || args.some(arg=>arg!=='--smoke')))throw Error('Expected test-ui [--smoke]');
if(process.platform!=='darwin')throw Error('The precise resident video editor currently requires macOS VideoToolbox. No CPU video fallback is provided.');
async function run(command,arguments_,env=process.env){
  await new Promise((accept,reject)=>{
    const child=spawn(command,arguments_,{cwd:root,stdio:'inherit',env});
    const forward=signal=>child.kill(signal);const sigint=()=>forward('SIGINT');const sigterm=()=>forward('SIGTERM');
    process.on('SIGINT',sigint);process.on('SIGTERM',sigterm);
    child.once('error',reject);child.once('exit',(code,signal)=>{process.removeListener('SIGINT',sigint);process.removeListener('SIGTERM',sigterm);if(code===0)accept();else reject(Error(`${command} failed (${signal||code})`));});
  });
}
await prepareMediaSdk();
if(mode!=='package')await run(process.execPath,['scripts/copy-ffmpeg.mjs']);
const sdk=join(mediaSdk,'lib');
const env={...process.env,OPEN_QUARTZ_FFMPEG_PATH:join(root,'apps/open_quartz_slint/runtime/ffmpeg'),DYLD_LIBRARY_PATH:[sdk,process.env.DYLD_LIBRARY_PATH].filter(Boolean).join(delimiter)};
const manifest=join(root,'apps/open_quartz_video_editor/Cargo.toml');
if(mode==='dev')await run('cargo',['run','--locked','--manifest-path',manifest,'--',...args],env);
if(mode==='test')await run('cargo',['test','--locked','--manifest-path',manifest,'--','--test-threads=1'],env);
if(mode==='build')await run('cargo',['build','--locked','--release','--manifest-path',manifest],env);
if(mode==='package'){
  const localEnv = {...env, MACOSX_DEPLOYMENT_TARGET:'13.0'};
  delete localEnv.CARGO_BUILD_TARGET;
  const target = join(root,'apps/open_quartz_video_editor/target');
  await run('cargo',['rustc','--locked','--release','--manifest-path',manifest,'--target-dir',target,'--',
    '-C','link-arg=-Wl,-rpath,@executable_path/../Resources/runtime'],localEnv);
  await packageVideoEditor(root,join(target,'release','open_quartz_video_editor'));
}

async function runEditorUi(binary, options, testEnv) {
  const failures = [];
  for (const script of ['test-video-editor-ui.mjs', 'test-video-editor-nodes-ui.mjs']) {
    try { await run(process.execPath, [`scripts/${script}`, binary, ...options], testEnv); }
    catch (error) { failures.push(error); }
  }
  if (failures.length) throw new AggregateError(failures, 'Video Editor UI failed');
}
if(mode==='test-ui'){
  await run('cargo',['build','--locked','--manifest-path',manifest,'--features','ui-automation'],env);
  await runEditorUi(join(root,'apps/open_quartz_video_editor/target/debug/open_quartz_video_editor'), args, env);
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
