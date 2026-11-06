import { spawnSync } from 'node:child_process';

// These files were excluded from the former native_host coverage gate before
// consolidation. Keep that exact scope; merging crates must not exclude more
// production implementation or relax the 100% SDK/execution thresholds.
const nativeIntegrationFiles = '/runtime/(execution|onnx_lifecycle_tests)\\.rs$|/resources/(models|models_tests)\\.rs$';
const ignoredFiles = `(/tests/|_coverage_tests\\.rs$|${nativeIntegrationFiles})`;
function run(args, collectFailure = false) {
  const result = spawnSync('cargo', args, { stdio: 'inherit' });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    if (!collectFailure) process.exit(result.status ?? 1);
    process.exitCode = result.status ?? 1;
  }
}

run(['llvm-cov', 'clean', '--workspace']);
const thresholds = ['--fail-under-lines', '100', '--fail-under-functions', '100', '--fail-under-regions', '100'];
run([
  'llvm-cov', '-p', 'open_quartz', '--lib', '--no-default-features',
  '--ignore-filename-regex', ignoredFiles, ...thresholds, '--summary-only',
], true);
run(['llvm-cov', 'clean', '--workspace']);
const features = ['native-runtime', 'native-video', 'native-onnx', 'model-cache'];
if (process.platform === 'win32') features.push('native-video-d3d12');
run([
  'llvm-cov', '-p', 'open_quartz', '--lib', '--features', features.join(','),
  '--ignore-filename-regex', ignoredFiles,
  '--fail-under-lines', '100', '--fail-under-functions', '100',
  '--fail-under-regions', '100', '--summary-only',
], true);
