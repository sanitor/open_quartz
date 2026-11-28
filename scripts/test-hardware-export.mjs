#!/usr/bin/env node
import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdir, mkdtemp } from 'node:fs/promises';
import { delimiter, join, resolve } from 'node:path';

const windows = process.platform === 'win32';
assert(windows || process.platform === 'darwin', 'The hardware export test requires Windows or macOS');
execFileSync(process.execPath, ['scripts/desktop.mjs', 'prepare'], { stdio: 'inherit' });
const runtime = resolve('apps/open_quartz_slint/runtime');
const env = { ...process.env };
let ffmpeg = join(runtime, 'ffmpeg');
let fixtureEncoder = ffmpeg;
let features = 'native-runtime,native-video';
if (windows) {
  env.FFMPEG_DIR = join(runtime, 'ffmpeg-shared');
  const bin = join(env.FFMPEG_DIR, 'bin');
  const pathName = Object.keys(env).find(key => key.toLowerCase() === 'path') || 'Path';
  env[pathName] = [bin, env[pathName]].filter(Boolean).join(delimiter);
  ffmpeg = join(bin, 'ffmpeg.exe');
  const { default: installer } = await import('@ffmpeg-installer/ffmpeg');
  fixtureEncoder = installer.path;
  features = 'native-runtime,native-video-precise-d3d12';
} else {
  delete env.FFMPEG_DIR;
  env.PKG_CONFIG_PATH = join(runtime, 'ffmpeg-media', 'lib', 'pkgconfig');
  env.DYLD_LIBRARY_PATH = join(runtime, 'ffmpeg-media', 'lib');
}
delete env.CARGO_BUILD_TARGET;
env.OPEN_QUARTZ_FFMPEG_PATH = ffmpeg;
execFileSync('cargo', ['test', '--locked', '-p', 'open_quartz', '--lib', '--features', features,
  'execution::pipeline::export::tests', '--', '--test-threads=1'], { env, stdio: 'inherit' });
const mediaEnv = { ...env, DYLD_LIBRARY_PATH: '', DYLD_FALLBACK_LIBRARY_PATH: '' };
const resultRoot = resolve('test-results/phase5');
await mkdir(resultRoot, { recursive: true });
const output = await mkdtemp(join(resultRoot, 'headless-export-'));
const source = join(output, 'source.mp4');
const destination = join(output, 'invert.mp4');
const width = 320;
const height = 180;
const frames = 12;
execFileSync(fixtureEncoder, ['-v', 'error', '-f', 'lavfi', '-i', `color=red:s=${width}x${height}:r=30`,
  '-vf', 'scale=in_color_matrix=bt601:out_color_matrix=bt709',
  '-frames:v', '24', '-c:v', 'libx264', '-pix_fmt', 'yuv420p',
  '-color_primaries', 'bt709', '-color_trc', 'bt709', '-colorspace', 'bt709', source],
{ env: mediaEnv, stdio: 'inherit' });

const result = spawnSync('cargo', ['run', '--locked', '-p', 'open_quartz', '--features', features,
  '--example', 'export_video', '--', source, destination, '1', String(frames + 1), '30', '1', '--invert'],
{ env, encoding: 'utf8' });
assert.ifError(result.error);
assert.equal(result.status, 0, result.stderr);
assert.match(result.stdout, /hardware: true/);
assert.match(result.stdout, windows ? /h264_d3d12va/ : /h264_videotoolbox/);

const stride = width * height * 3;
const passthrough = windows ? ['-fps_mode', 'passthrough'] : ['-vsync', '0'];
const pixels = execFileSync(ffmpeg, ['-v', 'error', '-i', destination, '-an',
  ...passthrough, '-pix_fmt', 'rgb24', '-f', 'rawvideo', '-'],
{ env: mediaEnv, maxBuffer: (frames + 2) * stride });
assert.equal(pixels.length, frames * stride, 'exported half-open range must retain every frame');
for (let frame = 0; frame < frames; frame++) {
  const offset = frame * stride + (Math.floor(height / 2) * width + Math.floor(width / 2)) * 3;
  const [red, green, blue] = pixels.subarray(offset, offset + 3);
  assert(red < 28 && green > 225 && blue > 225, `decoded frame ${frame} must be cyan after inversion`);
}
console.log(`Verified ${frames} hardware-exported and independently decoded frames in ${output}`);
