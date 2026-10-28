use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use open_quartz_execution::gpu::{GpuPreviewReader, SharedTextureFrame};
use open_quartz_execution::native_video::{
    find_ffmpeg, list_video_devices, NativeVideoConfig, NativeVideoDevice, NativeVideoInfo,
    NativeVideoSourceKind,
};
use open_quartz_native_host::{
    NativeFrameRendered, NativeHost, NativeOnnxConfig, NativeOnnxLoadOptions, NativeRuntimeInfo,
};
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Default)]
pub struct NativeRuntimeState {
    runtime: Arc<Mutex<Option<NativeHost>>>,
    preview: Mutex<Option<GpuPreviewReader>>,
    alive: Arc<AtomicBool>,
    playing: Arc<AtomicBool>,
    worker: Mutex<Option<JoinHandle<()>>>,
    #[cfg(windows)]
    presentation_scheduled: Arc<AtomicBool>,
}

impl Drop for NativeRuntimeState {
    fn drop(&mut self) {
        shutdown_worker(self);
    }
}

fn emit_perf(message: std::fmt::Arguments<'_>) {
    log::info!(target: "open_quartz::perf", "{message}");
    if std::env::var_os("OPEN_QUARTZ_NATIVE_VIDEO_BENCH").is_some()
        || std::env::var_os("OPEN_QUARTZ_PERF_LOG").is_some()
    {
        println!("[oq:perf] {message}");
    }
}

#[cfg(windows)]
struct NativePresentationPerf {
    window_started: Instant,
    frames: u64,
    total: Duration,
    max: Duration,
}

#[cfg(windows)]
impl NativePresentationPerf {
    fn new() -> Self {
        Self {
            window_started: Instant::now(),
            frames: 0,
            total: Duration::ZERO,
            max: Duration::ZERO,
        }
    }

    fn record(&mut self, elapsed: Duration) {
        self.frames += 1;
        self.total += elapsed;
        self.max = self.max.max(elapsed);
        let window = self.window_started.elapsed();
        if window < Duration::from_secs(1) {
            return;
        }
        let frames = self.frames.max(1) as f64;
        emit_perf(format_args!(
            "native-presentation fps={:.1} avg_ms={:.3} max_ms={:.3}",
            self.frames as f64 / window.as_secs_f64(),
            self.total.as_secs_f64() * 1000.0 / frames,
            self.max.as_secs_f64() * 1000.0,
        ));
        *self = Self::new();
    }
}

#[cfg(windows)]
static NATIVE_PRESENTATION_PERF: LazyLock<Mutex<NativePresentationPerf>> =
    LazyLock::new(|| Mutex::new(NativePresentationPerf::new()));

struct PreviewPerf {
    window_started: Instant,
    calls: u64,
    source_lookup: Duration,
    gpu_read: Duration,
    scale_submit: Duration,
    readback: Duration,
    total: Duration,
    max_total: Duration,
    bytes: u64,
}

impl PreviewPerf {
    fn new() -> Self {
        Self {
            window_started: Instant::now(),
            calls: 0,
            source_lookup: Duration::ZERO,
            gpu_read: Duration::ZERO,
            scale_submit: Duration::ZERO,
            readback: Duration::ZERO,
            total: Duration::ZERO,
            max_total: Duration::ZERO,
            bytes: 0,
        }
    }

    fn record(
        &mut self,
        source_lookup: Duration,
        gpu_read: Duration,
        scale_submit: Duration,
        readback: Duration,
        total: Duration,
        bytes: usize,
        width: u32,
        height: u32,
    ) {
        if self.calls == 0 {
            self.window_started = Instant::now();
        }
        self.calls += 1;
        self.source_lookup += source_lookup;
        self.gpu_read += gpu_read;
        self.scale_submit += scale_submit;
        self.readback += readback;
        self.total += total;
        self.max_total = self.max_total.max(total);
        self.bytes = self.bytes.saturating_add(bytes as u64);
        let window = self.window_started.elapsed();
        if window < Duration::from_secs(1) {
            return;
        }
        let calls = self.calls.max(1) as f64;
        emit_perf(format_args!(
            "native-preview rate={:.1}/s size={}x{} avg_ms={{source:{:.3},scale_submit:{:.3},readback:{:.3},gpu_total:{:.3},total:{:.3}}} max_total_ms={:.3} throughput_mib_s={:.1}",
            self.calls as f64 / window.as_secs_f64(),
            width,
            height,
            self.source_lookup.as_secs_f64() * 1000.0 / calls,
            self.scale_submit.as_secs_f64() * 1000.0 / calls,
            self.readback.as_secs_f64() * 1000.0 / calls,
            self.gpu_read.as_secs_f64() * 1000.0 / calls,
            self.total.as_secs_f64() * 1000.0 / calls,
            self.max_total.as_secs_f64() * 1000.0,
            self.bytes as f64 / (1024.0 * 1024.0) / window.as_secs_f64(),
        ));
        *self = Self::new();
    }
}

