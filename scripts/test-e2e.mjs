#!/usr/bin/env node
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const [tier, ...extra] = process.argv.slice(2);
if (extra.length || !['smoke', 'full'].includes(tier)) {
  throw new Error('Usage: node scripts/test-e2e.mjs smoke|full');
}
if (!['darwin', 'win32'].includes(process.platform)) {
  throw new Error('The repository-wide E2E suites require macOS or Windows. Use the individual Desktop commands on other supported hosts.');
}

const cwd = fileURLToPath(new URL('..', import.meta.url));
const options = tier === 'smoke' ? ['--smoke'] : [];
const suites = [
  ['Desktop interactions', 'scripts/desktop.mjs', 'test-ui'],
  ['Desktop catalog shaders', 'scripts/desktop.mjs', 'test-ui-shaders'],
  ['Desktop Color Corrector', 'scripts/desktop.mjs', 'test-ui-corrector'],
  ['Resident inference', 'scripts/desktop.mjs', 'test-inference'],
];

const failures = [];
for (const [label, script, ...args] of suites) {
  console.log(`[e2e:${tier}] ${label}`);
  try {
    await new Promise((resolve, reject) => {
      const child = spawn(process.execPath, [script, ...args, ...options], { cwd, stdio: 'inherit' });
      child.once('error', reject);
      child.once('close', (code, signal) => {
        if (code === 0) resolve();
        else reject(new Error(`${label} exited with ${signal || code}`));
      });
    });
  } catch (error) {
    failures.push(error);
  }
}
if (failures.length) throw new AggregateError(failures, `${tier} E2E suite failed`);
