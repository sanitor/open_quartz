use open_quartz_screensaver::{read_manifest, ScreenSaverExposedInput, ScreenSaverManifest};
#[cfg(windows)]
use open_quartz_screensaver::{
    ScreenSaverExportRequest as ExportRequest, ScreenSaverModelFiles, ScreenSaverRuntimeFiles,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, State};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenSaverExportRequest {
    output_path: String,
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
        tauri::async_runtime::spawn_blocking(move || export_with_app_paths(&app, request))
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
        if argument == "/c"
            || argument == "-c"
            || argument.starts_with("/c:")
            || argument.starts_with("-c:")
        {
            return ScreenSaverLaunchMode::Configure;
        }
        if argument == "/s" || argument == "-s" {
            return ScreenSaverLaunchMode::Run;
        }
        index += 1;
    }
    ScreenSaverLaunchMode::Run
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
fn export_with_app_paths(app: &AppHandle, request: ScreenSaverExportRequest) -> Result<(), String> {
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
    let model_cache_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Cannot resolve app data dir: {error}"))?
        .join("models");
    open_quartz_screensaver::export_windows(ExportRequest {
        output_path: PathBuf::from(request.output_path),
        name: request.name,
        project_json: request.project_json,
        renderer_node_id: request.renderer_node_id,
        exposed_inputs: request.exposed_inputs,
        stub_path: screen_saver_stub(app)?,
        runtime_files: ScreenSaverRuntimeFiles {
            ffmpeg: runtime_file("ffmpeg.exe"),
            onnxruntime: runtime_file("onnxruntime.dll"),
            directml: runtime_file("DirectML.dll"),
        },
        model_files: ScreenSaverModelFiles {
            cache_dir: model_cache_dir,
            resolved: HashMap::new(),
        },
    })
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
    use open_quartz_screensaver::MAGIC;

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
}