static PREVIEW_PERF: LazyLock<Mutex<PreviewPerf>> =
    LazyLock::new(|| Mutex::new(PreviewPerf::new()));

#[cfg(windows)]
async fn wait_for_texture_stream_capability(app: &AppHandle) {
    for _ in 0..40 {
        let capability = app
            .state::<crate::webview_texture_stream::TextureStreamCapabilityState>()
            .get();
        if capability.stream_ready || (!capability.available && capability.reason.is_some()) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn initialize_runtime(
    app: &AppHandle,
    state: &NativeRuntimeState,
) -> Result<NativeRuntimeInfo, String> {
    #[cfg(windows)]
    wait_for_texture_stream_capability(app).await;
    shutdown_worker(state);
    state
        .preview
        .lock()
        .map_err(|_| "Native preview lock is poisoned".to_owned())?
        .take();
    state
        .runtime
        .lock()
        .map_err(|_| "Native runtime lock is poisoned".to_owned())?
        .take();
    let (runtime, info) = NativeHost::new().await?;
    #[cfg(windows)]
    let mut info = info;
    #[cfg(windows)]
    if app
        .state::<crate::webview_texture_stream::TextureStreamCapabilityState>()
        .get()
        .stream_ready
        && info.shared_texture
    {
        info.output_mode = "webview-texture-stream".to_owned();
    }
    let preview = GpuPreviewReader::new(runtime.backend());
    *state
        .preview
        .lock()
        .map_err(|_| "Native preview lock is poisoned".to_owned())? = Some(preview);
    state.alive.store(true, Ordering::Release);
    *state
        .runtime
        .lock()
        .map_err(|_| "Native runtime lock is poisoned".to_owned())? = Some(runtime);
    state.playing.store(false, Ordering::Release);
    start_worker(app, state)?;
    Ok(info)
}

#[tauri::command]
pub async fn native_gpu_initialize(
    app: AppHandle,
    state: State<'_, NativeRuntimeState>,
) -> Result<NativeRuntimeInfo, String> {
    initialize_runtime(&app, &state).await
}

#[tauri::command]
pub fn native_gpu_set_graph(
    graph_json: String,
    state: State<'_, NativeRuntimeState>,
) -> Result<u32, String> {
    let should_restart = state.playing.load(Ordering::Acquire);
    with_runtime(&state, |runtime| {
        let revision = runtime.set_graph(&graph_json)?;
        if should_restart {
            runtime.start_playback()?;
        }
        Ok(revision)
    })
}

#[tauri::command]
pub fn native_host_resource_intents(request_json: String) -> Result<String, String> {
    open_quartz::ffi::plan_host_resource_intents_json(&request_json)
}

#[tauri::command]
pub fn native_gpu_upload_image(
    request: tauri::ipc::Request<'_>,
    state: State<'_, NativeRuntimeState>,
) -> Result<(), String> {
    let node_id = resource_header(&request, "x-open-quartz-node-id")?.to_owned();
    let width = resource_header(&request, "x-open-quartz-width")?
        .parse::<u32>()
        .map_err(|error| format!("Invalid image width: {error}"))?;
    let height = resource_header(&request, "x-open-quartz-height")?
        .parse::<u32>()
        .map_err(|error| format!("Invalid image height: {error}"))?;
    let rgba = match request.body() {
        tauri::ipc::InvokeBody::Raw(bytes) => bytes.as_slice(),
        tauri::ipc::InvokeBody::Json(_) => {
            return Err("Image upload requires a raw Uint8Array body".to_owned());
        }
    };
    with_runtime(&state, |runtime| {
        runtime.upload_image(&node_id, rgba, width, height)
    })
}

#[tauri::command]
pub fn native_gpu_remove_texture(
    node_id: String,
    state: State<'_, NativeRuntimeState>,
) -> Result<(), String> {
    with_runtime(&state, |runtime| {
        runtime.remove_texture(&node_id);
        Ok(())
    })
}

#[tauri::command]
pub async fn native_gpu_read_output(
    node_id: String,
    state: State<'_, NativeRuntimeState>,
) -> Result<tauri::ipc::Response, String> {
    let (backend, output) = with_runtime(&state, |runtime| {
        let output = runtime.preview_source(&node_id)?;
        Ok((runtime.backend(), output))
    })?;
    let rgba = backend
        .read_texture_rgba(&output.texture, output.width, output.height)
        .await?;
    Ok(tauri::ipc::Response::new(NativeHost::output_payload(
        &rgba,
        output.width,
        output.height,
    )))
}

fn read_preview_payload(
    state: &NativeRuntimeState,
    node_id: &str,
    max_dimension: u32,
) -> Result<Vec<u8>, String> {
    let total_started = Instant::now();
    let source_started = Instant::now();
    let source = with_runtime(state, |runtime| runtime.preview_source(node_id))?;
    let source_lookup = source_started.elapsed();
    let gpu_started = Instant::now();
    let mut preview = state
        .preview
        .lock()
        .map_err(|_| "Native preview lock is poisoned".to_owned())?;
    let reader = preview
        .as_mut()
        .ok_or_else(|| "Native preview reader is not initialized".to_owned())?;
    let image = pollster::block_on(reader.read(&source, max_dimension))?;
    let gpu_read = gpu_started.elapsed();
    let payload = NativeHost::output_payload(&image.rgba, image.width, image.height);
    let width = image.width;
    let height = image.height;
    let scale_submit = image.scale_submit;
    let readback = image.readback;
    drop(preview);
    if let Ok(mut perf) = PREVIEW_PERF.lock() {
        perf.record(
            source_lookup,
            gpu_read,
            scale_submit,
            readback,
            total_started.elapsed(),
            payload.len(),
            width,
            height,
        );
    }
    Ok(payload)
}

#[tauri::command]
pub fn native_gpu_read_preview(
    node_id: String,
    max_dimension: u32,
    state: State<'_, NativeRuntimeState>,
) -> Result<tauri::ipc::Response, String> {
    let payload = read_preview_payload(&state, &node_id, max_dimension)?;
    Ok(tauri::ipc::Response::new(payload))
}

#[tauri::command]
pub fn native_gpu_set_shared_texture_enabled(
    enabled: bool,
    state: State<'_, NativeRuntimeState>,
) -> Result<bool, String> {
    #[cfg(windows)]
    {
        return with_runtime(&state, |runtime| {
            runtime.set_shared_texture_enabled(enabled)
        });
    }
    #[cfg(not(windows))]
    {
        let _ = (enabled, state);
        Err("Shared textures are not implemented on this platform".to_owned())
    }
}

#[tauri::command]
pub fn native_gpu_take_shared_texture(
    state: State<'_, NativeRuntimeState>,
) -> Result<Option<SharedTextureFrame>, String> {
    #[cfg(windows)]
    {
        return with_runtime(&state, |runtime| Ok(runtime.take_shared_texture()));
    }
    #[cfg(not(windows))]
    {
        let _ = state;
        Err("Shared textures are not implemented on this platform".to_owned())
    }
}

#[tauri::command]
pub fn native_gpu_release_shared_texture(
    lease_id: u64,
    state: State<'_, NativeRuntimeState>,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        return with_runtime(&state, |runtime| runtime.release_shared_texture(lease_id));
    }
    #[cfg(not(windows))]
    {
        let _ = (lease_id, state);
        Err("Shared textures are not implemented on this platform".to_owned())
    }
}

fn resource_header<'a>(
    request: &'a tauri::ipc::Request<'_>,
    name: &str,
) -> Result<&'a str, String> {
    request
        .headers()
        .get(name)
        .ok_or_else(|| format!("Missing image resource header {name}"))?
        .to_str()
        .map_err(|error| format!("Invalid image resource header {name}: {error}"))
}

