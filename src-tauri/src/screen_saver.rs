use open_quartz_schema::Graph;
#[cfg(windows)]
use open_quartz_schema::{InputMode, NodeType};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
#[cfg(windows)]
use std::io::Write;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager, State};

const MAGIC: &[u8; 16] = b"OQSCRPKG00000003";
const FOOTER_LEN: u64 = 24;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenSaverExportRequest {
    output_path: String,
    name: String,
    project_json: String,
    renderer_node_id: String,
    exposed_inputs: Vec<ScreenSaverExposedInput>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenSaverExposedInput {
    node_id: String,
    label: String,
    kind: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScreenSaverResource {
    node_id: Option<String>,
    kind: String,
    file_name: String,
    offset: u64,
    length: u64,
    task: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScreenSaverManifest {
    version: u32,
    export_id: String,
    name: String,
    project_json: String,
    renderer_node_id: String,
    exposed_inputs: Vec<ScreenSaverExposedInput>,
    resources: Vec<ScreenSaverResource>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenSaverBootstrap {
    mode: String,
    project_json: String,
    renderer_node_id: String,
    exposed_inputs: Vec<ScreenSaverExposedInput>,
    settings: HashMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct PreparedScreenSaver {
    settings_root: PathBuf,
    mode: ScreenSaverLaunchMode,
    manifest: ScreenSaverManifest,
}

#[derive(Clone, Debug)]
enum ScreenSaverLaunchMode {
    Run,
    Preview(isize),
    Configure,
}

#[derive(Default)]
pub struct ScreenSaverState(Option<PreparedScreenSaver>);

pub fn prepare() -> Option<PreparedScreenSaver> {
    let args: Vec<_> = std::env::args_os().collect();
    let marker = args
        .iter()
        .position(|arg| arg == "--open-quartz-screen-saver-package")?;
    let package_path = PathBuf::from(args.get(marker + 1)?);
    prepare_package(
        &package_path,
        args.iter()
            .skip(marker + 2)
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect(),
    )
    .ok()
}

pub fn prepare_package(
    package_path: &Path,
    args: Vec<String>,
) -> Result<PreparedScreenSaver, String> {
    let manifest = read_manifest(package_path)?;
    if manifest.version != 3 {
        return Err(format!(
            "Unsupported OpenQuartz screen saver version {}",
            manifest.version
        ));
    }
    let mode = parse_mode(args);
    let settings_root = settings_root(&manifest.export_id)?;
    Ok(PreparedScreenSaver {
        settings_root,
        mode,
        manifest,
    })
}

pub fn state(prepared: Option<PreparedScreenSaver>) -> ScreenSaverState {
    ScreenSaverState(prepared)
}

#[tauri::command]
pub fn screen_saver_bootstrap(
    state: State<'_, ScreenSaverState>,
) -> Result<Option<ScreenSaverBootstrap>, String> {
    let Some(prepared) = &state.0 else {
        return Ok(None);
    };
    Ok(Some(ScreenSaverBootstrap {
        mode: match prepared.mode {
            ScreenSaverLaunchMode::Run => "run",
            ScreenSaverLaunchMode::Preview(_) => "preview",
            ScreenSaverLaunchMode::Configure => "configure",
        }
        .to_owned(),
        project_json: prepared.manifest.project_json.clone(),
        renderer_node_id: prepared.manifest.renderer_node_id.clone(),
        exposed_inputs: prepared.manifest.exposed_inputs.clone(),
        settings: read_settings(&prepared.settings_root)?,
    }))
}

#[tauri::command]
pub async fn screen_saver_read_file(path: String) -> Result<Vec<u8>, String> {
    tauri::async_runtime::spawn_blocking(move || fs::read(path).map_err(|error| error.to_string()))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn screen_saver_exit(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        window.close().map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn screen_saver_export(
    app: AppHandle,
    request: ScreenSaverExportRequest,
) -> Result<(), String> {
    #[cfg(not(windows))]
    {
        let _ = app;
        let _ = request;
        return Err("Windows screen saver export is only available on Windows".to_owned());
    }
    #[cfg(windows)]
    {
        tauri::async_runtime::spawn_blocking(move || export_windows(&app, request))
            .await
            .map_err(|error| format!("Screen saver export worker failed: {error}"))?
    }
}

pub fn configure_window(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let state = app.state::<ScreenSaverState>();
    let Some(prepared) = &state.0 else {
        return Ok(());
    };
    let Some(window) = app.get_webview_window("main") else {
        return Ok(());
    };
    match prepared.mode {
        ScreenSaverLaunchMode::Run => {
            window.set_decorations(false)?;
            window.set_always_on_top(true)?;
            window.set_skip_taskbar(true)?;
            window.set_cursor_visible(false)?;
            window.set_fullscreen(true)?;
        }
        ScreenSaverLaunchMode::Preview(parent) => {
            window.set_decorations(false)?;
            window.set_skip_taskbar(true)?;
            #[cfg(not(windows))]
            let _ = parent;
            #[cfg(windows)]
            attach_preview_window(&window, parent)?;
        }
        ScreenSaverLaunchMode::Configure => {}
    }
    Ok(())
}

fn parse_mode(args: Vec<String>) -> ScreenSaverLaunchMode {
    let mut index = 0;
    while index < args.len() {
        let argument = args[index].to_ascii_lowercase();
        if argument == "/p" || argument == "-p" {
            if let Some(parent) = args
                .get(index + 1)
                .and_then(|value| value.parse::<isize>().ok())
            {
                return ScreenSaverLaunchMode::Preview(parent);
            }
        }
        if let Some(parent) = argument
            .strip_prefix("/p:")
            .or_else(|| argument.strip_prefix("-p:"))
            .and_then(|value| value.parse::<isize>().ok())
        {
            return ScreenSaverLaunchMode::Preview(parent);
        }
        if argument == "/c" || argument == "-c" || argument.starts_with("/c:") || argument.starts_with("-c:") {
            return ScreenSaverLaunchMode::Configure;
        }
        if argument == "/s" || argument == "-s" {
            return ScreenSaverLaunchMode::Run;
        }
        index += 1;
    }
    ScreenSaverLaunchMode::Run
}

fn read_manifest(package_path: &Path) -> Result<ScreenSaverManifest, String> {
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

fn settings_root(export_id: &str) -> Result<PathBuf, String> {
    if export_id.is_empty()
        || !export_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err("Invalid screen saver export ID".to_owned());
    }
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    Ok(base.join("OpenQuartz").join("ScreenSavers").join(export_id))
}

fn read_settings(root: &Path) -> Result<HashMap<String, String>, String> {
    let path = root.join("settings.json");
    if !path.is_file() {
        return Ok(HashMap::new());
    }
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

#[cfg(windows)]
fn export_windows(app: &AppHandle, request: ScreenSaverExportRequest) -> Result<(), String> {
    let (graph, project_json) = collect_export_project(
        &request.project_json,
        &request.renderer_node_id,
    )?;
    validate_shader_sources(&graph)?;
    let output = PathBuf::from(&request.output_path);
    if output
        .extension()
        .and_then(|value| value.to_str())
        .is_none_or(|extension| !extension.eq_ignore_ascii_case("scr"))
    {
        return Err("Screen saver output must use the .scr extension".to_owned());
    }
    if let Some(parent) = output.parent() {
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
    let stub = screen_saver_stub(app)?;
    let resources = collect_resources(app, &graph)?;
    let manifest = ScreenSaverManifest {
        version: 3,
        export_id,
        name: request.name,
        project_json,
        renderer_node_id: request.renderer_node_id,
        exposed_inputs: request.exposed_inputs,
        resources: Vec::new(),
    };
    write_package(&stub, &output, manifest, &resources)
}

fn parse_export_graph(project_json: &str) -> Result<Graph, String> {
    let value: serde_json::Value = serde_json::from_str(project_json)
        .map_err(|error| format!("Invalid screen saver project JSON: {error}"))?;
    let graph_value = value.get("graph").unwrap_or(&value);
    serde_json::from_value(graph_value.clone())
        .map_err(|error| format!("Invalid screen saver graph: {error}"))
}

fn collect_export_project(project_json: &str, renderer_node_id: &str) -> Result<(Graph, String), String> {
    let sdk = open_quartz::OpenQuartz::new(open_quartz::Environment::headless());
    let normalized = sdk
        .screen_saver_export_project_json(project_json, renderer_node_id)
        .map_err(|error| error.to_string())?;
    let graph = parse_export_graph(&normalized)?;
    Ok((graph, normalized))
}

#[cfg(windows)]
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

#[cfg(windows)]
fn collect_resources(
    app: &AppHandle,
    graph: &Graph,
) -> Result<Vec<(ScreenSaverResource, PathBuf)>, String> {
    let runtime_dir = app
        .path()
        .resource_dir()
        .map_err(|error| format!("Cannot resolve runtime resources: {error}"))?
        .join("runtime");
    let source_runtime_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("runtime");
    let runtime_file = |name: &str| {
        let bundled = runtime_dir.join(name);
        if bundled.is_file() {
            bundled
        } else {
            source_runtime_dir.join(name)
        }
    };
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
            let model_path = node
                .data
                .onnx_custom_path
                .as_deref()
                .map(PathBuf::from)
                .unwrap_or(crate::models_dir(app)?.join(format!("{model_id}.onnx")));
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
        push_resource(
            &mut resources,
            None,
            "runtime",
            &runtime_file("ffmpeg.exe"),
            None,
        )?;
    }
    if needs_onnx {
        for name in ["onnxruntime.dll", "DirectML.dll"] {
            push_resource(&mut resources, None, "runtime", &runtime_file(name), None)?;
        }
    }
    Ok(resources)
}

#[cfg(windows)]
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

#[cfg(windows)]
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

#[cfg(windows)]

fn write_package(
    stub: &Path,
    output: &Path,
    mut manifest: ScreenSaverManifest,
    resources: &[(ScreenSaverResource, PathBuf)],
) -> Result<(), String> {
    let temporary = output.with_extension("scr.tmp");
    let mut destination = File::create(&temporary).map_err(|error| error.to_string())?;
    let mut stub_file = File::open(stub).map_err(|error| error.to_string())?;
    std::io::copy(&mut stub_file, &mut destination).map_err(|error| error.to_string())?;
    for (resource, path) in resources {
        let offset = destination
            .stream_position()
            .map_err(|error| error.to_string())?;
        let mut source = File::open(path).map_err(|error| error.to_string())?;
        let copied =
            std::io::copy(&mut source, &mut destination).map_err(|error| error.to_string())?;
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
    drop(destination);
    if output.exists() {
        fs::remove_file(output).map_err(|error| error.to_string())?;
    }
    fs::rename(&temporary, output).map_err(|error| error.to_string())
}

#[cfg(windows)]
fn screen_saver_stub(app: &AppHandle) -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join("runtime/open-quartz-screensaver-stub.exe"));
    }
    candidates.push(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("runtime/open-quartz-screensaver-stub.exe"),
    );
    candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            "Screen saver launcher is unavailable; run npm run build:screensaver-stub".to_owned()
        })
}

#[cfg(windows)]
fn attach_preview_window(
    window: &tauri::WebviewWindow,
    parent: isize,
) -> Result<(), Box<dyn std::error::Error>> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClientRect, SetParent, SetWindowLongPtrW, SetWindowPos, GWL_STYLE, SWP_NOACTIVATE,
        SWP_NOZORDER, SWP_SHOWWINDOW, WS_CHILD, WS_VISIBLE,
    };
    let child = window.hwnd()?;
    let parent = HWND(parent as *mut _);
    let mut rect = RECT::default();
    unsafe {
        GetClientRect(parent, &mut rect)?;
        let _ = SetParent(child, Some(parent));
        SetWindowLongPtrW(child, GWL_STYLE, (WS_CHILD.0 | WS_VISIBLE.0) as isize);
        SetWindowPos(
            child,
            None,
            0,
            0,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOACTIVATE | SWP_NOZORDER | SWP_SHOWWINDOW,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_windows_preview_mode() {
        assert!(matches!(
            parse_mode(vec!["/s".into()]),
            ScreenSaverLaunchMode::Run
        ));
        assert!(matches!(
            parse_mode(vec!["/p".into(), "42".into()]),
            ScreenSaverLaunchMode::Preview(42)
        ));
        assert!(matches!(
            parse_mode(vec!["/P:73".into()]),
            ScreenSaverLaunchMode::Preview(73)
        ));
        assert!(matches!(
            parse_mode(vec!["/c:19".into()]),
            ScreenSaverLaunchMode::Configure
        ));
    }

    #[test]
    fn rejects_unsafe_export_ids() {
        assert!(settings_root("safe-id_1").is_ok());
        assert!(settings_root("../unsafe").is_err());
    }

    #[test]
    fn rejects_unsupported_manifest_versions() {
        let root = std::env::temp_dir().join(format!(
            "open-quartz-screen-saver-version-test-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let package = root.join("old.scr");
        let manifest = serde_json::json!({
            "version": 2,
            "exportId": "test-export",
            "name": "Test",
            "projectJson": "{\"version\":\"0.4.0\",\"name\":\"Test\",\"createdAt\":\"\",\"updatedAt\":\"\",\"graph\":{\"nodes\":[],\"edges\":[]}}",
            "rendererNodeId": "renderer",
            "exposedInputs": [],
            "resources": []
        });
        let manifest = serde_json::to_vec(&manifest).unwrap();
        fs::write(
            &package,
            [
                b"stub".as_slice(),
                manifest.as_slice(),
                &(manifest.len() as u64).to_le_bytes(),
                MAGIC,
            ]
            .concat(),
        )
        .unwrap();

        let error = prepare_package(&package, vec![]).unwrap_err();

        assert!(error.contains("Unsupported OpenQuartz screen saver version 2"));
        fs::remove_dir_all(root).unwrap();
    }

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
    }

    #[cfg(windows)]
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

    #[cfg(windows)]
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
}
