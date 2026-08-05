import { copyFile, cp, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { basename, dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import installer from '@ffmpeg-installer/ffmpeg';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const runtimeDir = resolve(root, 'src-tauri', 'runtime');
await mkdir(runtimeDir, { recursive: true });

if (process.platform === 'win32' && process.arch === 'x64') {
  const assetUrl = 'https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-n8.1-latest-win64-lgpl-shared-8.1.zip';
  const digest = '0d0f7449a5600ab5df9af19da861b24ca1534279ede099d6541f1fefb17bfba9';
  const cacheDir = resolve(root, 'node_modules', '.cache', 'open-quartz');
  const archive = resolve(cacheDir, `ffmpeg-shared-${digest}.zip`);
  const extracted = resolve(cacheDir, `ffmpeg-shared-${digest}`);
  const sharedRuntime = resolve(runtimeDir, 'ffmpeg-shared');
  const marker = resolve(sharedRuntime, '.sha256');
  const installedDigest = await readFile(marker, 'utf8').catch(() => '');
  if (installedDigest.trim() !== digest) {
    await mkdir(cacheDir, { recursive: true });
    const response = await fetch(assetUrl, {
      headers: { Accept: 'application/zip', 'User-Agent': 'OpenQuartz-runtime-preparer' },
      signal: AbortSignal.timeout(600_000),
    });
    const bytes = Buffer.from(await response.arrayBuffer());
    const actualDigest = createHash('sha256').update(bytes).digest('hex');
    if (actualDigest !== digest) {
      throw new Error(`Shared FFmpeg checksum mismatch: ${actualDigest}`);
    }
    await writeFile(archive, bytes);
    await rm(extracted, { recursive: true, force: true });
    await mkdir(extracted, { recursive: true });
    const unpack = spawnSync('tar.exe', ['-xf', archive, '-C', extracted], { stdio: 'inherit' });
    if (unpack.status !== 0) throw new Error(`Cannot extract shared FFmpeg (tar exit ${unpack.status})`);
    await rm(sharedRuntime, { recursive: true, force: true });
    await cp(
      resolve(extracted, 'ffmpeg-n8.1-latest-win64-lgpl-shared-8.1'),
      sharedRuntime,
      { recursive: true },
    );
    await writeFile(marker, `${digest}\n`, 'utf8');
  }
  for (const file of await readdir(resolve(sharedRuntime, 'bin'))) {
    if (file.endsWith('.dll')) {
      await copyFile(resolve(sharedRuntime, 'bin', file), resolve(runtimeDir, file));
    }
  }
  await copyFile(resolve(sharedRuntime, 'bin', 'ffmpeg.exe'), resolve(runtimeDir, 'ffmpeg.exe'));
  await writeFile(
    resolve(runtimeDir, 'FFMPEG-NOTICE.txt'),
    [
      'FFmpeg n8.1 LGPL shared build from BtbN/FFmpeg-Builds.',
      `Asset: ${assetUrl}`,
      `SHA-256: ${digest}`,
      'FFmpeg is distributed under LGPL-2.1-or-later; source offer and license are in the bundled build.',
      '',
    ].join('\n'),
    'utf8',
  );
  console.log(`[copy-ffmpeg] prepared shared FFmpeg at ${sharedRuntime}`);
} else {
  const target = resolve(runtimeDir, basename(installer.path));
  await copyFile(installer.path, target);
  await writeFile(
    resolve(runtimeDir, 'FFMPEG-NOTICE.txt'),
    [
      `FFmpeg ${installer.version}`,
      `Source and license information: ${installer.url}`,
      'Distributed by @ffmpeg-installer/ffmpeg under LGPL-2.1.',
      '',
    ].join('\n'),
    'utf8',
  );
  console.log(`[copy-ffmpeg] copied ${installer.path} to ${target}`);
}