#[tauri::command]
pub fn native_gpu_play(state: State<'_, NativeRuntimeState>) -> Result<(), String> {
    with_runtime(&state, |runtime| {
        if !runtime.has_graph() {
            return Err("Native runtime must receive a graph before play".to_owned());
        }
        runtime.start_playback()
    })?;
    state.playing.store(true, Ordering::Release);
    Ok(())
}

#[tauri::command]
pub fn native_gpu_pause(state: State<'_, NativeRuntimeState>) -> Result<(), String> {
    let was_playing = state.playing.swap(false, Ordering::AcqRel);
    let result = with_runtime(&state, NativeHost::pause);
    if result.is_err() && was_playing {
        state.playing.store(true, Ordering::Release);
    }
    result
}

#[tauri::command]
pub fn native_gpu_resume(state: State<'_, NativeRuntimeState>) -> Result<(), String> {
    with_runtime(&state, NativeHost::resume)?;
    state.playing.store(true, Ordering::Release);
    Ok(())
}

#[tauri::command]
pub fn native_gpu_stop(state: State<'_, NativeRuntimeState>) -> Result<(), String> {
    state.playing.store(false, Ordering::Release);
    with_runtime(&state, NativeHost::stop)
}

#[tauri::command]
pub fn native_gpu_attach_video(
    node_id: String,
    kind: NativeVideoSourceKind,
    source: String,
    looping: bool,
    playback_rate: f64,
    state: State<'_, NativeRuntimeState>,
) -> Result<NativeVideoInfo, String> {
    with_runtime(&state, |runtime| {
        runtime.attach_video(
            &node_id,
            NativeVideoConfig {
                kind,
                source,
                looping,
                playback_rate,
            },
        )
    })
}

#[tauri::command]
pub fn native_gpu_video_metrics(
    node_id: String,
    state: State<'_, NativeRuntimeState>,
) -> Result<open_quartz_execution::native_video::NativeVideoMetrics, String> {
    with_runtime(&state, |runtime| runtime.video_metrics(&node_id))
}

#[tauri::command]
pub fn native_gpu_detach_video(
    node_id: String,
    state: State<'_, NativeRuntimeState>,
) -> Result<(), String> {
    with_runtime(&state, |runtime| runtime.detach_video(&node_id))
}
#[tauri::command]
pub fn native_video_devices() -> Result<Vec<NativeVideoDevice>, String> {
    list_video_devices()
}

