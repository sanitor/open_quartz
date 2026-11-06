import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

// Research only: these manifests are independent of the application workspace.
const here = dirname(fileURLToPath(import.meta.url));
const repository = resolve(here, '../..');
const directory = resolve(process.argv[2] ?? join(repository, 'test-results/research', `wgsl-highlight-${new Date().toISOString().replaceAll(':', '-')}`));
await mkdir(directory, { recursive: true });
const catalogBytes = await readFile(join(repository, 'crates/open_quartz/src/project/shader_catalog.json'));
const catalog = JSON.parse(catalogBytes);
const cases = catalog.groups.flatMap((group) => group.items.map((item) => ({ label: item.label, catalog: true, source: item.code })));
const edges = [
  ['nested-comment', '/* 外层 /* inner */ still comment */\nfn main() {}'],
  ['unterminated-comment', 'fn main() {}\n/* 外层 /* inner */'],
  ['unfinished-expression', '@fragment fn main() -> @location(0) vec4f { return vec4f('],
  ['unicode-comment-crlf-tabs', '// 中文 😀\r\nfn main() {\r\n\tlet x = 1.0;\r\n}\r\n'],
  ['unicode-identifier', 'fn main() { let 增益 = 1.0; }'],
  ['modern-types', 'alias Pixel = vec4f; const factor = 1.0; fn main() -> Pixel { return Pixel(factor); }'],
  ['numeric-literals', 'fn main() { let a = 0x1.fp+2f; let b = 1e-3f; let c = 12u; let d = .5h; }'],
  ['diagnostic-directive', 'diagnostic(off, derivative_uniformity); @fragment fn main() -> @location(0) vec4f { return vec4f(1.0); }'],
  ['const-assert', 'const_assert 1 < 2; fn main() {}'],
  ['generic-and-shift', 'fn main() { var a: array<vec4<f32>, 4>; let b = 8u >> 1u; }'],
  ['bevy-extension', '#import bevy_pbr::forward_io::VertexOutput\nfn main() {}'],
  ['unfinished-attribute', '@group(0) @binding('],
  ['hex-float-only', 'fn main() { let a = 0x1.fp+2f; }'],
  ['numeric-enabled', 'enable f16; fn main() { let a = 0x1.fp+2f; let b = 1e-3f; let c = 12u; let d = .5h; }'],
  ['unknown-char-recovery', 'fn main() { § let next = 1.0; }'],
];
cases.push(...edges.map(([label, source]) => ({ label, source, catalog: false })));
const input = join(directory, 'cases.json');
await writeFile(input, JSON.stringify({ cases }, null, 2));
const candidates = [
  ['legacy', 'tree-sitter-wgsl 0.0.6 / tree-sitter 0.20.10'],
  ['bevy', 'tree-sitter-wgsl-bevy 0.1.4 / tree-sitter 0.25.10'],
  ['lexer', 'wgsl-parse 0.3.2 / raw Logos 0.16.1'],
  ['lexer4', 'wgsl-parse 0.4.0 / wgsl-types 0.4.0 / raw Logos 0.16.1'],
];
const summaries = [];
for (const [name, candidate] of candidates) {
  const output = join(directory, `${name}.json`);
  console.log(`Research candidate: ${candidate}`);
  await new Promise((done, fail) => {
    const child = spawn('cargo', ['run', '--release', '--locked', '--manifest-path', join(here, name, 'Cargo.toml'), '--', input, output], {
      cwd: repository,
      env: { ...process.env, WGSL_RESEARCH_CANDIDATE: candidate, CARGO_TARGET_DIR: join(directory, 'cargo-target') },
      stdio: 'inherit',
    });
    child.once('error', fail);
    child.once('exit', (code, signal) => code === 0 ? done() : fail(new Error(`${candidate}: exit=${code} signal=${signal}`)));
  });
  const result = JSON.parse(await readFile(output, 'utf8'));
  assert.equal(result.results.length, cases.length);
  assert.ok(result.results.every((entry) => entry.utf8_boundaries_valid), `${candidate}: invalid UTF-8 boundary`);
  assert.equal(result.catalog_successes ?? result.catalog_lexical_successes, catalog.groups.flatMap((group) => group.items).length);
  const benchmarks = result.parser_only_benchmarks ?? result.lexer_only_benchmarks;
  for (const benchmark of benchmarks) {
    if ('incremental_matches_fresh' in benchmark) assert.equal(benchmark.incremental_matches_fresh, true);
  }
  summaries.push({ candidate, catalogSuccesses: result.catalog_successes ?? result.catalog_lexical_successes, benchmarks,
    edges: result.results.filter((entry) => !entry.catalog).map((entry) => ({ label: entry.label, hasError: entry.has_error ?? entry.errors.length > 0,
      utf8: entry.utf8_boundaries_valid, errors: entry.anomalies ?? entry.errors, nagaAccepts: entry.naga_parse_error === undefined ? undefined : entry.naga_parse_error === null,
      comments: entry.comments?.map(({ kind, start, end }) => ({ kind, start, end })) })) });
}
const metadata = { measuredAt: new Date().toISOString(), platform: process.platform, architecture: process.arch,
  node: process.version, catalogSha256: createHash('sha256').update(catalogBytes).digest('hex'),
  catalogCases: cases.filter((entry) => entry.catalog).length, edgeCases: edges.length, candidates: summaries,
  limitations: ['Parser/lexer measurements only; no query evaluation, style model publication or Slint shaping/drawing.',
    'Tree-sitter error recovery does not prove usable highlighting or WGSL validity.',
    'A raw lexer does not perform parsing or semantic validation.',
    'Incremental benchmark inserts a comment at byte zero of repeated synthetic functions; results are workload-specific.'] };
await writeFile(join(directory, 'summary.json'), JSON.stringify(metadata, null, 2));
console.log(`Research evidence: ${directory}`);
