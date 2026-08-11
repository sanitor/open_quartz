#[cfg(windows)]
use open_quartz::types::{Graph, InputMode, NodeType};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
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
struct ScreenSaverManifest {
    version: u32,
    export_id: String,
    name: String,
    project_json: String,
    renderer_node_id: String,
    exposed_inputs: Vec<ScreenSaverExposedInput>,
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
            #[cfg(windows)]
            attach_preview_window(&window, parent)?;
        }
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
    validate_minimum_profile(&request.project_json)?;
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
    let manifest = ScreenSaverManifest {
        version: 3,
        export_id,
        name: request.name,
        project_json: request.project_json,
        renderer_node_id: request.renderer_node_id,
        exposed_inputs: request.exposed_inputs,
    };
    let manifest = serde_json::to_vec(&manifest).map_err(|error| error.to_string())?;
    write_package(&screen_saver_stub(app)?, &output, &manifest)
}

#[cfg(windows)]
fn validate_minimum_profile(project_json: &str) -> Result<(), String> {
    let value: serde_json::Value = serde_json::from_str(project_json)
        .map_err(|error| format!("Invalid screen saver project JSON: {error}"))?;
    let graph_value = value.get("graph").unwrap_or(&value);
    let graph: Graph = serde_json::from_value(graph_value.clone())
        .map_err(|error| format!("Invalid screen saver graph: {error}"))?;
    let unsupported = graph
        .nodes
        .iter()
        .filter_map(|node| {
            let capability = if node.node_type == NodeType::Onnx {
                Some("ONNX")
            } else if node.node_type == NodeType::Input
                && node.data.input_mode == Some(InputMode::Video)
            {
                Some("video")
            } else {
                None
            };
            capability.map(|capability| format!("{} ({capability})", node.data.label))
        })
        .collect::<Vec<_>>();
    if unsupported.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "This graph requires screen saver capability profiles that are not packaged yet: {}",
            unsupported.join(", ")
        ))
    }
}

fn write_package(stub: &Path, output: &Path, manifest: &[u8]) -> Result<(), String> {
    let temporary = output.with_extension("scr.tmp");
    let mut destination = File::create(&temporary).map_err(|error| error.to_string())?;
    let mut stub_file = File::open(stub).map_err(|error| error.to_string())?;
    std::io::copy(&mut stub_file, &mut destination).map_err(|error| error.to_string())?;
    destination
        .write_all(manifest)
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
    }

    #[test]
    fn rejects_unsafe_export_ids() {
        assert!(settings_root("safe-id_1").is_ok());
        assert!(settings_root("../unsafe").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn rejects_unpacked_screen_saver_capabilities_before_export() {
        let graph = serde_json::json!({
            "nodes": [{
                "id": "onnx",
                "type": "onnx",
                "position": { "x": 0.0, "y": 0.0 },
                "data": {
                    "type": "onnx", "label": "Detector", "shaderCode": "",
                    "inputs": [], "outputs": [], "uniforms": {}
                }
            }],
            "edges": []
        });
        let error = validate_minimum_profile(&graph.to_string()).unwrap_err();
        assert!(error.contains("Detector (ONNX)"));
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
        let manifest = ScreenSaverManifest {
            version: 3,
            export_id: "test-export".to_owned(),
            name: "Test".to_owned(),
            project_json: "{\"nodes\":[],\"edges\":[]}".to_owned(),
            renderer_node_id: "renderer".to_owned(),
            exposed_inputs: Vec::new(),
        };
        write_package(&stub, &package, &serde_json::to_vec(&manifest).unwrap()).unwrap();
        let decoded = read_manifest(&package).unwrap();
        assert_eq!(decoded.version, 3);
        assert_eq!(decoded.project_json, manifest.project_json);
        assert_eq!(&fs::read(&package).unwrap()[..11], b"native-host");
        fs::remove_dir_all(root).unwrap();
    }
}