#[tauri::command]
pub fn native_gpu_render_once(
    state: State<'_, NativeRuntimeState>,
) -> Result<NativeFrameRendered, String> {
    with_runtime(&state, NativeHost::render_next)
}

#[tauri::command]
pub fn native_gpu_set_mouse(
    mouse: [f32; 4],
    state: State<'_, NativeRuntimeState>,
) -> Result<(), String> {
    with_runtime(&state, |runtime| {
        runtime.set_mouse(mouse);
        Ok(())
    })
}

#[tauri::command]
pub fn native_gpu_drain_events(state: State<'_, NativeRuntimeState>) -> Result<String, String> {
    with_runtime(&state, NativeHost::drain_events_json)
}

#[tauri::command]
pub fn native_gpu_close(state: State<'_, NativeRuntimeState>) -> Result<(), String> {
    close_runtime(&state)
}

fn close_runtime(state: &NativeRuntimeState) -> Result<(), String> {
    shutdown_worker(state);
    state
        .preview
        .lock()
        .map_err(|_| "Native preview lock is poisoned".to_owned())?
        .take();
    state
        .runtime
        .lock()
        .map_err(|_| "Native runtime lock is poisoned".to_owned())?
        .take();
    Ok(())
}

#[tauri::command]
pub fn native_onnx_capabilities(
) -> Result<open_quartz_execution::onnx::NativeOnnxCapabilities, String> {
    open_quartz_execution::onnx::native_onnx_capabilities()
}

#[tauri::command]
pub async fn native_onnx_load_model(
    app: AppHandle,
    node_id: String,
    model_id: String,
    options: NativeOnnxLoadOptions,
    state: State<'_, NativeRuntimeState>,
) -> Result<open_quartz_execution::onnx::OnnxSessionInfo, String> {
    let cache_directory = crate::models_dir(&app)?;
    let model_id_for_cache = model_id.clone();
    let config = NativeOnnxConfig {
        model_id,
        task: options.task,
        target_size: options.target_size,
        score_threshold: options.score_threshold,
        iou_threshold: options.iou_threshold,
    };
    let prefer_direct_ml = options.prefer_direct_ml;
    let prepared = tokio::task::spawn_blocking(move || {
        if let Some(model_path) = options.model_path {
            open_quartz_native_host::models::PreparedOnnxModel::open(
                &std::path::PathBuf::from(model_path),
                config,
                prefer_direct_ml,
            )
        } else {
            open_quartz_native_host::models::ModelStore::new(cache_directory).open_cached(
                &model_id_for_cache,
                config,
                prefer_direct_ml,
            )
        }
    })
    .await
    .map_err(|error| format!("Model loading worker failed: {error}"))??;
    with_runtime(&state, move |runtime| prepared.attach(runtime, node_id))
}

#[tauri::command]
pub fn native_onnx_unload_model(
    node_id: String,
    state: State<'_, NativeRuntimeState>,
) -> Result<(), String> {
    with_runtime(&state, |runtime| {
        runtime.unload_onnx(&node_id);
        Ok(())
    })
}

fn smoke_native_onnx() -> Result<(String, f32), String> {
    let capabilities = open_quartz_execution::onnx::native_onnx_capabilities()?;
    let provider = if capabilities.direct_ml {
        open_quartz_execution::onnx::NativeOnnxProvider::DirectMl
    } else {
        open_quartz_execution::onnx::NativeOnnxProvider::Cpu
    };
    let mut session = open_quartz_execution::onnx::OnnxSession::from_memory_with_options(
        include_bytes!("../../crates/open_quartz/tests/data/identity.onnx"),
        open_quartz_execution::onnx::NativeOnnxOptions {
            provider,
            allow_cpu_fallback: false,
        },
    )?;
    let backend = session.info().backend.clone();
    let output = session.run_f32(vec![7.0], vec![1])?;
    Ok((backend, output.data[0]))
}

