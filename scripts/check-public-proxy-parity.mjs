import { readFileSync } from 'node:fs';

const objects = ['OpenQuartzClient', 'Project', 'Graph', 'Node', 'Port', 'Player', 'Resource', 'Output', 'Catalog'];
const ts = readFileSync('src/sdk/index.ts', 'utf8');
for (const object of objects) {
  if (!ts.includes(object)) throw new Error(`TypeScript public proxy is missing ${object}`);
  const java = readFileSync(`java/sdk/src/main/java/com/sanitor/openquartz/${object}.java`, 'utf8');
  if (!new RegExp(`(?:class|record) ${object}`).test(java)) {
    throw new Error(`Java public proxy is missing ${object}`);
  }
}
for (const forbidden of ['PipelineRuntime', 'ExecutionEngine', 'Compositor', 'GpuExecutor']) {
  if (ts.includes(forbidden)) throw new Error(`TypeScript public SDK exports internal ${forbidden}`);
}
console.log(`Public proxy parity passed for ${objects.length} objects`);
