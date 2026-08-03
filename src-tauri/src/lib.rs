mod native_runtime;
mod native_video;

use futures::StreamExt;
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};

/// Models directory: `<app_data_dir>/models/`
pub(crate) fn models_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Cannot resolve app data dir: {e}"))?;
    let dir = base.join("models");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create models dir: {e}"))?;
    Ok(dir)
}

/// Progress event payload sent to the frontend.
#[derive(Clone, Serialize)]
struct DownloadProgress {
    model_id: String,
    received: u64,
    total: u64,
}

/// Download a model from `url` to `<models_dir>/<model_id>.onnx`.
/// Emits `model-download-progress` events for the frontend progress bar.
/// Returns the absolute path of the downloaded file.
#[tauri::command]
async fn download_model(
    app: AppHandle,
    model_id: String,
    url: String,
    expected_size: u64,
) -> Result<String, String> {
    let dir = models_dir(&app)?;
    let file_path = dir.join(format!("{model_id}.onnx"));

    // Already downloaded?
    if file_path.exists() {
        let meta = std::fs::metadata(&file_path).map_err(|e| format!("Cannot stat file: {e}"))?;
        if meta.len() > 0 {
            return Ok(file_path.to_string_lossy().into_owned());
        }
    }

    let response = reqwest::get(&url)
        .await
        .map_err(|e| format!("Download failed: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("HTTP {}: {}", response.status(), url));
    }

    let total = response.content_length().unwrap_or(expected_size);
    let mut stream = response.bytes_stream();
    let mut received: u64 = 0;
    let mut data = Vec::with_capacity(total as usize);

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Stream error: {e}"))?;
        received += chunk.len() as u64;
        data.extend_from_slice(&chunk);

        // Emit progress every ~100KB to avoid flooding
        if received % 102_400 < chunk.len() as u64 || received == total {
            let _ = app.emit(
                "model-download-progress",
                DownloadProgress {
                    model_id: model_id.clone(),
                    received,
                    total,
                },
            );
        }
    }

    tokio::fs::write(&file_path, &data)
        .await
        .map_err(|e| format!("Cannot write model file: {e}"))?;

    Ok(file_path.to_string_lossy().into_owned())
}

/// Read a previously downloaded model into memory (returns bytes).
#[tauri::command]
async fn read_model(app: AppHandle, model_id: String) -> Result<Vec<u8>, String> {
    let dir = models_dir(&app)?;
    let file_path = dir.join(format!("{model_id}.onnx"));
    if !file_path.exists() {
        return Err(format!("Model not found: {}", file_path.display()));
    }
    tokio::fs::read(&file_path)
        .await
        .map_err(|e| format!("Cannot read model: {e}"))
}

/// Check if a model is already downloaded.
#[tauri::command]
async fn is_model_downloaded(app: AppHandle, model_id: String) -> Result<bool, String> {
    let dir = models_dir(&app)?;
    let file_path = dir.join(format!("{model_id}.onnx"));
    Ok(file_path.exists()
        && std::fs::metadata(&file_path)
            .map(|m| m.len() > 0)
            .unwrap_or(false))
}

#[cfg(target_os = "windows")]
fn configure_ort_runtime() {
    if std::env::var_os("ORT_DYLIB_PATH").is_some() {
        return;
    }
    let dev_runtime = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../node_modules/onnxruntime-node/bin/napi-v3/win32/x64/onnxruntime.dll");
    let bundled_runtime = std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(std::path::Path::to_owned))
        .map(|directory| directory.join("runtime/onnxruntime.dll"));
    if let Some(runtime) = std::iter::once(dev_runtime)
        .chain(bundled_runtime)
        .find(|candidate| candidate.is_file())
    {
        std::env::set_var("ORT_DYLIB_PATH", runtime);
    }
}

#[cfg(target_os = "windows")]
fn configure_bundled_ort_runtime(app: &AppHandle) {
    if std::env::var_os("ORT_DYLIB_PATH").is_some() {
        return;
    }
    let runtime = app
        .path()
        .resource_dir()
        .ok()
        .map(|directory| directory.join("runtime/onnxruntime.dll"));
    if let Some(runtime) = runtime.filter(|candidate| candidate.is_file()) {
        std::env::set_var("ORT_DYLIB_PATH", runtime);
    }
}

#[cfg(not(target_os = "windows"))]
fn configure_ort_runtime() {}
#[cfg(not(target_os = "windows"))]
fn configure_bundled_ort_runtime(_app: &AppHandle) {}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    configure_ort_runtime();
    tauri::Builder::default()
        .manage(native_runtime::NativeRuntimeState::default())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            download_model,
            read_model,
            is_model_downloaded,
            native_runtime::native_gpu_initialize,
            native_runtime::native_gpu_set_graph,
            native_runtime::native_gpu_upload_image,
            native_runtime::native_gpu_remove_texture,
            native_runtime::native_gpu_read_output,
            native_runtime::native_gpu_read_preview,
            native_runtime::native_gpu_attach_video,
            native_runtime::native_gpu_detach_video,
            native_runtime::native_gpu_video_metrics,
            native_runtime::native_video_devices,
            native_runtime::native_gpu_play,
            native_runtime::native_gpu_pause,
            native_runtime::native_gpu_resume,
            native_runtime::native_gpu_stop,
            native_runtime::native_gpu_render_once,
            native_runtime::native_gpu_set_mouse,
            native_runtime::native_gpu_drain_events,
            native_runtime::native_gpu_close,
            native_runtime::native_onnx_capabilities,
            native_runtime::native_onnx_load_model,
            native_runtime::native_onnx_unload_model,
        ])
        .setup(|app| {
            configure_bundled_ort_runtime(app.handle());
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            native_runtime::maybe_start_smoke(app.handle());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
