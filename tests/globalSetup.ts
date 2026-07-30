import { execFileSync } from 'node:child_process';
import { resolve } from 'node:path';

export default function setup(): void {
  const crate = resolve('crates/open_quartz');
  execFileSync(
    'wasm-pack',
    [
      'build',
      crate,
      '--target',
      'nodejs',
      '--dev',
      '--out-dir',
      '../../node_modules/.tmp/open_quartz-sdk-node',
      '--out-name',
      'open_quartz',
    ],
    { stdio: 'pipe' },
  );
}
