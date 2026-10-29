use open_quartz_schema::{Graph, InputMode, NodeType};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAGIC: &[u8; 16] = b"OQSCRPKG00000003";
const FOOTER_LEN: u64 = 24;

/// All host-resolved inputs needed to export a Windows screen saver package.
#[derive(Clone, Debug)]
pub struct ScreenSaverExportRequest {
    pub output_path: PathBuf,
    pub name: String,
    pub project_json: String,
    pub renderer_node_id: String,
    pub exposed_inputs: Vec<ScreenSaverExposedInput>,
    pub stub_path: PathBuf,
    pub runtime_files: ScreenSaverRuntimeFiles,
    pub model_files: ScreenSaverModelFiles,
}

/// Runtime paths are checked only when the selected renderer requires them.
#[derive(Clone, Debug)]
pub struct ScreenSaverRuntimeFiles {
    pub ffmpeg: PathBuf,
    pub onnxruntime: PathBuf,
    pub directml: PathBuf,
}

/// Resolved model files are keyed by node ID and take precedence over project
/// custom paths. Remaining models use `<cache_dir>/<model_id>.onnx`.
#[derive(Clone, Debug)]
pub struct ScreenSaverModelFiles {
    pub cache_dir: PathBuf,
    pub resolved: HashMap<String, PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenSaverExposedInput {
    pub node_id: String,
    pub label: String,
    pub kind: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenSaverResource {
    pub node_id: Option<String>,
    pub kind: String,
    pub file_name: String,
    pub offset: u64,
    pub length: u64,
    pub task: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenSaverManifest {
    pub version: u32,
    pub export_id: String,
    pub name: String,
    pub project_json: String,
    pub renderer_node_id: String,
    pub exposed_inputs: Vec<ScreenSaverExposedInput>,
    pub resources: Vec<ScreenSaverResource>,
}

/// Normalize and validate the selected renderer graph, then write a self-contained
/// Windows `.scr` package. Callers can run this blocking operation on a worker.
pub fn export_windows(request: ScreenSaverExportRequest) -> Result<(), String> {
    let (graph, project_json) =
        collect_export_project(&request.project_json, &request.renderer_node_id)?;
    validate_shader_sources(&graph)?;
    let output = &request.output_path;
    if output
        .extension()
        .and_then(|value| value.to_str())
        .map_or(true, |extension| !extension.eq_ignore_ascii_case("scr"))
    {
        return Err("Screen saver output must use the .scr extension".to_owned());
    }
    if let Some(parent) = output.parent().filter(|path| !path.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let export_id = format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos(),
    );
    let resources = collect_resources(&graph, &request.runtime_files, &request.model_files)?;
    let manifest = ScreenSaverManifest {
        version: 3,
        export_id,
        name: request.name,
        project_json,
        renderer_node_id: request.renderer_node_id,
        exposed_inputs: request.exposed_inputs,
        resources: Vec::new(),
    };
    write_package(&request.stub_path, output, manifest, &resources)
}

fn parse_export_graph(project_json: &str) -> Result<Graph, String> {
    let value: serde_json::Value = serde_json::from_str(project_json)
        .map_err(|error| format!("Invalid screen saver project JSON: {error}"))?;
    let graph_value = value.get("graph").unwrap_or(&value);
    serde_json::from_value(graph_value.clone())
        .map_err(|error| format!("Invalid screen saver graph: {error}"))
}

fn collect_export_project(
    project_json: &str,
    renderer_node_id: &str,
) -> Result<(Graph, String), String> {
    let sdk = open_quartz::OpenQuartz::new(open_quartz::Environment::headless());
    let normalized = sdk
        .screen_saver_export_project_json(project_json, renderer_node_id)
        .map_err(|error| error.to_string())?;
    let graph = parse_export_graph(&normalized)?;
    Ok((graph, normalized))
}

fn validate_shader_sources(graph: &Graph) -> Result<(), String> {
    let missing = graph
        .nodes
        .iter()
        .filter(|node| {
            node.node_type == NodeType::Shader && node.data.shader_code.trim().is_empty()
        })
        .map(|node| node.data.label.as_str())
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Screen saver export contains shader nodes without embedded source: {}",
            missing.join(", ")
        ))
    }
}