fn smoke_native_onnx_graph(runtime: &mut NativeHost) -> Result<bool, String> {
    let graph = r#"{
        "nodes": [
            {
                "id": "image", "type": "input", "position": {"x": 0.0, "y": 0.0},
                "data": {
                    "type": "input", "label": "Image", "shaderCode": "", "inputs": [],
                    "outputs": [{"id": "image_out", "label": "output", "dataType": "sampler2D", "direction": "output"}],
                    "uniforms": {}, "inputMode": "image", "inputDataType": "sampler2D",
                    "imageWidth": 2, "imageHeight": 2
                }
            },
            {
                "id": "onnx", "type": "onnx", "position": {"x": 1.0, "y": 0.0},
                "data": {
                    "type": "onnx", "label": "Image identity", "shaderCode": "",
                    "inputs": [{"id": "onnx_in", "label": "image", "dataType": "sampler2D", "direction": "input"}],
                    "outputs": [{"id": "onnx_out", "label": "output", "dataType": "sampler2D", "direction": "output"}],
                    "uniforms": {}, "onnxCatalogId": "image-identity"
                }
            },
            {
                "id": "renderer", "type": "renderer", "position": {"x": 2.0, "y": 0.0},
                "data": {
                    "type": "renderer", "label": "Output", "shaderCode": "",
                    "inputs": [{"id": "renderer_in", "label": "inputImage", "dataType": "sampler2D", "direction": "input"}],
                    "outputs": [], "uniforms": {}
                }
            }
        ],
        "edges": [
            {"id": "e1", "source": "image", "sourceHandle": "image_out", "target": "onnx", "targetHandle": "onnx_in"},
            {"id": "e2", "source": "onnx", "sourceHandle": "onnx_out", "target": "renderer", "targetHandle": "renderer_in"}
        ]
    }"#;
    runtime.set_graph(graph)?;
    let pixels = [
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
    ];
    runtime.upload_image("image", &pixels, 2, 2)?;
    let session = open_quartz_execution::onnx::OnnxSession::from_memory(include_bytes!(
        "../../crates/open_quartz/tests/data/image_identity.onnx"
    ))?;
    runtime.load_onnx(
        "onnx".to_owned(),
        session,
        NativeOnnxConfig {
            model_id: "image-identity".to_owned(),
            task: open_quartz_execution::onnx::OnnxTask::Generic,
            target_size: 2,
            score_threshold: 0.25,
            iou_threshold: 0.45,
        },
    )?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while !runtime.has_output_texture("onnx") && Instant::now() < deadline {
        runtime.render_next()?;
        std::thread::sleep(Duration::from_millis(10));
    }
    let readback = runtime.read_output("renderer")?;
    let events = runtime.take_output_events();
    Ok(readback.get(8..) == Some(pixels.as_slice())
        && events
            .iter()
            .any(|event| event.node_id == "onnx" && event.backend == "cpu"))
}

#[cfg(windows)]
fn smoke_shared_texture_presenter(runtime: &mut NativeHost) -> Result<bool, String> {
    runtime.set_shared_texture_enabled(true)?;
    runtime.render_next()?;
    let Some(frame) = runtime.take_shared_texture() else {
        return Ok(false);
    };
    let valid = frame.platform == open_quartz_execution::gpu::SharedTexturePlatform::Dxgi
        && frame.resource_handle != 0
        && frame.sync_handle.is_some()
        && frame.sync_value > 0
        && frame.width > 0
        && frame.height > 0;
    runtime.release_shared_texture(frame.lease_id)?;
    runtime.set_shared_texture_enabled(false)?;
    Ok(valid)
}

fn smoke_native_preview_latency(
    state: &NativeRuntimeState,
    image_graph: &str,
) -> Result<f64, String> {
    let graph = image_graph
        .replace(
            "\"imageWidth\": 2, \"imageHeight\": 2",
            "\"imageWidth\": 1920, \"imageHeight\": 1080",
        )
        .replace(
            "\"width\": 2, \"height\": 2",
            "\"width\": 1920, \"height\": 1080",
        );
    with_runtime(state, |runtime| {
        runtime.set_graph(&graph)?;
        let pixels = vec![255; 1920 * 1080 * 4];
        runtime.upload_image("image", &pixels, 1920, 1080)?;
        runtime.render_next()?;
        Ok(())
    })?;
    let started = Instant::now();
    let preview = read_preview_payload(state, "renderer", 960)?;
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    if preview.get(..8) != Some([192, 3, 0, 0, 28, 2, 0, 0].as_slice()) {
        return Err("Native 1080p preview did not scale to 960x540".to_owned());
    }
    Ok(elapsed_ms)
}

fn smoke_native_video(runtime: &mut NativeHost, image_graph: &str) -> Result<bool, String> {
    let ffmpeg = find_ffmpeg()?;
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("open-quartz-smoke-video-{suffix}.mp4"));
    let status = std::process::Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=16x16:r=10",
            "-t",
            "0.4",
            "-pix_fmt",
            "yuv420p",
            "-y",
        ])
        .arg(&path)
        .status()
        .map_err(|error| format!("Cannot create native video smoke fixture: {error}"))?;
    if !status.success() {
        return Err("FFmpeg could not create native video smoke fixture".to_owned());
    }

    let result = (|| {
        runtime.remove_texture("image");
        let video_graph =
            image_graph.replacen("\"inputMode\": \"image\"", "\"inputMode\": \"video\"", 1);
        runtime.set_graph(&video_graph)?;
        runtime.attach_video(
            "image",
            NativeVideoConfig {
                kind: NativeVideoSourceKind::File,
                source: path.to_string_lossy().into_owned(),
                looping: true,
                playback_rate: 1.0,
            },
        )?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while !runtime.has_output_texture("image") && Instant::now() < deadline {
            runtime.upload_video_frames()?;
            std::thread::sleep(Duration::from_millis(20));
        }
        if !runtime.has_output_texture("image") {
            return Err("Native video decoder produced no GPU frame".to_owned());
        }
        runtime.render_next()?;
        let readback = runtime.read_output("renderer")?;
        Ok(readback.get(8..12).is_some_and(|pixel| {
            pixel[0] > 240 && pixel[1] < 16 && pixel[2] < 16 && pixel[3] == 255
        }))
    })();
    let _ = runtime.detach_video("image");
    let _ = std::fs::remove_file(path);
    result
}

