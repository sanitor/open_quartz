use std::cell::Cell;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use regex::Regex;
use serde::{Deserialize, Serialize};

#[cfg(windows)]
use open_quartz::gpu::D3d12VideoFrame;

pub enum NativeVideoFrame<'a> {
    Rgba(&'a [u8]),
    #[cfg(windows)]
    D3d12(&'a D3d12VideoFrame),
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NativeVideoSourceKind {
    File,
    Camera,
}

#[derive(Clone, Debug)]
pub struct NativeVideoConfig {
    pub kind: NativeVideoSourceKind,
    pub source: String,
    pub looping: bool,
    pub playback_rate: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeVideoInfo {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub decoder: String,
    #[serde(skip)]
    d3d12_eligible: bool,
    #[serde(skip)]
    d3d12_detach_required: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeVideoMetrics {
    pub decoded_frames: u64,
    pub uploaded_frames: u64,
    pub cpu_copy_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NativeVideoDevice {
    pub id: String,
    pub label: String,
}

struct FrameSlot {
    generation: u64,
    rgba: Vec<u8>,
    #[cfg(windows)]
    d3d12: Option<D3d12VideoFrame>,
}

pub struct NativeVideoSource {
    ffmpeg: PathBuf,
    config: NativeVideoConfig,
    info: NativeVideoInfo,
    slot: Arc<Mutex<FrameSlot>>,
    alive: Arc<AtomicBool>,
    decoded_frames: Arc<AtomicU64>,
    uploaded_frames: u64,
    cpu_copy_bytes: u64,
    child: Option<Child>,
    reader: Option<JoinHandle<()>>,
    position_seconds: f64,
    uploaded_generation: u64,
}

impl NativeVideoSource {
    pub fn open(config: NativeVideoConfig) -> Result<Self, String> {
        if !config.playback_rate.is_finite() || !(0.1..=4.0).contains(&config.playback_rate) {
            return Err("Video playback rate must be between 0.1 and 4.0".to_owned());
        }
        if config.kind == NativeVideoSourceKind::File && !Path::new(&config.source).is_file() {
            return Err(format!("Video file does not exist: {}", config.source));
        }
        let ffmpeg = find_ffmpeg()?;
        let info = probe_video(&ffmpeg, &config)?;
        let mut source = Self {
            ffmpeg,
            config,
            slot: Arc::new(Mutex::new(FrameSlot {
                generation: 0,
                rgba: Vec::new(),
                #[cfg(windows)]
                d3d12: None,
            })),
            alive: Arc::new(AtomicBool::new(false)),
            decoded_frames: Arc::new(AtomicU64::new(0)),
            uploaded_frames: 0,
            cpu_copy_bytes: 0,
            child: None,
            reader: None,
            position_seconds: 0.0,
            uploaded_generation: 0,
            info,
        };
        source.start_decoder()?;
        Ok(source)
    }

    pub fn info(&self) -> &NativeVideoInfo {
        &self.info
    }

    pub fn upload_latest(
        &mut self,
        mut upload: impl FnMut(NativeVideoFrame<'_>, u32, u32) -> Result<(), String>,
    ) -> Result<bool, String> {
        let mut slot = self
            .slot
            .lock()
            .map_err(|_| "Native video frame lock is poisoned".to_owned())?;
        if slot.generation == 0 || slot.generation == self.uploaded_generation {
            return Ok(false);
        }
        #[cfg(windows)]
        let used_d3d12 = slot.d3d12.is_some();
        {
            #[cfg(windows)]
            let frame = slot
                .d3d12
                .as_ref()
                .map(NativeVideoFrame::D3d12)
                .unwrap_or_else(|| NativeVideoFrame::Rgba(&slot.rgba));
            #[cfg(not(windows))]
            let frame = NativeVideoFrame::Rgba(&slot.rgba);
            upload(frame, self.info.width, self.info.height)?;
        }
        self.uploaded_generation = slot.generation;
        self.uploaded_frames = self.uploaded_frames.saturating_add(1);
        if !slot.rgba.is_empty() {
            self.cpu_copy_bytes = self.cpu_copy_bytes.saturating_add(slot.rgba.len() as u64);
        }
        #[cfg(windows)]
        if used_d3d12 {
            slot.d3d12 = None;
        }
        Ok(true)
    }

    pub fn metrics(&self) -> NativeVideoMetrics {
        NativeVideoMetrics {
            decoded_frames: self.decoded_frames.load(Ordering::Acquire),
            uploaded_frames: self.uploaded_frames,
            cpu_copy_bytes: self.cpu_copy_bytes,
        }
    }

    pub fn is_active(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    pub fn pause(&mut self) {
        self.stop_decoder();
    }

    pub fn resume(&mut self) -> Result<(), String> {
        if !self.alive.load(Ordering::Acquire) {
            self.start_decoder()?;
        }
        Ok(())
    }

    fn start_decoder(&mut self) -> Result<(), String> {
        self.alive.store(true, Ordering::Release);
        self.decoded_frames.store(0, Ordering::Release);
        #[cfg(windows)]
        if self.config.kind == NativeVideoSourceKind::File && self.info.d3d12_eligible {
            self.info.decoder = "ffmpeg-d3d12va".to_owned();
            match self.start_d3d12_decoder() {
                Ok(()) => return Ok(()),
                Err(error) => {
                    log::warn!("D3D12VA unavailable for this file; falling back to CPU RGBA decode: {error}");
                    self.info.decoder = "ffmpeg-native".to_owned();
                    self.alive.store(true, Ordering::Release);
                }
            }
        }
        let mut command = Command::new(&self.ffmpeg);
        command.args(["-hide_banner", "-loglevel", "error"]);
        append_input_args(&mut command, &self.config, self.position_seconds);
        command.args([
            "-an", "-sn", "-dn", "-f", "rawvideo", "-pix_fmt", "rgba", "-",
        ]);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        hide_console_window(&mut command);
        let mut child = command
            .spawn()
            .map_err(|error| format!("Cannot start FFmpeg video decoder: {error}"))?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| "FFmpeg decoder did not expose stdout".to_owned())?;
        let slot = self.slot.clone();
        let alive = self.alive.clone();
        let decoded_frames = self.decoded_frames.clone();
        let frame_size = self.info.width as usize * self.info.height as usize * 4;
        let frame_interval =
            Duration::from_secs_f64(1.0 / (self.info.fps * self.config.playback_rate));
        let reader = std::thread::Builder::new()
            .name("open-quartz-video-decoder".to_owned())
            .spawn(move || {
                let mut frame = vec![0; frame_size];
                while alive.load(Ordering::Acquire) {
                    let started = Instant::now();
                    if stdout.read_exact(&mut frame).is_err() {
                        break;
                    }
                    if let Ok(mut current) = slot.lock() {
                        std::mem::swap(&mut current.rgba, &mut frame);
                        #[cfg(windows)]
                        {
                            current.d3d12 = None;
                        }
                        frame.resize(frame_size, 0);
                        current.generation = current.generation.saturating_add(1);
                    } else {
                        break;
                    }
                    decoded_frames.fetch_add(1, Ordering::AcqRel);
                    let elapsed = started.elapsed();
                    if elapsed < frame_interval {
                        std::thread::sleep(frame_interval - elapsed);
                    }
                }
                alive.store(false, Ordering::Release);
            })
            .map_err(|error| format!("Cannot start video frame reader: {error}"))?;
        self.child = Some(child);
        self.reader = Some(reader);
        Ok(())
    }

    #[cfg(windows)]
    fn start_d3d12_decoder(&mut self) -> Result<(), String> {
        let config = self.config.clone();
        let position = self.position_seconds;
        let info = self.info.clone();
        let slot = self.slot.clone();
        let alive = self.alive.clone();
        let decoded_frames = self.decoded_frames.clone();
        let (ready_sender, ready_receiver) = std::sync::mpsc::sync_channel(1);
        let reader = std::thread::Builder::new()
            .name("open-quartz-d3d12va-decoder".to_owned())
            .spawn(move || {
                let result = decode_d3d12_frames(
                    &config,
                    position,
                    &info,
                    &slot,
                    &alive,
                    &decoded_frames,
                    ready_sender,
                );
                if let Err(error) = result {
                    log::error!("D3D12VA video decoder stopped: {error}");
                }
                alive.store(false, Ordering::Release);
            })
            .map_err(|error| format!("Cannot start D3D12VA decoder thread: {error}"))?;
        match ready_receiver.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => {
                self.reader = Some(reader);
                Ok(())
            }
            Ok(Err(error)) => {
                self.alive.store(false, Ordering::Release);
                let _ = reader.join();
                Err(error)
            }
            Err(error) => {
                self.alive.store(false, Ordering::Release);
                let _ = reader.join();
                Err(format!("D3D12VA decoder initialization timed out: {error}"))
            }
        }
    }

    fn stop_decoder(&mut self) {
        self.alive.store(false, Ordering::Release);
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.child = None;
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        if self.config.kind == NativeVideoSourceKind::File {
            self.position_seconds +=
                self.decoded_frames.swap(0, Ordering::AcqRel) as f64 / self.info.fps;
        }
    }
}

impl Drop for NativeVideoSource {
    fn drop(&mut self) {
        self.stop_decoder();
    }
}

#[cfg(windows)]
#[repr(C)]
struct FfmpegD3d12Sync {
    fence: *mut std::ffi::c_void,
    event: *mut std::ffi::c_void,
    fence_value: u64,
}

#[cfg(windows)]
#[repr(C)]
struct FfmpegD3d12Frame {
    texture: *mut std::ffi::c_void,
    subresource_index: i32,
    sync: FfmpegD3d12Sync,
    flags: i32,
}

#[cfg(windows)]
unsafe extern "C" fn select_d3d12_format(
    _context: *mut ffmpeg_next::ffi::AVCodecContext,
    formats: *const ffmpeg_next::ffi::AVPixelFormat,
) -> ffmpeg_next::ffi::AVPixelFormat {
    let mut current = formats;
    while !current.is_null()
        && unsafe { *current } != ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_NONE
    {
        if unsafe { *current } == ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_D3D12 {
            return unsafe { *current };
        }
        current = unsafe { current.add(1) };
    }
    ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_NONE
}

fn frame_pacing_delay(
    next_frame_at: &mut Instant,
    now: Instant,
    frame_interval: Duration,
) -> Duration {
    *next_frame_at += frame_interval;
    if now < *next_frame_at {
        *next_frame_at - now
    } else {
        *next_frame_at = now;
        Duration::ZERO
    }
}
fn validate_monotonic_pts(last_pts: &Cell<Option<i64>>, pts: Option<i64>) -> Result<(), String> {
    let Some(pts) = pts else {
        return Ok(());
    };
    if last_pts.get().is_some_and(|previous| pts < previous) {
        return Err(format!(
            "D3D12VA decoder produced out-of-order timestamp {pts} after {}",
            last_pts.get().unwrap_or_default(),
        ));
    }
    last_pts.set(Some(pts));
    Ok(())
}

#[cfg(windows)]
fn decode_d3d12_frames(
    config: &NativeVideoConfig,
    initial_position: f64,
    info: &NativeVideoInfo,
    slot: &Arc<Mutex<FrameSlot>>,
    alive: &AtomicBool,
    decoded_frames: &AtomicU64,
    ready: std::sync::mpsc::SyncSender<Result<(), String>>,
) -> Result<(), String> {
    use ffmpeg_next::{codec, format, media::Type, util::frame::video::Video};
    use std::ffi::CString;

    ffmpeg_next::init().map_err(|error| format!("Cannot initialize libav: {error}"))?;
    let frame_interval = Duration::from_secs_f64(1.0 / (info.fps * config.playback_rate));
    let mut next_frame_at = Instant::now();
    let setup = (|| {
        let mut input = format::input(&config.source)
            .map_err(|error| format!("Cannot open video with libav: {error}"))?;
        let stream = input
            .streams()
            .best(Type::Video)
            .ok_or_else(|| "libav found no video stream".to_owned())?;
        let stream_index = stream.index();
        let mut context = codec::context::Context::from_parameters(stream.parameters())
            .map_err(|error| format!("Cannot create HEVC decoder: {error}"))?;
        let device_name = CString::new("d3d12va").expect("static device name");
        let mut device = std::ptr::null_mut();
        unsafe {
            let device_type = ffmpeg_next::ffi::av_hwdevice_find_type_by_name(device_name.as_ptr());
            if device_type == ffmpeg_next::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_NONE {
                return Err("This libav build does not expose D3D12VA".to_owned());
            }
            let result = ffmpeg_next::ffi::av_hwdevice_ctx_create(
                &mut device,
                device_type,
                std::ptr::null(),
                std::ptr::null_mut(),
                0,
            );
            if result < 0 || device.is_null() {
                return Err(format!("Cannot create FFmpeg D3D12VA device: {result}"));
            }
            let raw = context.as_mut_ptr();
            (*raw).get_format = Some(select_d3d12_format);
            (*raw).hw_device_ctx = ffmpeg_next::ffi::av_buffer_ref(device);
        }
        let decoder = context
            .decoder()
            .video()
            .map_err(|error| format!("Cannot open D3D12VA video decoder: {error}"));
        unsafe { ffmpeg_next::ffi::av_buffer_unref(&mut device) };
        let decoder = decoder?;
        if initial_position > 0.0 {
            let timestamp = (initial_position * f64::from(ffmpeg_next::ffi::AV_TIME_BASE)) as i64;
            input
                .seek(timestamp, ..timestamp)
                .map_err(|error| format!("Cannot seek video decoder: {error}"))?;
        }
        ready
            .send(Ok(()))
            .map_err(|_| "Native runtime dropped D3D12VA startup channel".to_owned())?;
        Ok((input, stream_index, decoder))
    })();
    let (mut input, stream_index, mut decoder) = match setup {
        Ok(value) => value,
        Err(error) => {
            let _ = ready.send(Err(error.clone()));
            return Err(error);
        }
    };
    let last_pts = Cell::new(None);

    let mut publish_frame = |decoded: &mut Video| -> Result<(), String> {
        validate_monotonic_pts(&last_pts, decoded.timestamp())?;
        if decoded.format() != ffmpeg_next::format::Pixel::D3D12 {
            return Err(format!(
                "FFmpeg returned {:?}, not a D3D12 frame",
                decoded.format()
            ));
        }
        let raw = unsafe { &*decoded.as_ptr() };
        let descriptor = unsafe { &*(raw.data[0] as *const FfmpegD3d12Frame) };
        let frame = unsafe {
            D3d12VideoFrame::from_raw(
                descriptor.texture,
                descriptor.sync.fence,
                descriptor.sync.event,
                descriptor.sync.fence_value,
                decoded.width(),
                decoded.height(),
                u32::try_from(descriptor.subresource_index)
                    .map_err(|_| "FFmpeg returned a negative D3D12 subresource".to_owned())?,
                info.d3d12_detach_required,
            )?
        };
        unsafe { ffmpeg_next::ffi::av_frame_unref(decoded.as_mut_ptr()) };
        let mut current = slot
            .lock()
            .map_err(|_| "Native video frame lock is poisoned".to_owned())?;
        current.rgba.clear();
        current.d3d12 = Some(frame);
        current.generation = current.generation.saturating_add(1);
        drop(current);
        decoded_frames.fetch_add(1, Ordering::AcqRel);
        while alive.load(Ordering::Acquire) {
            let consumed = slot
                .lock()
                .map_err(|_| "Native video frame lock is poisoned".to_owned())?
                .d3d12
                .is_none();
            if consumed {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let delay = frame_pacing_delay(&mut next_frame_at, Instant::now(), frame_interval);
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
        Ok(())
    };

    loop {
        if !alive.load(Ordering::Acquire) {
            break;
        }
        let mut decoded = Video::empty();
        for (packet_stream, packet) in input.packets() {
            if !alive.load(Ordering::Acquire) {
                break;
            }
            if packet_stream.index() != stream_index {
                continue;
            }
            decoder
                .send_packet(&packet)
                .map_err(|error| format!("D3D12VA packet decode failed: {error}"))?;
            while decoder.receive_frame(&mut decoded).is_ok() {
                publish_frame(&mut decoded)?;
            }
        }
        if !alive.load(Ordering::Acquire) {
            break;
        }
        decoder
            .send_eof()
            .map_err(|error| format!("D3D12VA decoder drain failed: {error}"))?;
        while decoder.receive_frame(&mut decoded).is_ok() {
            publish_frame(&mut decoded)?;
        }
        if !config.looping {
            break;
        }
        input
            .seek(0, ..)
            .map_err(|error| format!("Cannot seek video loop: {error}"))?;
        decoder.flush();
        last_pts.set(None);
    }
    Ok(())
}

pub fn find_ffmpeg() -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    if let Some(configured) = std::env::var_os("OPEN_QUARTZ_FFMPEG_PATH") {
        candidates.push(PathBuf::from(configured));
    }
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            candidates.push(directory.join("runtime").join(ffmpeg_binary_name()));
        }
    }
    candidates.push(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("runtime")
            .join(ffmpeg_binary_name()),
    );
    candidates.push(installer_binary_path());
    candidates.push(PathBuf::from(ffmpeg_binary_name()));

    for candidate in candidates {
        let mut command = Command::new(&candidate);
        command
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        hide_console_window(&mut command);
        if command.status().is_ok_and(|status| status.success()) {
            return Ok(candidate);
        }
    }
    Err(
        "FFmpeg runtime is unavailable; run npm run prepare:runtime or set OPEN_QUARTZ_FFMPEG_PATH"
            .to_owned(),
    )
}

pub fn read_video_thumbnail(path: &str) -> Result<Vec<u8>, String> {
    if !Path::new(path).is_file() {
        return Err(format!("Video file does not exist: {path}"));
    }
    let ffmpeg = find_ffmpeg()?;
    let mut command = Command::new(ffmpeg);
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-ss",
        "0.1",
        "-i",
        path,
        "-frames:v",
        "1",
        "-vf",
        "scale=480:-2",
        "-f",
        "image2pipe",
        "-vcodec",
        "mjpeg",
        "-q:v",
        "4",
        "-",
    ]);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    hide_console_window(&mut command);
    let output = command
        .output()
        .map_err(|error| format!("Cannot create video thumbnail: {error}"))?;
    if !output.status.success() || output.stdout.is_empty() {
        let detail = String::from_utf8_lossy(&output.stderr);
        return Err(format!("FFmpeg thumbnail extraction failed: {detail}"));
    }
    Ok(output.stdout)
}

fn d3d12_decode_policy(video_line: &str) -> (bool, bool) {
    let detach_required = video_line.contains("h264");
    let codec_supported = detach_required || video_line.contains("hevc");
    let format_supported = video_line.contains("yuv420p")
        || video_line.contains("nv12")
        || video_line.contains("p010");
    (codec_supported && format_supported, detach_required)
}

fn probe_video(ffmpeg: &Path, config: &NativeVideoConfig) -> Result<NativeVideoInfo, String> {
    let mut command = Command::new(ffmpeg);
    command.args(["-hide_banner"]);
    append_input_args(&mut command, config, 0.0);
    command.stdout(Stdio::null()).stderr(Stdio::piped());
    hide_console_window(&mut command);
    let output = command
        .output()
        .map_err(|error| format!("Cannot probe video source: {error}"))?;
    let metadata = String::from_utf8_lossy(&output.stderr);
    let video_line = metadata
        .lines()
        .find(|line| line.contains("Video:"))
        .ok_or_else(|| format!("FFmpeg found no video stream in {}", config.source))?;
    let size = Regex::new(r"(?:^|\s)(\d{2,5})x(\d{2,5})(?:[\s,]|$)")
        .expect("valid video size regex")
        .captures(video_line)
        .ok_or_else(|| format!("Cannot determine video dimensions from: {video_line}"))?;
    let width = size[1]
        .parse::<u32>()
        .map_err(|error| format!("Invalid video width: {error}"))?;
    let height = size[2]
        .parse::<u32>()
        .map_err(|error| format!("Invalid video height: {error}"))?;
    let fps = Regex::new(r"([0-9]+(?:\.[0-9]+)?)\s+fps")
        .expect("valid video fps regex")
        .captures(video_line)
        .and_then(|capture| capture[1].parse::<f64>().ok())
        .filter(|fps| *fps > 0.0)
        .unwrap_or(30.0);
    let (d3d12_eligible, d3d12_detach_required) = d3d12_decode_policy(video_line);
    Ok(NativeVideoInfo {
        width,
        height,
        fps,
        decoder: "ffmpeg-native".to_owned(),
        d3d12_eligible,
        d3d12_detach_required,
    })
}

pub fn list_video_devices() -> Result<Vec<NativeVideoDevice>, String> {
    #[cfg(target_os = "linux")]
    {
        let mut devices = std::fs::read_dir("/dev")
            .map_err(|error| format!("Cannot enumerate /dev video devices: {error}"))?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                name.starts_with("video").then(|| NativeVideoDevice {
                    id: entry.path().to_string_lossy().into_owned(),
                    label: name,
                })
            })
            .collect::<Vec<_>>();
        devices.sort_by(|left, right| left.id.cmp(&right.id));
        return Ok(devices);
    }
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        let ffmpeg = find_ffmpeg()?;
        let mut command = Command::new(ffmpeg);
        command.args(["-hide_banner", "-list_devices", "true"]);
        #[cfg(target_os = "windows")]
        command.args(["-f", "dshow", "-i", "dummy"]);
        #[cfg(target_os = "macos")]
        command.args(["-f", "avfoundation", "-i", ""]);
        command.stdout(Stdio::null()).stderr(Stdio::piped());
        hide_console_window(&mut command);
        let output = command
            .output()
            .map_err(|error| format!("Cannot enumerate native video devices: {error}"))?;
        Ok(parse_video_devices(&String::from_utf8_lossy(
            &output.stderr,
        )))
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn parse_video_devices(output: &str) -> Vec<NativeVideoDevice> {
    #[cfg(target_os = "windows")]
    let pattern = Regex::new(r#"\"([^\"]+)\" \(video\)"#).expect("valid DirectShow device regex");
    #[cfg(target_os = "macos")]
    let pattern = Regex::new(r"\[(\d+)\]\s+(.+)$").expect("valid AVFoundation device regex");
    let mut devices = Vec::new();
    for line in output.lines() {
        let Some(capture) = pattern.captures(line) else {
            continue;
        };
        #[cfg(target_os = "windows")]
        devices.push(NativeVideoDevice {
            id: capture[1].to_owned(),
            label: capture[1].to_owned(),
        });
        #[cfg(target_os = "macos")]
        devices.push(NativeVideoDevice {
            id: capture[1].to_owned(),
            label: capture[2].trim().to_owned(),
        });
    }
    devices
}

fn append_input_args(command: &mut Command, config: &NativeVideoConfig, position: f64) {
    if config.kind == NativeVideoSourceKind::File {
        if config.looping {
            command.args(["-stream_loop", "-1"]);
        }
        if position > 0.0 {
            command.arg("-ss").arg(format!("{position:.6}"));
        }
        command.arg("-i").arg(&config.source);
        return;
    }
    #[cfg(target_os = "windows")]
    command.args(["-f", "dshow", "-i", &format!("video={}", config.source)]);
    #[cfg(target_os = "macos")]
    command.args([
        "-f",
        "avfoundation",
        "-i",
        &format!("{}:none", config.source),
    ]);
    #[cfg(target_os = "linux")]
    command.args(["-f", "v4l2", "-i", &config.source]);
}

fn installer_binary_path() -> PathBuf {
    let platform = if cfg!(target_os = "windows") {
        "win32"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    };
    let architecture = if cfg!(target_arch = "x86_64") {
        "x64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "ia32"
    };
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../node_modules/@ffmpeg-installer")
        .join(format!("{platform}-{architecture}"))
        .join(ffmpeg_binary_name())
}

fn ffmpeg_binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    }
}

