// Reproducible, LGPL-only libav SDK for the macOS in-process VideoToolbox backend.
// The existing FFmpeg executable remains the camera/software fallback and is
// distributed separately with its own notices. No Homebrew libraries are bundled.
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { availableParallelism } from 'node:os';
import { mkdir, readFile, writeFile, copyFile, cp, rm, access, readdir, rename } from 'node:fs/promises';
import { resolve, join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

export const version = '8.1.1';
export const sha256 = 'b6863adde98898f42602017462871b5f6333e65aec803fdd7a6308639c52edf3';
export const sourceUrl = `https://ffmpeg.org/releases/ffmpeg-${version}.tar.xz`;
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export const mediaSdk = join(root, 'apps', 'open_quartz_slint', 'runtime', 'ffmpeg-media');
const cache = join(root, 'node_modules', '.cache', 'open-quartz', `libav-${version}-${process.arch}`);
const revision = 'oq-vt-hw3-metal-encode-r2';

function run(command, args, cwd) {
  return new Promise((accept, reject) => {
    const child = spawn(command, args, { cwd, stdio: 'inherit' });
    child.once('error', reject);
    child.once('exit', (code, signal) => code === 0 ? accept() : reject(new Error(`${command} failed: ${signal || code}`)));
  });
}

export async function prepareMediaSdk() {
  if (process.platform !== 'darwin') return;
  if (!['arm64', 'x64'].includes(process.arch)) throw new Error(`Unsupported macOS architecture: ${process.arch}`);
  const marker = `${sha256}\n${revision}\n${process.arch}\n`;
  if (await readFile(join(mediaSdk, '.build-id'), 'utf8').catch(() => '') === marker) {
    await access(join(mediaSdk, 'lib', 'libavcodec.62.dylib'));
    await access(join(mediaSdk, 'lib', 'pkgconfig', 'libavcodec.pc'));
    return;
  }
  await mkdir(cache, { recursive: true });
  const archive = join(cache, `ffmpeg-${version}.tar.xz`);
  let bytes = await readFile(archive).catch(() => null);
  if (!bytes) {
    const response = await fetch(sourceUrl, { signal: AbortSignal.timeout(180_000) });
    if (!response.ok) throw new Error(`Cannot download ${sourceUrl}: ${response.status}`);
    bytes = Buffer.from(await response.arrayBuffer());
    await writeFile(archive, bytes);
  }
  if (createHash('sha256').update(bytes).digest('hex') !== sha256) throw new Error(`libav source checksum mismatch: ${archive}`);
  const source = join(cache, `ffmpeg-${version}`);
  await rm(source, { recursive: true, force: true });
  await run('tar', ['-xf', archive, '-C', cache], root);
  await run('patch', ['-p1', '-i', join(root, 'scripts', 'ffmpeg-videotoolbox-hardware.patch')], source);
  await run('patch', ['-p1', '-i', join(root, 'scripts', 'ffmpeg-videotoolbox-reorder.patch')], source);
  const install = join(cache, 'install');
  await rm(install, { recursive: true, force: true });
  const args = [
    `--prefix=${install}`, '--disable-autodetect', '--disable-everything',
    `--extra-version=${revision}`,
    '--disable-programs', '--disable-doc', '--disable-debug', '--disable-static', '--enable-shared',
    '--disable-gpl', '--disable-nonfree', '--disable-avdevice', '--disable-avfilter', '--disable-swresample',
    '--enable-avcodec', '--enable-avformat', '--enable-swscale', '--enable-videotoolbox',
    '--enable-encoder=h264_videotoolbox', '--enable-muxer=mp4,mov',
    '--enable-decoder=h264,hevc,mpeg4,vp9,av1', '--enable-parser=h264,hevc,mpeg4video,vp9,av1',
    '--enable-demuxer=mov,matroska,avi,mpegts', '--enable-protocol=file',
    '--enable-hwaccel=h264_videotoolbox,hevc_videotoolbox',
    '--install-name-dir=@rpath', '--extra-cflags=-mmacosx-version-min=13.0',
    '--extra-ldflags=-mmacosx-version-min=13.0',
    ...(process.arch === 'x64' ? ['--disable-x86asm'] : []),
  ];
  await run('./configure', args, source);
  await run('make', [`-j${Math.min(availableParallelism(), 12)}`], source);
  await run('make', ['install'], source);
  // Stage before replacing the SDK so failed preparation does not look complete.
  const staging = `${mediaSdk}.staging`;
  await rm(staging, { recursive: true, force: true });
  await cp(install, staging, { recursive: true, dereference: true });
  // pkg-config records the final (not temporary install) SDK path.
  const pc = join(staging, 'lib', 'pkgconfig');
  for (const file of await readdir(pc)) {
    const path = join(pc, file);
    await writeFile(path, (await readFile(path, 'utf8')).replaceAll(install, mediaSdk));
  }
  await copyFile(join(source, 'COPYING.LGPLv2.1'), join(staging, 'COPYING.LGPLv2.1'));
  await copyFile(archive, join(staging, `ffmpeg-${version}.tar.xz`));
  await copyFile(join(root, 'scripts', 'ffmpeg-videotoolbox-hardware.patch'), join(staging, 'ffmpeg-videotoolbox-hardware.patch'));
  await copyFile(join(root, 'scripts', 'ffmpeg-videotoolbox-reorder.patch'), join(staging, 'ffmpeg-videotoolbox-reorder.patch'));
  await writeFile(join(staging, 'BUILD-CONFIG.txt'), `${sourceUrl}\nSHA-256: ${sha256}\n./configure ${args.join(' ')}\n`);
  await writeFile(join(staging, 'FFMPEG-MEDIA-NOTICE.txt'), `FFmpeg ${version}, LGPL-2.1-or-later.\nSource: ${sourceUrl}\nSHA-256: ${sha256}\nBuilt without GPL/nonfree/external codec libraries.\nModifications: require H.264/HEVC hardware decode and Metal-compatible output; enable hardware H.264 encoder and MP4/MOV mux; backport upstream H.264 reorder-depth correction 2da55bf59a68801a8157ab141a487196ce3416a8.\nApply patch -p1 < ffmpeg-videotoolbox-hardware.patch and patch -p1 < ffmpeg-videotoolbox-reorder.patch before configuring. Original source, exact patches, LGPL license and build options included.\nlibavcodec ABI 62 / libavformat ABI 62 / libavutil ABI 60.\n`);
  await writeFile(join(staging, '.build-id'), marker);
  await rm(mediaSdk, { recursive: true, force: true });
  await rename(staging, mediaSdk);
  console.log(`[media-sdk] prepared ${mediaSdk}`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await prepareMediaSdk();