fn start_native_video_benchmark(app: &AppHandle, path: String) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<NativeRuntimeState>();
        let result = async {
            let info = initialize_runtime(&app, &state).await?;
            let graph = r#"{
                "nodes": [
                    {
                        "id": "video", "type": "input", "position": {"x": 0.0, "y": 0.0},
                        "data": {
                            "type": "input", "label": "Video", "shaderCode": "", "inputs": [],
                            "outputs": [{"id": "video_out", "label": "output", "dataType": "sampler2D", "direction": "output"}],
                            "uniforms": {}, "inputMode": "video", "inputDataType": "sampler2D",
                            "imageWidth": 7680, "imageHeight": 3840
                        }
                    },
                    {
                        "id": "renderer", "type": "renderer", "position": {"x": 1.0, "y": 0.0},
                        "data": {
                            "type": "renderer", "label": "Output", "shaderCode": "",
                            "inputs": [{"id": "renderer_in", "label": "inputImage", "dataType": "sampler2D", "direction": "input"}],
                            "outputs": [], "uniforms": {}
                        }
                    }
                ],
                "edges": [
                    {"id": "e1", "source": "video", "sourceHandle": "video_out", "target": "renderer", "targetHandle": "renderer_in"}
                ]
            }"#;
            let video_info = with_runtime(&state, |runtime| {
                runtime.set_graph(graph)?;
                runtime.attach_video(
                    "video",
                    NativeVideoConfig {
                        kind: NativeVideoSourceKind::File,
                        source: path,
                        looping: true,
                        playback_rate: 1.0,
                    },
                )
            })?;
            let startup_deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < startup_deadline {
                let ready = with_runtime(&state, |runtime| {
                    runtime.upload_video_frames()?;
                    Ok(runtime.has_output_texture("video"))
                })?;
                if ready {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            let started = Instant::now();
            let deadline = started + Duration::from_secs(10);
            let mut rendered = 0_u64;
            let mut previews = 0_u64;
            let mut missed_deadlines = 0_u64;
            while Instant::now() < deadline {
                let tick_started = Instant::now();
                with_runtime(&state, |runtime| runtime.render_next())?;
                rendered += 1;
                let _ = read_preview_payload(&state, "renderer", 960)?;
                previews += 1;
                let elapsed = tick_started.elapsed();
                if elapsed < Duration::from_millis(16) {
                    std::thread::sleep(Duration::from_millis(16) - elapsed);
                } else {
                    missed_deadlines += 1;
                }
            }
            let elapsed = started.elapsed();
            let metrics = with_runtime(&state, |runtime| runtime.video_metrics("video"))?;
            Ok::<_, String>((
                info,
                video_info,
                rendered,
                previews,
                missed_deadlines,
                elapsed,
                metrics,
            ))
        }
        .await;
        let succeeded = result.is_ok();
        match result {
            Ok((info, video, rendered, previews, missed, elapsed, metrics)) => println!(
                "NATIVE_VIDEO_BENCH_OK adapter={} backend={} decoder={} source={}x{}@{:.1} duration_s={:.2} render_fps={:.2} preview_fps={:.2} missed={} decoded={} uploaded={} cpu_copy_bytes={}",
                info.adapter_name,
                info.backend,
                video.decoder,
                video.width,
                video.height,
                video.fps,
                elapsed.as_secs_f64(),
                rendered as f64 / elapsed.as_secs_f64(),
                previews as f64 / elapsed.as_secs_f64(),
                missed,
                metrics.decoded_frames,
                metrics.uploaded_frames,
                metrics.cpu_copy_bytes,
            ),
            Err(error) => eprintln!("NATIVE_VIDEO_BENCH_ERROR {error}"),
        }
        let _ = close_runtime(&state);
        app.exit(if succeeded { 0 } else { 1 });
    });
}

