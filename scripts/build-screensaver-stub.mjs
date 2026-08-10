#!/usr/bin/env node
import { copyFile, mkdir } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';

if (process.platform !== 'win32') {
  console.log('[screensaver-stub] skipped: Windows-only export runtime');
  process.exit(0);
}

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const manifest = resolve(root, 'crates', 'open-quartz-screensaver-stub', 'Cargo.toml');
const targetDir = resolve(root, 'crates', 'open-quartz-screensaver-stub', 'target');
const executable = resolve(targetDir, 'release', 'open-quartz-screensaver-stub.exe');
const destination = resolve(root, 'src-tauri', 'runtime', 'open-quartz-screensaver-stub.exe');

await new Promise((resolvePromise, reject) => {
  const child = spawn('cargo', ['build', '--release', '--manifest-path', manifest], {
    cwd: root,
    stdio: 'inherit',
    env: { ...process.env, CARGO_TARGET_DIR: targetDir },
  });
  child.once('error', reject);
  child.once('exit', (code) => code === 0
    ? resolvePromise()
    : reject(new Error(`Screen saver stub build failed with exit code ${code}`)));
});
await mkdir(dirname(destination), { recursive: true });
await copyFile(executable, destination);
