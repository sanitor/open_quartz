import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';

const root = resolve('.');
const dir = mkdtempSync(join(tmpdir(), 'open-quartz-boundary-'));

function cargoCheck(source, expectSuccess, dependencies = `
open_quartz = { path = ${JSON.stringify(join(root, 'crates/open_quartz'))}, default-features = false }
`) {
  mkdirSync(join(dir, 'src'), { recursive: true });
  writeFileSync(join(dir, 'Cargo.toml'), `
[package]
name = "open_quartz_boundary_check"
version = "0.0.0"
edition = "2021"

[dependencies]
${dependencies}
`);
  writeFileSync(join(dir, 'src/main.rs'), source);
  const result = spawnSync('cargo', ['check', '--quiet', '--manifest-path', join(dir, 'Cargo.toml')], {
    cwd: dir,
    encoding: 'utf8',
  });
  const ok = result.status === 0;
  if (ok !== expectSuccess) {
    const expectation = expectSuccess ? 'succeed' : 'fail';
    throw new Error(`Expected boundary compile to ${expectation}\nSTDOUT:\n${result.stdout}\nSTDERR:\n${result.stderr}`);
  }
}

function cargoMetadata(packageName) {
  const result = spawnSync('cargo', ['metadata', '--quiet', '--no-deps', '--format-version', '1'], {
    cwd: root,
    encoding: 'utf8',
  });
  if (result.status !== 0) {
    throw new Error(`cargo metadata failed\nSTDOUT:\n${result.stdout}\nSTDERR:\n${result.stderr}`);
  }
  const metadata = JSON.parse(result.stdout);
  const pkg = metadata.packages.find((candidate) => candidate.name === packageName);
  if (!pkg) throw new Error(`Package ${packageName} not found in workspace metadata`);
  return pkg.dependencies.map((dependency) => dependency.name).sort();
}

try {
  cargoCheck(`
use open_quartz::{DataType, Environment, OpenQuartz, ProjectFile, PROJECT_FILE_VERSION};

fn main() {
    let sdk = OpenQuartz::new(Environment::headless());
    let project = sdk.create_project("public");
    let _: ProjectFile = project.to_file();
    let _: DataType = DataType::Float;
    let _: &str = PROJECT_FILE_VERSION;
}
`, true);

  const forbidden = [
    ['engine', 'ExecutionEngine'],
    ['gpu', 'GpuExecutor'],
    ['runtime', 'Runtime'],
    ['event', 'EngineState'],
    ['host', 'PlayerHost'],
    ['onnx', 'OnnxSession'],
    ['wgsl', 'CompileRequest'],
    ['graph', 'GraphRequest'],
    ['types', 'Graph'],
  ];
  for (const [moduleName, itemName] of forbidden) {
    cargoCheck(`
use open_quartz::${moduleName}::${itemName};

fn main() {
    let _ = core::any::type_name::<${itemName}>();
}
`, false);
  }

  const schemaDependency = `
open_quartz_schema = { path = ${JSON.stringify(join(root, 'crates/open_quartz_schema'))} }
`;
  cargoCheck(`
use open_quartz_schema::{DataType, NodeData, ProjectFile, PROJECT_FILE_VERSION};

fn main() {
    let _: DataType = DataType::Float;
    let _: NodeData = NodeData::default();
    let _: Option<ProjectFile> = None;
    let _: &str = PROJECT_FILE_VERSION;
}
`, true, schemaDependency);
  cargoCheck(`
use open_quartz_schema::OnnxStatus;

fn main() {
    let _ = core::any::type_name::<OnnxStatus>();
}
`, false, schemaDependency);
  cargoCheck(`
use open_quartz_schema::{NodeData, NodeType};

fn main() {
    let _ = NodeData {
        node_type: NodeType::Shader,
        label: String::new(),
        shader_code: String::new(),
        inputs: vec![],
        outputs: vec![],
        uniforms: serde_json::Map::new(),
        collapsed: None,
        ..NodeData::default()
    };
}
`, false, `${schemaDependency}serde_json = "1.0"\n`);

  const sdkDependency = `
open_quartz_sdk = { path = ${JSON.stringify(join(root, 'crates/open_quartz_sdk'))} }
`;
  cargoCheck(`
use open_quartz_sdk::{Environment, OpenQuartz, PlayerState, ProjectFile};

fn main() {
    let sdk = OpenQuartz::new(Environment::headless());
    let project = sdk.create_project("sdk");
    let _: ProjectFile = project.to_file();
    let _: PlayerState = sdk.player(project.graph()).build().unwrap().state();
}
`, true, sdkDependency);
  cargoCheck(`
use open_quartz_execution::runtime::Runtime;

fn main() {
    let _ = core::any::type_name::<Runtime>();
}
`, false, sdkDependency);

  const hostDependency = `
open_quartz_host_api = { path = ${JSON.stringify(join(root, 'crates/open_quartz_host_api'))} }
serde_json = "1.0"
`;
  cargoCheck(`
use open_quartz_host_api::{plan_host_resource_intents, HostResourceIntentRequest};

fn main() {
    let request: HostResourceIntentRequest = serde_json::from_value(serde_json::json!({
        "host": "browser",
        "graph": { "nodes": [], "edges": [] }
    })).unwrap();
    let _ = plan_host_resource_intents(request).unwrap();
}
`, true, hostDependency);

  const openQuartzManifest = readFileSync(join(root, 'crates/open_quartz/Cargo.toml'), 'utf8');
  if (openQuartzManifest.includes('host-internals')) {
    throw new Error('open_quartz still declares the obsolete host-internals pseudo-boundary');
  }
  const sdkDeps = cargoMetadata('open_quartz_sdk');
  for (const required of ['open_quartz_execution', 'open_quartz_host_api', 'open_quartz_schema']) {
    if (!sdkDeps.includes(required)) throw new Error(`open_quartz_sdk is missing ${required}`);
  }
  const executionDeps = cargoMetadata('open_quartz_execution');
  for (const forbidden of ['open_quartz', 'open_quartz_sdk']) {
    if (executionDeps.includes(forbidden)) {
      throw new Error(`open_quartz_execution must not depend on ${forbidden}`);
    }
  }
  console.log('Rust dependency boundaries passed');
} finally {
  rmSync(dir, { recursive: true, force: true });
}