pub fn maybe_start_smoke(app: &AppHandle) {
    if let Some(path) = std::env::var_os("OPEN_QUARTZ_NATIVE_VIDEO_BENCH") {
        start_native_video_benchmark(app, path.to_string_lossy().into_owned());
        return;
    }
    if std::env::var_os("OPEN_QUARTZ_NATIVE_SMOKE").is_none() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<NativeRuntimeState>();
        let graph = r#"{
            "nodes": [
                {
                    "id": "image", "type": "input", "position": {"x": 0.0, "y": 0.0},
                    "data": {
                        "type": "input", "label": "Image", "shaderCode": "",
                        "inputs": [],
                        "outputs": [{"id": "image_out", "label": "output", "dataType": "sampler2D", "direction": "output"}],
                        "uniforms": {}, "inputMode": "image", "inputDataType": "sampler2D",
                        "imageWidth": 2, "imageHeight": 2
                    }
                },
                {
                    "id": "copy", "type": "shader", "position": {"x": 1.0, "y": 0.0},
                    "data": {
                        "type": "shader", "label": "Copy",
                        "shaderCode": "@group(0) @binding(0) var inputImage: texture_2d<f32>; @group(0) @binding(1) var inputImageSampler: sampler; @fragment fn main(@location(0) uv: vec2f) -> @location(0) vec4f { return textureSample(inputImage, inputImageSampler, uv); }",
                        "inputs": [{"id": "copy_in", "label": "inputImage", "dataType": "sampler2D", "direction": "input"}],
                        "outputs": [{"id": "copy_out", "label": "output", "dataType": "sampler2D", "direction": "output"}],
                        "uniforms": {}, "autoSize": false, "width": 2, "height": 2
                    }
                },
                {
                    "id": "renderer", "type": "renderer", "position": {"x": 2.0, "y": 0.0},
                    "data": {
                        "type": "renderer", "label": "Output", "shaderCode": "",
                        "inputs": [{"id": "renderer_in", "label": "inputImage", "dataType": "sampler2D", "direction": "input"}],
                        "outputs": [], "uniforms": {}
                    }
                }
            ],
            "edges": [
                {"id": "e1", "source": "image", "sourceHandle": "image_out", "target": "copy", "targetHandle": "copy_in"},
                {"id": "e2", "source": "copy", "sourceHandle": "copy_out", "target": "renderer", "targetHandle": "renderer_in"}
            ]
        }"#;
        let result = async {
            let info = initialize_runtime(&app, &state).await?;
            let (frame, mut readback_ok) = with_runtime(&state, |runtime| {
                runtime.set_graph(graph)?;
                runtime.start_playback()?;
                let pixels = [
                    255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
                ];
                runtime.upload_image("image", &pixels, 2, 2)?;
                let frame = runtime.render_next()?;
                let readback = runtime.read_output("renderer")?;
                Ok((frame, readback.get(8..) == Some(pixels.as_slice())))
            })?;
            let preview = read_preview_payload(&state, "renderer", 1)?;
            readback_ok &= preview.len() == 12 && preview[..8] == [1, 0, 0, 0, 1, 0, 0, 0];
            #[cfg(windows)]
            let shared_texture_ok = with_runtime(&state, smoke_shared_texture_presenter)?;
            #[cfg(not(windows))]
            let shared_texture_ok = true;
            let preview_ms = smoke_native_preview_latency(&state, graph)?;
            let video_ok = with_runtime(&state, |runtime| smoke_native_video(runtime, graph))?;
            let onnx_graph_ok = with_runtime(&state, smoke_native_onnx_graph)?;
            let onnx = smoke_native_onnx()?;
            Ok::<_, String>((
                info,
                frame,
                onnx,
                readback_ok,
                video_ok,
                onnx_graph_ok,
                shared_texture_ok,
                preview_ms,
            ))
        }
        .await;
        match result {
            Ok((
                info,
                frame,
                (onnx_backend, onnx_output),
                readback_ok,
                video_ok,
                onnx_graph_ok,
                shared_texture_ok,
                preview_ms,
            )) => {
                if !readback_ok || !video_ok || !onnx_graph_ok || !shared_texture_ok {
                    eprintln!(
                        "NATIVE_GPU_SMOKE_ERROR resource mismatch image={readback_ok} video={video_ok} onnx_graph={onnx_graph_ok} shared_texture={shared_texture_ok}"
                    );
                    let _ = close_runtime(&state);
                    app.exit(1);
                    return;
                }
                println!(
                    "NATIVE_GPU_SMOKE_OK adapter={} backend={} frame={} output={} size={}x{} image_readback=true video_readback=true onnx_graph=true shared_texture=true preview_1080p_ms={:.2} onnx={} onnx_output={}",
                    info.adapter_name,
                    info.backend,
                    frame.frame,
                    frame.output_node_id,
                    frame.width,
                    frame.height,
                    preview_ms,
                    onnx_backend,
                    onnx_output
                );
                let _ = close_runtime(&state);
                app.exit(0);
            }
            Err(error) => {
                eprintln!("NATIVE_GPU_SMOKE_ERROR {error}");
                let _ = close_runtime(&state);
                app.exit(1);
            }
        }
    });
}

fn with_runtime<T>(
    state: &NativeRuntimeState,
    operation: impl FnOnce(&mut NativeHost) -> Result<T, String>,
) -> Result<T, String> {
    let mut guard = state
        .runtime
        .lock()
        .map_err(|_| "Native runtime lock is poisoned".to_owned())?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| "Native GPU runtime is not initialized".to_owned())?;
    operation(runtime)
}