fn collect_resources(
    graph: &Graph,
    runtime_files: &ScreenSaverRuntimeFiles,
    model_files: &ScreenSaverModelFiles,
) -> Result<Vec<(ScreenSaverResource, PathBuf)>, String> {
    let mut resources = Vec::new();
    let mut needs_ffmpeg = false;
    let mut needs_onnx = false;
    for node in &graph.nodes {
        if node.node_type == NodeType::Input && node.data.input_mode == Some(InputMode::Video) {
            needs_ffmpeg = true;
            if node.data.video_source_type == Some(open_quartz_schema::VideoSourceType::Camera) {
                return Err(format!(
                    "Camera node '{}' cannot be exported as a deterministic screen saver input",
                    node.data.label
                ));
            }
            if let Some(path) = node.data.video_file_path.as_deref() {
                push_resource(
                    &mut resources,
                    Some(&node.id),
                    "video",
                    Path::new(path),
                    None,
                )?;
            }
        }
        if node.node_type == NodeType::Onnx {
            needs_onnx = true;
            let model_id = node
                .data
                .onnx_model_id
                .as_deref()
                .or(node.data.onnx_catalog_id.as_deref())
                .ok_or_else(|| format!("ONNX node '{}' has no model ID", node.data.label))?;
            let model_path = model_files
                .resolved
                .get(&node.id)
                .cloned()
                .or_else(|| node.data.onnx_custom_path.as_deref().map(PathBuf::from))
                .unwrap_or_else(|| model_files.cache_dir.join(format!("{model_id}.onnx")));
            push_resource(
                &mut resources,
                Some(&node.id),
                "onnx-model",
                &model_path,
                Some(onnx_task(model_id)),
            )?;
        }
    }
    if needs_ffmpeg {
        push_resource(&mut resources, None, "runtime", &runtime_files.ffmpeg, None)?;
        // Shared FFmpeg distributions require their sibling codec DLLs. Static
        // FFmpeg builds have none and remain valid export inputs.
        push_runtime_libraries(
            &mut resources,
            &runtime_files.ffmpeg,
            &[
                "avcodec-",
                "avformat-",
                "avutil-",
                "avdevice-",
                "avfilter-",
                "swresample-",
                "swscale-",
            ],
        )?;
    }
    if needs_onnx {
        for path in [&runtime_files.onnxruntime, &runtime_files.directml] {
            push_resource(&mut resources, None, "runtime", path, None)?;
        }
        push_runtime_libraries(
            &mut resources,
            &runtime_files.onnxruntime,
            &["msvcp140", "vcruntime140", "concrt140"],
        )?;
    }
    Ok(resources)
}

fn push_runtime_libraries(
    resources: &mut Vec<(ScreenSaverResource, PathBuf)>,
    executable: &Path,
    prefixes: &[&str],
) -> Result<(), String> {
    let directory = executable
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut libraries = Vec::new();
    for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if name
            .rsplit_once('.')
            .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("dll"))
            && prefixes.iter().any(|prefix| {
                name.get(..prefix.len())
                    .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
            })
            && path.is_file()
        {
            libraries.push(path);
        }
    }
    libraries.sort();
    for library in libraries {
        push_resource(resources, None, "runtime", &library, None)?;
    }
    Ok(())
}

fn push_resource(
    resources: &mut Vec<(ScreenSaverResource, PathBuf)>,
    node_id: Option<&str>,
    kind: &str,
    path: &Path,
    task: Option<&str>,
) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!(
            "Required screen saver resource is missing: {}",
            path.display()
        ));
    }
    let length = fs::metadata(path).map_err(|error| error.to_string())?.len();
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("Invalid resource file name: {}", path.display()))?
        .to_owned();
    resources.push((
        ScreenSaverResource {
            node_id: node_id.map(str::to_owned),
            kind: kind.to_owned(),
            file_name,
            offset: 0,
            length,
            task: task.map(str::to_owned),
        },
        path.to_owned(),
    ));
    Ok(())
}

fn onnx_task(model_id: &str) -> &'static str {
    match model_id {
        "yolov8n" => "detection",
        "super-resolution-3x" | "realesrgan-x4" => "super-resolution",
        "u2netp" | "modnet" => "background-removal",
        "midas-small" => "depth-estimation",
        "yolo26n-sem" => "segmentation",
        _ => "generic",
    }
}

