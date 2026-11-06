import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';

const root = resolve('.');
const dir = mkdtempSync(join(tmpdir(), 'open-quartz-boundary-'));
const dependency = `open_quartz = { path = ${JSON.stringify(join(root, 'crates/open_quartz'))}, default-features = false }`;

function execute(command, args, cwd) {
  const result = spawnSync(command, args, { cwd, encoding: 'utf8', timeout: 300_000, killSignal: 'SIGKILL' });
  if (result.error) throw new Error(`${command} ${args.join(' ')} could not finish: ${result.error.message}`);
  if (result.signal) throw new Error(`${command} ${args.join(' ')} terminated by ${result.signal}`);
  return result;
}

function run(command, args, cwd) {
  const result = execute(command, args, cwd);
  if (result.status !== 0) throw new Error(`${command} ${args.join(' ')} failed\n${result.stdout}\n${result.stderr}`);
  return result.stdout;
}

function consumer(name, source, dependencies = dependency) {
  const project = join(dir, name);
  mkdirSync(join(project, 'src'), { recursive: true });
  writeFileSync(join(project, 'Cargo.toml'), `[package]\nname = "${name}"\nversion = "0.0.0"\nedition = "2021"\n\n[dependencies]\n${dependencies}\n`);
  writeFileSync(join(project, 'src/main.rs'), source);
  return project;
}

try {
  const publicConsumer = consumer('open_quartz_public_check', `
use open_quartz::{DataType, Environment, Graph, OpenQuartz, PlayerState, ProjectFile, PROJECT_FILE_VERSION};
fn main() {
    let sdk = OpenQuartz::new(Environment::headless());
    let project = sdk.create_project("public");
    let _: ProjectFile = project.to_file();
    let _: Graph = project.graph_snapshot();
    let _: DataType = DataType::Float;
    let _: &str = PROJECT_FILE_VERSION;
    let _: PlayerState = sdk.player(project.graph()).build().unwrap().state();
}
`);
  run('cargo', ['check', '--quiet', '--manifest-path', join(publicConsumer, 'Cargo.toml')], root);

  const heavy = ['wgpu', 'ort', 'ffmpeg-next', 'reqwest', 'oq-media', 'objc2', 'windows', 'wasm-bindgen', 'web-sys', 'js-sys'];
  const tree = run('cargo', ['tree', '--quiet', '--manifest-path', join(publicConsumer, 'Cargo.toml'), '-e', 'normal,build', '--prefix', 'none'], root);
  for (const dependencyName of heavy) {
    if (tree.split('\n').some((line) => line.startsWith(`${dependencyName} `) || line === dependencyName)) {
      throw new Error(`Default open_quartz dependency closure unexpectedly contains ${dependencyName}`);
    }
  }

  const host = run('rustc', ['-vV'], root).match(/^host: (.+)$/m)?.[1];
  if (!host) throw new Error('Cannot determine the Rust host target');
  const featureConsumers = [
    {
      feature: 'native-runtime',
      forbidden: ['ort', 'ffmpeg-next', 'oq-media', 'reqwest'],
      source: `
use open_quartz::execution::runtime::ExecutionRuntime;
fn main() {
    let _ = ExecutionRuntime::from_shared_wgpu;
}
`,
    },
    {
      feature: 'native-video',
      forbidden: ['ort', 'reqwest', 'wgpu', 'objc2-metal'],
      source: `
use open_quartz::media::native::NativeVideoSource;
fn main() {
    let _: Option<NativeVideoSource> = None;
}
`,
    },
    {
      feature: 'native-onnx',
      forbidden: ['wgpu', 'reqwest'],
      source: `
use open_quartz::inference::OnnxSession;
fn main() {
    let _ = OnnxSession::from_memory(&[]);
}
`,
    },
    {
      feature: 'model-cache',
      forbidden: ['ort', 'wgpu'],
      source: `
use open_quartz::resources::models::ModelStore;
fn main() {
    let _ = ModelStore::new(std::path::PathBuf::from("models"));
}
`,
    },
  ];
  for (const { feature, forbidden, source } of featureConsumers) {
    const dependencies = dependency.replace('default-features = false', `default-features = false, features = [${JSON.stringify(feature)}]`);
    const project = consumer(`open_quartz_${feature.replaceAll('-', '_')}_check`, source, dependencies);
    const manifest = join(project, 'Cargo.toml');
    run('cargo', ['check', '--quiet', '--manifest-path', manifest, '--target', host], root);
    const featureTree = run('cargo', ['tree', '--quiet', '--manifest-path', manifest, '--target', host, '-e', 'normal,build', '--prefix', 'none'], root);
    for (const dependencyName of forbidden) {
      if (featureTree.split('\n').some((line) => line.startsWith(`${dependencyName} `) || line === dependencyName)) {
        throw new Error(`${feature} open_quartz dependency closure unexpectedly contains ${dependencyName}`);
      }
    }
  }

  const privateModel = consumer('open_quartz_private_model_check', `
use open_quartz::project::model::Graph;
fn main() { let _ = core::any::type_name::<Graph>(); }
`);
  const privatePlayer = consumer('open_quartz_private_player_check', `
use open_quartz::player::playback::Player;
fn main() { let _ = core::any::type_name::<Player>(); }
`);
  for (const [project, diagnostic] of [
    [privateModel, /error\[E0603\]: module `model` is private/],
    [privatePlayer, /error\[E0603\]: module `playback` is private/],
  ]) {
    const result = execute('cargo', ['check', '--quiet', '--manifest-path', join(project, 'Cargo.toml')], root);
    if (result.status === 0) throw new Error(`Internal module unexpectedly public: ${project}`);
    if (!diagnostic.test(result.stderr)) {
      throw new Error(`Boundary fixture did not fail for the expected reason: ${project}\n${result.stdout}\n${result.stderr}`);
    }
  }

  const metadata = JSON.parse(run('cargo', ['metadata', '--quiet', '--no-deps', '--format-version', '1'], root));
  const members = metadata.packages.map((pkg) => pkg.name);
  if (!members.includes('open_quartz')) throw new Error('open_quartz core package is missing');
  for (const obsolete of ['open_quartz_schema', 'open_quartz_execution', 'open_quartz_sdk']) {
    if (members.includes(obsolete)) throw new Error(`Obsolete core package remains in workspace: ${obsolete}`);
  }
  const manifests = [
    'Cargo.toml', 'apps/open_quartz_slint/Cargo.toml', 'crates/slint_code_editor/Cargo.toml',
    'crates/open_quartz/Cargo.toml',
    'crates/open_quartz_screensaver/Cargo.toml', 'crates/open-quartz-screensaver-stub/Cargo.toml',
  ];
  for (const manifest of manifests) {
    const content = readFileSync(join(root, manifest), 'utf8');
    for (const obsolete of ['open_quartz_schema', 'open_quartz_execution', 'open_quartz_sdk']) {
      if (content.includes(obsolete)) throw new Error(`${manifest} still references ${obsolete}`);
    }
  }
  console.log('Single-core API, default, and feature dependency boundaries passed');
} finally {
  rmSync(dir, { recursive: true, force: true });
}
