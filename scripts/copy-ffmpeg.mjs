import { copyFile, mkdir, writeFile } from 'node:fs/promises';
import { basename, dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import installer from '@ffmpeg-installer/ffmpeg';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const runtimeDir = resolve(root, 'src-tauri', 'runtime');
const target = resolve(runtimeDir, basename(installer.path));

await mkdir(runtimeDir, { recursive: true });
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