fn write_package(
    stub: &Path,
    output: &Path,
    mut manifest: ScreenSaverManifest,
    resources: &[(ScreenSaverResource, PathBuf)],
) -> Result<(), String> {
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
    let destination = temporary.as_file_mut();
    let mut stub_file = File::open(stub).map_err(|error| error.to_string())?;
    std::io::copy(&mut stub_file, &mut *destination).map_err(|error| error.to_string())?;
    for (resource, path) in resources {
        let offset = destination
            .stream_position()
            .map_err(|error| error.to_string())?;
        let mut source = File::open(path).map_err(|error| error.to_string())?;
        let copied =
            std::io::copy(&mut source, &mut *destination).map_err(|error| error.to_string())?;
        if copied != resource.length {
            return Err(format!(
                "Resource changed while exporting: {}",
                path.display()
            ));
        }
        let mut descriptor = resource.clone();
        descriptor.offset = offset;
        manifest.resources.push(descriptor);
    }
    let manifest = serde_json::to_vec(&manifest).map_err(|error| error.to_string())?;
    destination
        .write_all(&manifest)
        .map_err(|error| error.to_string())?;
    destination
        .write_all(&(manifest.len() as u64).to_le_bytes())
        .map_err(|error| error.to_string())?;
    destination
        .write_all(MAGIC)
        .map_err(|error| error.to_string())?;
    destination.sync_all().map_err(|error| error.to_string())?;
    temporary
        .persist(output)
        .map_err(|error| error.error.to_string())?;
    Ok(())
}

