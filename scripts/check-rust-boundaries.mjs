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
use open_quartz::compiler::{shader::parse_shader, ExecutionPlan};
use open_quartz::evaluation::{FrameInputs, GraphEvaluator};
use open_quartz::{DataType, Environment, Graph, NodeFactoryRequest, OpenQuartz, PlayerState, ProjectFile, PROJECT_FILE_VERSION};
fn main() {
    let sdk = OpenQuartz::new(Environment::headless());
    let mut project = sdk.create_project("public");
    let (renderer, _) = project.create_graph_node(
        NodeFactoryRequest::Renderer { position: None }, project.graph_revision(),
    ).unwrap();
    let _: ProjectFile = project.to_file();
    let _: Graph = project.graph_snapshot();
    let _: DataType = DataType::Float;
    let _: &str = PROJECT_FILE_VERSION;
    let _: PlayerState = sdk.player(project.graph()).build().unwrap().state();
    let parsed = parse_shader("@fragment fn main() -> @location(0) vec4f { return vec4f(1.0); }");
    assert!(parsed.parse_error.is_none());
    assert_eq!(parsed.outputs.len(), 1);
    let graph = project.graph_snapshot();
    let mut evaluator = GraphEvaluator::prepare(graph.nodes, graph.edges);
    let plan: &ExecutionPlan = evaluator.plan();
    assert!(plan.nodes.iter().any(|node| node.id == renderer.id));
    let frame = evaluator.run_frame(&FrameInputs {
        time: 0.0,
        delta: 0.0,
        frame: 1,
        date: [0.0; 4],
        mouse: [0.0; 4],
        resolution: [512.0, 512.0, 1.0],
        video_nodes: Vec::new(),
    });
    assert_eq!(frame.frame, 1);
    assert!(frame.commands.iter().any(|command| command.node_id == renderer.id && command.kind == "renderer"));
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
use open_quartz::NativePlayer;
fn main() {
    let _ = NativePlayer::from_shared_wgpu;
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
use open_quartz::inference::InferenceSession;
fn main() {
    let _ = InferenceSession::from_memory(&[]);
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

  const inferenceDependency = `oq-inference = { path = ${JSON.stringify(join(root, 'crates/oq-inference'))}, default-features = false }`;
  const inferenceConsumer = consumer('oq_inference_contract_check', `
use oq_inference::ExecutionPolicy;
fn main() { let _ = ExecutionPolicy::default(); }
`, inferenceDependency);
  const inferenceManifest = join(inferenceConsumer, 'Cargo.toml');
  run('cargo', ['check', '--quiet', '--manifest-path', inferenceManifest], root);
  const inferenceTree = run('cargo', ['tree', '--quiet', '--manifest-path', inferenceManifest, '-e', 'normal,build', '--prefix', 'none'], root);
  for (const forbidden of ['open_quartz', 'ort', 'wgpu', 'oq-media', 'reqwest', 'slint']) {
    if (inferenceTree.split('\n').some((line) => line.startsWith(`${forbidden} `) || line === forbidden)) {
      throw new Error(`Inference contracts unexpectedly depend on ${forbidden}`);
    }
  }
  const ortConsumer = consumer('oq_inference_ort_check', `
use oq_inference::InferenceSession;
fn main() { let _ = InferenceSession::from_memory(&[]); }
`, inferenceDependency.replace('default-features = false', 'default-features = false, features = ["ort"]'));
  const ortManifest = join(ortConsumer, 'Cargo.toml');
  run('cargo', ['check', '--quiet', '--manifest-path', ortManifest, '--target', host], root);
  const ortTree = run('cargo', ['tree', '--quiet', '--manifest-path', ortManifest, '--target', host, '-e', 'normal,build', '--prefix', 'none'], root);
  for (const forbidden of ['open_quartz', 'wgpu', 'oq-media', 'reqwest', 'slint']) {
    if (ortTree.split('\n').some((line) => line.startsWith(`${forbidden} `) || line === forbidden)) {
      throw new Error(`Independent ORT backend unexpectedly depends on ${forbidden}`);
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
  for (const required of ['open_quartz', 'oq-inference']) {
    if (!members.includes(required)) throw new Error(`${required} implementation package is missing`);
  }
  for (const obsolete of ['open_quartz_schema', 'open_quartz_execution', 'open_quartz_sdk']) {
    if (members.includes(obsolete)) throw new Error(`Obsolete core package remains in workspace: ${obsolete}`);
  }
  const manifests = [
    'Cargo.toml', 'apps/open_quartz_slint/Cargo.toml', 'crates/slint_code_editor/Cargo.toml',
    'crates/open_quartz/Cargo.toml', 'crates/oq-inference/Cargo.toml',
    'crates/open_quartz_screensaver/Cargo.toml', 'crates/open-quartz-screensaver-stub/Cargo.toml',
  ];
  for (const manifest of manifests) {
    const content = readFileSync(join(root, manifest), 'utf8');
    for (const obsolete of ['open_quartz_schema', 'open_quartz_execution', 'open_quartz_sdk']) {
      if (content.includes(obsolete)) throw new Error(`${manifest} still references ${obsolete}`);
    }
  }
  console.log('Core API, feature boundaries, and independent inference contracts/backend passed');
} finally {
  rmSync(dir, { recursive: true, force: true });
}
