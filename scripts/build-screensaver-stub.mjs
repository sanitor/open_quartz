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
const targetDir = resolve(root, 'src-tauri', 'target');
const target = 'x86_64-pc-windows-msvc';
const executable = resolve(targetDir, target, 'release', 'open-quartz-screensaver-stub.exe');
const destination = resolve(root, 'src-tauri', 'runtime', 'open-quartz-screensaver-stub.exe');
// The exported .scr must reach main() without app-local VC++ sidecars so it
// can extract the FFmpeg/ORT DLLs before their first use.
// An explicit target keeps these flags off host build scripts/proc-macros.
const stubEnv = { ...process.env, CARGO_TARGET_DIR: targetDir };
if (stubEnv.CARGO_ENCODED_RUSTFLAGS !== undefined) {
  stubEnv.CARGO_ENCODED_RUSTFLAGS += '\x1f-C\x1ftarget-feature=+crt-static';
} else {
  stubEnv.RUSTFLAGS = `${stubEnv.RUSTFLAGS || ''} -C target-feature=+crt-static`;
}

await new Promise((resolvePromise, reject) => {
  const child = spawn('cargo', ['build', '--release', '--locked', '--target', target, '--manifest-path', manifest], {
    cwd: root,
    stdio: 'inherit',
    env: stubEnv,
  });
  child.once('error', reject);
  child.once('exit', (code) => code === 0
    ? resolvePromise()
    : reject(new Error(`Screen saver stub build failed with exit code ${code}`)));
});
await mkdir(dirname(destination), { recursive: true });
await copyFile(executable, destination);