pub fn read_manifest(package_path: &Path) -> Result<ScreenSaverManifest, String> {
    let mut file = File::open(package_path).map_err(|error| error.to_string())?;
    let length = file.metadata().map_err(|error| error.to_string())?.len();
    if length < FOOTER_LEN {
        return Err("Invalid OpenQuartz screen saver package".to_owned());
    }
    file.seek(SeekFrom::End(-(FOOTER_LEN as i64)))
        .map_err(|error| error.to_string())?;
    let mut length_bytes = [0_u8; 8];
    file.read_exact(&mut length_bytes)
        .map_err(|error| error.to_string())?;
    let manifest_length = u64::from_le_bytes(length_bytes);
    let mut magic = [0_u8; 16];
    file.read_exact(&mut magic)
        .map_err(|error| error.to_string())?;
    if &magic != MAGIC || manifest_length > length - FOOTER_LEN {
        return Err("Invalid OpenQuartz screen saver footer".to_owned());
    }
    file.seek(SeekFrom::Start(length - FOOTER_LEN - manifest_length))
        .map_err(|error| error.to_string())?;
    let mut bytes = vec![0_u8; manifest_length as usize];
    file.read_exact(&mut bytes)
        .map_err(|error| error.to_string())?;
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_project_uses_rust_upstream_graph_contract() {
        let project_json = serde_json::json!({
            "version": "0.4.0",
            "name": "Saver",
            "createdAt": "created",
            "updatedAt": "updated",
            "graph": {
                "nodes": [
                    {
                        "id": "source",
                        "type": "shader",
                        "position": { "x": 0.0, "y": 0.0 },
                        "data": {
                            "type": "shader",
                            "label": "Source",
                            "shaderCode": "@fragment fn main() -> @location(0) vec4f { return vec4f(1.0); }",
                            "inputs": [],
                            "outputs": [{
                                "id": "out",
                                "label": "out",
                                "dataType": "sampler2D",
                                "direction": "output"
                            }],
                            "uniforms": {}
                        }
                    },
                    {
                        "id": "renderer",
                        "type": "renderer",
                        "position": { "x": 10.0, "y": 0.0 },
                        "data": {
                            "type": "renderer",
                            "label": "Renderer",
                            "shaderCode": "",
                            "inputs": [{
                                "id": "in",
                                "label": "in",
                                "dataType": "sampler2D",
                                "direction": "input"
                            }],
                            "outputs": [],
                            "uniforms": {}
                        }
                    },
                    {
                        "id": "unused",
                        "type": "shader",
                        "position": { "x": 20.0, "y": 0.0 },
                        "data": {
                            "type": "shader",
                            "label": "Unused",
                            "shaderCode": "",
                            "inputs": [],
                            "outputs": [],
                            "uniforms": {}
                        }
                    }
                ],
                "edges": [{
                    "id": "source-renderer",
                    "source": "source",
                    "sourceHandle": "out",
                    "target": "renderer",
                    "targetHandle": "in"
                }]
            }
        })
        .to_string();

        let (graph, normalized) = collect_export_project(&project_json, "renderer").unwrap();
        let normalized: serde_json::Value = serde_json::from_str(&normalized).unwrap();

        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(normalized["graph"]["nodes"][0]["id"], "source");
        assert_eq!(normalized["graph"]["nodes"][1]["id"], "renderer");
        assert_eq!(normalized["graph"]["edges"].as_array().unwrap().len(), 1);

        let root = tempfile::tempdir().unwrap();
        let stub = root.path().join("host.exe");
        let output = root.path().join("sample.scr");
        fs::write(&stub, b"native-host").unwrap();
        fs::write(&output, b"previous-output").unwrap();
        export_windows(ScreenSaverExportRequest {
            output_path: output.clone(),
            name: "Saver".to_owned(),
            project_json,
            renderer_node_id: "renderer".to_owned(),
            exposed_inputs: vec![ScreenSaverExposedInput {
                node_id: "source".to_owned(),
                label: "Source".to_owned(),
                kind: "color".to_owned(),
            }],
            stub_path: stub,
            runtime_files: ScreenSaverRuntimeFiles {
                ffmpeg: root.path().join("unused-ffmpeg.exe"),
                onnxruntime: root.path().join("unused-onnxruntime.dll"),
                directml: root.path().join("unused-DirectML.dll"),
            },
            model_files: ScreenSaverModelFiles {
                cache_dir: root.path().join("unused-models"),
                resolved: HashMap::new(),
            },
        })
        .unwrap();
        let manifest = read_manifest(&output).unwrap();
        let packaged: serde_json::Value = serde_json::from_str(&manifest.project_json).unwrap();
        assert_eq!(packaged, normalized);
        assert_eq!(manifest.renderer_node_id, "renderer");
        assert_eq!(manifest.exposed_inputs[0].node_id, "source");
        assert!(manifest.resources.is_empty());
    }

    #[test]
    fn rejects_shader_nodes_without_embedded_source() {
        let graph: Graph = serde_json::from_value(serde_json::json!({
            "nodes": [{
                "id": "hue",
                "type": "shader",
                "position": { "x": 0.0, "y": 0.0 },
                "data": {
                    "type": "shader",
                    "label": "Hue Rotate",
                    "shaderTemplateId": "Hue Rotate",
                    "shaderCode": "",
                    "inputs": [],
                    "outputs": [],
                    "uniforms": {}
                }
            }],
            "edges": []
        }))
        .unwrap();
        let error = validate_shader_sources(&graph).unwrap_err();
        assert!(error.contains("Hue Rotate"));
    }

    #[test]
    fn writes_a_self_contained_version_three_package() {
        let root = std::env::temp_dir().join(format!(
            "open-quartz-screen-saver-test-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let stub = root.join("host.exe");
        let package = root.join("sample.scr");
        fs::write(&stub, b"native-host").unwrap();
        let resource = root.join("model.onnx");
        fs::write(&resource, b"onnx-model").unwrap();
        let manifest = ScreenSaverManifest {
            version: 3,
            export_id: "test-export".to_owned(),
            name: "Test".to_owned(),
            project_json: "{\"nodes\":[],\"edges\":[]}".to_owned(),
            renderer_node_id: "renderer".to_owned(),
            exposed_inputs: Vec::new(),
            resources: Vec::new(),
        };
        let descriptor = ScreenSaverResource {
            node_id: Some("onnx".to_owned()),
            kind: "onnx-model".to_owned(),
            file_name: "model.onnx".to_owned(),
            offset: 0,
            length: 10,
            task: Some("generic".to_owned()),
        };
        write_package(&stub, &package, manifest.clone(), &[(descriptor, resource)]).unwrap();
        let decoded = read_manifest(&package).unwrap();
        assert_eq!(decoded.version, 3);
        assert_eq!(decoded.project_json, manifest.project_json);
        assert_eq!(decoded.resources.len(), 1);
        assert_eq!(decoded.resources[0].length, 10);
        let package_bytes = fs::read(&package).unwrap();
        let offset = decoded.resources[0].offset as usize;
        assert_eq!(&package_bytes[..11], b"native-host");
        assert_eq!(&package_bytes[offset..offset + 10], b"onnx-model");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn collects_explicit_runtime_and_model_files() {
        let root = tempfile::tempdir().unwrap();
        let video = root.path().join("clip.mp4");
        let model = root.path().join("resolved.onnx");
        let cache_dir = root.path().join("models");
        let runtime_files = ScreenSaverRuntimeFiles {
            ffmpeg: root.path().join("ffmpeg.exe"),
            onnxruntime: root.path().join("onnxruntime.dll"),
            directml: root.path().join("DirectML.dll"),
        };
        for path in [
            &video,
            &model,
            &runtime_files.ffmpeg,
            &runtime_files.onnxruntime,
            &runtime_files.directml,
        ] {
            fs::write(path, b"resource").unwrap();
        }
        let mut graph: Graph = serde_json::from_value(serde_json::json!({
            "nodes": [
                {
                    "id": "video",
                    "type": "input",
                    "position": { "x": 0.0, "y": 0.0 },
                    "data": {
                        "type": "input",
                        "label": "Video",
                        "shaderCode": "",
                        "inputs": [],
                        "outputs": [],
                        "uniforms": {},
                        "inputMode": "video",
                        "videoFilePath": video
                    }
                },
                {
                    "id": "model",
                    "type": "onnx",
                    "position": { "x": 0.0, "y": 0.0 },
                    "data": {
                        "type": "onnx",
                        "label": "Model",
                        "shaderCode": "",
                        "inputs": [],
                        "outputs": [],
                        "uniforms": {},
                        "onnxModelId": "yolov8n",
                        "onnxCustomPath": root.path().join("missing-custom.onnx")
                    }
                }
            ],
            "edges": []
        }))
        .unwrap();
        let mut model_files = ScreenSaverModelFiles {
            cache_dir,
            resolved: HashMap::from([("model".to_owned(), model.clone())]),
        };

        let resources = collect_resources(&graph, &runtime_files, &model_files).unwrap();

        assert_eq!(resources.len(), 5);
        assert_eq!(resources[0].1, video);
        assert_eq!(resources[1].1, model);
        assert_eq!(resources[1].0.task.as_deref(), Some("detection"));
        assert_eq!(resources[2].1, runtime_files.ffmpeg);
        assert_eq!(resources[3].1, runtime_files.onnxruntime);
        assert_eq!(resources[4].1, runtime_files.directml);

        model_files.resolved.clear();
        graph.nodes[1].data.onnx_custom_path = Some(model.to_string_lossy().into_owned());
        let resources = collect_resources(&graph, &runtime_files, &model_files).unwrap();
        assert_eq!(resources[1].1, model);

        graph.nodes[1].data.onnx_custom_path = None;
        fs::create_dir_all(&model_files.cache_dir).unwrap();
        let cached_model = model_files.cache_dir.join("yolov8n.onnx");
        fs::write(&cached_model, b"cached-model").unwrap();
        let resources = collect_resources(&graph, &runtime_files, &model_files).unwrap();
        assert_eq!(resources[1].1, cached_model);

        let codec = root.path().join("avcodec-63.dll");
        fs::write(&codec, b"codec-runtime").unwrap();
        fs::write(root.path().join("avcodec-63.lib"), b"link-only").unwrap();
        fs::write(root.path().join("unrelated.dll"), b"unrelated").unwrap();
        let crt = root.path().join("vcruntime140.dll");
        fs::write(&crt, b"vc-runtime").unwrap();
        let resources = collect_resources(&graph, &runtime_files, &model_files).unwrap();
        let bundled: Vec<_> = resources.iter().map(|(_, path)| path.clone()).collect();
        assert_eq!(
            bundled,
            vec![
                video,
                cached_model,
                runtime_files.ffmpeg.clone(),
                codec,
                runtime_files.onnxruntime.clone(),
                runtime_files.directml.clone(),
                crt,
            ]
        );

        fs::remove_file(&runtime_files.ffmpeg).unwrap();
        let error = collect_resources(&graph, &runtime_files, &model_files).unwrap_err();
        assert!(error.contains("ffmpeg.exe"));
    }

    #[test]
    fn failed_export_preserves_existing_output_and_cleans_temporary_file() {
        let root = tempfile::tempdir().unwrap();
        let stub = root.path().join("host.exe");
        let output = root.path().join("sample.scr");
        let resource = root.path().join("model.onnx");
        fs::write(&stub, b"native-host").unwrap();
        fs::write(&output, b"previous-output").unwrap();
        fs::write(&resource, b"changed").unwrap();
        let manifest = ScreenSaverManifest {
            version: 3,
            export_id: "test-export".to_owned(),
            name: "Test".to_owned(),
            project_json: "{\"nodes\":[],\"edges\":[]}".to_owned(),
            renderer_node_id: "renderer".to_owned(),
            exposed_inputs: Vec::new(),
            resources: Vec::new(),
        };
        let descriptor = ScreenSaverResource {
            node_id: Some("onnx".to_owned()),
            kind: "onnx-model".to_owned(),
            file_name: "model.onnx".to_owned(),
            offset: 0,
            length: 10,
            task: Some("generic".to_owned()),
        };

        let error = write_package(&stub, &output, manifest, &[(descriptor, resource)]).unwrap_err();

        assert!(error.contains("Resource changed while exporting"));
        assert_eq!(fs::read(&output).unwrap(), b"previous-output");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 3);
    }
}