#[cfg(windows)]
fn present_latest_shared_texture(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<NativeRuntimeState>();
    let Some(frame) = with_runtime(&state, |runtime| Ok(runtime.take_shared_texture()))? else {
        return Ok(());
    };
    let started = Instant::now();
    let result = crate::webview_texture_stream::present_shared_frame(&frame);
    let presented = result.as_ref().is_ok_and(|frame| frame.presented);
    let release = with_runtime(&state, |runtime| {
        runtime.release_shared_texture(frame.lease_id)
    });
    let outcome = result.and(release).map(|_| ());
    if outcome.is_ok() && presented {
        if let Ok(mut perf) = NATIVE_PRESENTATION_PERF.lock() {
            perf.record(started.elapsed());
        }
    }
    outcome
}

fn lock_if_enabled<'a, T>(
    value: &'a Mutex<T>,
    enabled: &AtomicBool,
    poisoned: &str,
) -> Result<Option<std::sync::MutexGuard<'a, T>>, String> {
    let guard = value.lock().map_err(|_| poisoned.to_owned())?;
    Ok(enabled.load(Ordering::Acquire).then_some(guard))
}

fn start_worker(app: &AppHandle, state: &NativeRuntimeState) -> Result<(), String> {
    let runtime = state.runtime.clone();
    let alive = state.alive.clone();
    let playing = state.playing.clone();
    #[cfg(windows)]
    let presentation_scheduled = state.presentation_scheduled.clone();
    let app = app.clone();
    let worker = std::thread::Builder::new()
        .name("open-quartz-native-render".to_owned())
        .spawn(move || {
            while alive.load(Ordering::Acquire) {
                let tick_started = Instant::now();
                if playing.load(Ordering::Acquire) {
                    let result = lock_if_enabled(
                        &runtime,
                        &playing,
                        "Native runtime lock is poisoned",
                    )
                    .and_then(|guard| {
                        let Some(mut guard) = guard else {
                            return Ok(None);
                        };
                        let runtime = guard.as_mut().ok_or_else(|| {
                            "Native GPU runtime is not initialized".to_owned()
                        })?;
                        let frame = runtime.render_next()?;
                        #[cfg(windows)]
                        let presentation_pending = runtime.has_shared_texture_pending();
                        #[cfg(not(windows))]
                        let presentation_pending = false;
                        Ok(Some((frame, runtime.take_output_events(), presentation_pending)))
                    });
                    match result {
                        Ok(Some((frame, output_events, presentation_pending))) => {
                            #[cfg(not(windows))]
                            let _ = presentation_pending;
                            for event in output_events {
                                let _ = app.emit("native-runtime-output", event);
                            }
                            if frame.width > 0 && frame.height > 0 {
                                let _ = app.emit("native-runtime-frame", frame);
                            }
                            #[cfg(windows)]
                            if presentation_pending
                                && !presentation_scheduled.swap(true, Ordering::AcqRel)
                            {
                                let callback_app = app.clone();
                                let callback_scheduled = presentation_scheduled.clone();
                                if let Err(error) = app.run_on_main_thread(move || {
                                    if let Err(error) = present_latest_shared_texture(&callback_app) {
                                        eprintln!("[oq:native] texture-stream-present-error {error}");
                                        let state = callback_app.state::<NativeRuntimeState>();
                                        let _ = with_runtime(&state, |runtime| {
                                            runtime.set_shared_texture_enabled(false).map(|_| ())
                                        });

                                        let _ = callback_app.emit(
                                            "native-runtime-presentation-fallback",
                                            error,
                                        );
                                    }
                                    callback_scheduled.store(false, Ordering::Release);
                                }) {
                                    presentation_scheduled.store(false, Ordering::Release);
                                    let _ = app.emit(
                                        "native-runtime-error",
                                        format!("Cannot schedule WebView2 texture presentation: {error}"),
                                    );
                                }
                            }
                        }
                        Ok(None) => {}
                        Err(error) => {
                            playing.store(false, Ordering::Release);
                            let _ = app.emit("native-runtime-error", error);
                        }
                    }
                }
                let elapsed = tick_started.elapsed();
                if elapsed < Duration::from_millis(16) {
                    std::thread::sleep(Duration::from_millis(16) - elapsed);
                }
            }
        })
        .map_err(|error| format!("Cannot start native render thread: {error}"))?;
    *state
        .worker
        .lock()
        .map_err(|_| "Native worker lock is poisoned".to_owned())? = Some(worker);
    Ok(())
}

fn shutdown_worker(state: &NativeRuntimeState) {
    state.playing.store(false, Ordering::Release);
    state.alive.store(false, Ordering::Release);
    if let Ok(mut worker) = state.worker.lock() {
        if let Some(worker) = worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;

    #[test]
    fn render_worker_rechecks_playing_after_waiting_for_runtime_lock() {
        let runtime = Arc::new(Mutex::new(()));
        let playing = Arc::new(AtomicBool::new(true));
        let guard = match runtime.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let worker_runtime = runtime.clone();
        let worker_playing = playing.clone();
        let worker = thread::spawn(move || {
            super::lock_if_enabled(&worker_runtime, &worker_playing, "poisoned")
                .expect("runtime lock should remain available")
                .is_some()
        });

        playing.store(false, Ordering::Release);
        drop(guard);

        assert!(!worker.join().expect("render worker should finish"));
    }
}