fn hide_console_window(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        d3d12_decode_policy, find_ffmpeg, frame_pacing_delay, read_video_thumbnail,
        validate_monotonic_pts, NativeVideoConfig, NativeVideoFrame, NativeVideoSource,
        NativeVideoSourceKind,
    };
    use std::cell::Cell;
    use std::process::Command;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    #[test]
    fn rejects_regressing_decoder_timestamps_without_advancing_the_clock() {
        let last_pts = Cell::new(None);
        assert!(validate_monotonic_pts(&last_pts, Some(10)).is_ok());
        assert!(validate_monotonic_pts(&last_pts, Some(10)).is_ok());
        assert!(validate_monotonic_pts(&last_pts, Some(11)).is_ok());
        assert!(validate_monotonic_pts(&last_pts, Some(9)).is_err());
        assert_eq!(last_pts.get(), Some(11));
    }

    #[test]
    fn d3d12_pacing_counts_decode_time_and_skips_sleep_when_late() {
        let frame_interval = Duration::from_millis(33);
        let started_at = Instant::now();
        let mut next_frame_at = started_at;

        assert_eq!(
            frame_pacing_delay(
                &mut next_frame_at,
                started_at + Duration::from_millis(8),
                frame_interval,
            ),
            Duration::from_millis(25),
        );
        assert_eq!(
            frame_pacing_delay(
                &mut next_frame_at,
                started_at + Duration::from_millis(44),
                frame_interval,
            ),
            Duration::from_millis(22),
        );
        assert_eq!(
            frame_pacing_delay(
                &mut next_frame_at,
                started_at + Duration::from_millis(100),
                frame_interval,
            ),
            Duration::ZERO,
        );
        assert_eq!(next_frame_at, started_at + Duration::from_millis(100));
    }

    #[test]
    fn keeps_h264_detached_but_hevc_on_direct_d3d12_import() {
        assert_eq!(
            d3d12_decode_policy("Video: h264 (High), yuv420p, 1920x1080"),
            (true, true),
        );
        assert_eq!(
            d3d12_decode_policy("Video: hevc (Main 10), yuv420p10le, 7680x3840"),
            (true, false),
        );
        assert_eq!(
            d3d12_decode_policy("Video: vp9, yuv420p, 1920x1080"),
            (false, false),
        );
    }

    #[test]
    fn decodes_file_frames_without_webview_pixel_ipc() {
        let ffmpeg = find_ffmpeg().unwrap();
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("open-quartz-video-{suffix}.mp4"));
        let status = Command::new(&ffmpeg)
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
            .unwrap();
        assert!(status.success());
        let thumbnail = read_video_thumbnail(&path.to_string_lossy()).unwrap();
        assert_eq!(thumbnail.get(..2), Some([0xff, 0xd8].as_slice()));
        assert_eq!(
            thumbnail.get(thumbnail.len().saturating_sub(2)..),
            Some([0xff, 0xd9].as_slice()),
        );

        let mut source = NativeVideoSource::open(NativeVideoConfig {
            kind: NativeVideoSourceKind::File,
            source: path.to_string_lossy().into_owned(),
            looping: true,
            playback_rate: 1.0,
        })
        .unwrap();
        assert_eq!((source.info().width, source.info().height), (16, 16));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut frames = 0;
        let mut first_pixel = None;
        while std::time::Instant::now() < deadline && frames < 2 {
            source
                .upload_latest(|frame, width, height| {
                    let NativeVideoFrame::Rgba(rgba) = frame else {
                        panic!("8-bit test video must use CPU RGBA fallback");
                    };
                    assert_eq!(rgba.len(), width as usize * height as usize * 4);
                    assert_eq!((width, height), (16, 16));
                    if frames == 0 {
                        first_pixel = Some(rgba[..4].to_vec());
                    }
                    frames += 1;
                    Ok(())
                })
                .unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(frames >= 2, "decoder must produce multiple complete frames");
        let metrics = source.metrics();
        assert!(metrics.decoded_frames >= metrics.uploaded_frames);
        assert!(metrics.cpu_copy_bytes >= metrics.uploaded_frames * 16 * 16 * 4);
        let pixel = first_pixel.expect("decoder must produce a frame");
        assert!(pixel[0] > 240 && pixel[1] < 16 && pixel[2] < 16 && pixel[3] == 255);
        source.pause();
        assert!(source.child.is_none());
        let uploaded_before_resume = source.metrics().uploaded_frames;
        source.resume().unwrap();
        let resume_deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < resume_deadline
            && source.metrics().uploaded_frames == uploaded_before_resume
        {
            source.upload_latest(|_, _, _| Ok(())).unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(source.metrics().uploaded_frames > uploaded_before_resume);
        drop(source);
        let _ = std::fs::remove_file(path);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn parses_directshow_video_devices_without_audio_or_aliases() {
        let devices = super::parse_video_devices(
            r#"[dshow @ 0001] "Integrated Camera" (video)
[dshow @ 0001]   Alternative name "@device_pnp_camera"
[dshow @ 0001] "Microphone Array" (audio)"#,
        );
        assert_eq!(
            devices,
            vec![super::NativeVideoDevice {
                id: "Integrated Camera".to_owned(),
                label: "Integrated Camera".to_owned(),
            }]
        );
    }
}
