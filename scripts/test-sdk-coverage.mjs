import { spawnSync } from 'node:child_process';

const packages = [
  ['open_quartz_schema', 'coverage_tests\\.rs|/tests/'],
  ['open_quartz_execution', '(/tests/|_coverage_tests\\.rs$)'],
  ['open_quartz_host_api', 'coverage_tests\\.rs|/tests/'],
  ['open_quartz_sdk', 'coverage_tests\\.rs|/tests/'],
  ['open_quartz', 'coverage_tests\\.rs|/tests/'],
  ['open_quartz_bindings', 'coverage_tests\\.rs|/tests/'],
];

for (const [packageName, ignoredFiles] of packages) {
  const clean = spawnSync('cargo', ['llvm-cov', 'clean', '--workspace'], { stdio: 'inherit' });
  if (clean.error) throw clean.error;
  if (clean.status !== 0) process.exit(clean.status ?? 1);
  const args = [
    'llvm-cov',
    '-p',
    packageName,
    '--lib',
    '--ignore-filename-regex',
    ignoredFiles,
    '--fail-under-lines',
    '100',
    '--fail-under-functions',
    '100',
    '--fail-under-regions',
    '100',
    '--summary-only',
  ];
  if (process.platform === 'win32' && packageName === 'open_quartz_execution') {
    args.splice(4, 0, '--features', 'native-video-d3d12');
  }
  console.log(`\n[sdk-coverage] ${packageName}`);
  const result = spawnSync('cargo', args, { stdio: 'inherit' });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}
