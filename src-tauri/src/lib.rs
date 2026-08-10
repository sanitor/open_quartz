mod native_runtime;
mod native_video;
mod screen_saver;
mod webview_texture_stream;

use futures::StreamExt;
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager, State};

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

#[cfg(windows)]
fn schedule_texture_stream_retry(app: AppHandle) {
    std::thread::spawn(move || {
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if app
                .state::<webview_texture_stream::TextureStreamCapabilityState>()
                .get()
                .stream_ready
            {
                return;
            }
            let callback_app = app.clone();
            let _ = app.run_on_main_thread(move || {
                if callback_app
                    .state::<webview_texture_stream::TextureStreamCapabilityState>()
                    .get()
                    .stream_ready
                {
                    return;
                }
                let Some(webview) = callback_app.get_webview_window("main") else {
                    return;
                };
                let initialize_app = callback_app.clone();
                let _ = webview.with_webview(move |platform_webview| {
                    let capability =
                        webview_texture_stream::initialize(&platform_webview.environment());
                    if capability.stream_ready {
                        println!(
                            "[oq:native] webview-texture-stream ready adapterLuid={:?}",
                            capability.adapter_luid
                        );
                    }
                    initialize_app
                        .state::<webview_texture_stream::TextureStreamCapabilityState>()
                        .set(capability);
                });
            });
        }
    });
}

#[tauri::command]
fn webview_texture_stream_capability(
    state: State<'_, webview_texture_stream::TextureStreamCapabilityState>,
) -> webview_texture_stream::TextureStreamCapability {
    state.get()
}

#[tauri::command]
async fn native_video_thumbnail(path: String) -> Result<Vec<u8>, String> {
    tauri::async_runtime::spawn_blocking(move || native_video::read_video_thumbnail(&path))
        .await
        .map_err(|error| format!("Video thumbnail worker failed: {error}"))?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let prepared_screen_saver = screen_saver::prepare();
    configure_ort_runtime();
    tauri::Builder::default()
        .manage(screen_saver::state(prepared_screen_saver))
        .manage(native_runtime::NativeRuntimeState::default())
        .manage(webview_texture_stream::TextureStreamCapabilityState::default())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            download_model,
            read_model,
            is_model_downloaded,
            screen_saver::screen_saver_export,
            screen_saver::screen_saver_bootstrap,
            screen_saver::screen_saver_read_file,
            screen_saver::screen_saver_exit,
            native_runtime::native_gpu_initialize,
            webview_texture_stream_capability,
            native_video_thumbnail,
            native_runtime::native_gpu_set_graph,
            native_runtime::native_gpu_upload_image,
            native_runtime::native_gpu_remove_texture,
            native_runtime::native_gpu_read_output,
            native_runtime::native_gpu_read_preview,
            native_runtime::native_gpu_set_shared_texture_enabled,
            native_runtime::native_gpu_take_shared_texture,
            native_runtime::native_gpu_release_shared_texture,
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
            screen_saver::configure_window(app)?;
            configure_bundled_ort_runtime(app.handle());
            #[cfg(windows)]
            if let Some(webview) = app.get_webview_window("main") {
                let app_handle = app.handle().clone();
                webview.with_webview(move |platform_webview| {
                    let state = app_handle
                        .state::<webview_texture_stream::TextureStreamCapabilityState>();
                    let capability = webview_texture_stream::initialize(
                        &platform_webview.environment(),
                    );
                    println!(
                        "[oq:native] webview-texture-stream available={} streamReady={} adapterLuid={:?} reason={:?}",
                        capability.available,
                        capability.stream_ready,
                        capability.adapter_luid,
                        capability.reason
                    );
                    let retry = !capability.stream_ready;
                    state.set(capability);
                    if retry {
                        schedule_texture_stream_retry(app_handle.clone());
                    }
                })?;
            }
            #[cfg(not(windows))]
            app.state::<webview_texture_stream::TextureStreamCapabilityState>()
                .set(webview_texture_stream::TextureStreamCapability {
                    available: false,
                    adapter_luid: None,
                    stream_ready: false,
                    reason: Some("TextureStream is only available in WebView2".to_owned()),
                });
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
