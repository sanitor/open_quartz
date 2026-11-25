use super::EncoderSurface;
use super::{
    AudioRender, EncodeAdmission, EncodeFrame, EncoderConfig, EncoderProfile, EncoderReport, EncoderStatus,
};
use crate::{MediaError, RationalTimeBase};
use ffmpeg_next::{codec, format, Dictionary, Packet};
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[path = "mp4_audio.rs"]
mod audio;
use audio::AudioEncoder;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);
enum Command {
    Frame(EncodeFrame, InputCredit),
    Finish,
    Commit,
}
struct InputCredit {
    count: Arc<AtomicUsize>,
    waker: Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>,
}
impl Drop for InputCredit {
    fn drop(&mut self) {
        self.count.fetch_sub(1, Ordering::AcqRel);
        let callback = self
            .waker
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().cloned());
        if let Some(callback) = callback {
            callback();
        }
    }
}

pub struct Mp4Encoder {
    tx: mpsc::SyncSender<Command>,
    status: Arc<Mutex<EncoderStatus>>,
    lifecycle: Arc<AtomicU8>,
    inflight: Arc<AtomicUsize>,
    waker: Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>,
    worker: Option<JoinHandle<()>>,
    config: EncoderConfig,
    target: PathBuf,
    last_pts: Option<i64>,
    finish_sent: bool,
    profile: EncoderProfile,
}
impl Mp4Encoder {
    pub fn open(path: impl AsRef<Path>, config: EncoderConfig) -> Result<Self, MediaError> {
        Self::open_with_audio(path, config, None)
    }

    pub fn open_with_audio(
        path: impl AsRef<Path>,
        config: EncoderConfig,
        audio: Option<AudioRender>,
    ) -> Result<Self, MediaError> {
        Self::open_with_profile(path, config, audio, EncoderProfile::Sdr)
    }

    pub fn open_with_profile(
        path: impl AsRef<Path>, config: EncoderConfig, audio: Option<AudioRender>, profile: EncoderProfile,
    ) -> Result<Self, MediaError> {
        Self::start(path, config, audio, profile, |file| file)
    }

    #[cfg(test)]
    fn open_with_writer<W, F>(
        path: impl AsRef<Path>,
        config: EncoderConfig,
        make_writer: F,
    ) -> Result<Self, MediaError>
    where
        W: std::io::Write + std::io::Seek + Send + 'static,
        F: FnOnce(File) -> W + Send + 'static,
    {
        Self::start(path, config, None, EncoderProfile::Sdr, make_writer)
    }

    fn start<W, F>(
        path: impl AsRef<Path>,
        config: EncoderConfig,
        audio: Option<AudioRender>,
        profile: EncoderProfile,
        make_writer: F,
    ) -> Result<Self, MediaError>
    where
        W: std::io::Write + std::io::Seek + Send + 'static,
        F: FnOnce(File) -> W + Send + 'static,
    {
        validate(&config)?;
        let target = path.as_ref().to_path_buf();
        if !target
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"))
        {
            return Err("H.264 export target must have an .mp4 extension".into());
        }
        if target.exists() && !config.overwrite {
            return Err("Export target already exists and overwrite is disabled".into());
        }
        let parent = target
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        if !parent.is_dir() {
            return Err("Export target directory does not exist".into());
        }
        let (temp, temp_guard) = create_temp(parent)?;
        let inflight = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = mpsc::sync_channel(config.queue_capacity);
        let status = Arc::new(Mutex::new(EncoderStatus::Running));
        let lifecycle = Arc::new(AtomicU8::new(0));
        let waker = Arc::new(Mutex::new(None));
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread_status = status.clone();
        let thread_lifecycle = lifecycle.clone();
        let thread_waker = waker.clone();
        let thread_target = target.clone();
        let thread_config = config.clone();
        let worker = thread::Builder::new()
            .name("oq-videotoolbox-mp4-encoder".into())
            .spawn(move || {
                let writer = match temp_guard.try_clone() {
                    Ok(file) => file,
                    Err(error) => {
                        let _ = fs::remove_file(&temp);
                        let _ = ready_tx.send(Err(error.to_string()));
                        return;
                    }
                };
                let writer = make_writer(writer);
                let mut pipeline = match Pipeline::open(&temp, writer, &thread_config, audio, profile) {
                    Ok(pipeline) => pipeline,
                    Err(error) => {
                        let _ = fs::remove_file(&temp);
                        set_status(
                            &thread_status,
                            &thread_waker,
                            EncoderStatus::Failed(error.clone()),
                        );
                        let _ = ready_tx.send(Err(error.to_string()));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(()));
                let mut inputs = 0u64;
                let mut packets = 0u64;
                loop {
                    if thread_lifecycle.load(Ordering::Acquire) == 1 {
                        drop(pipeline);
                        let _ = fs::remove_file(&temp);
                        set_status(&thread_status, &thread_waker, EncoderStatus::Cancelled);
                        return;
                    }
                    match rx.recv_timeout(Duration::from_millis(20)) {
                        Ok(Command::Frame(frame, credit)) => match pipeline.encode(frame, credit) {
                            Ok(count) => {
                                inputs += 1;
                                packets += count;
                            }
                            Err(error) => {
                                drop(pipeline);
                                let _ = fs::remove_file(&temp);
                                set_status(
                                    &thread_status,
                                    &thread_waker,
                                    if error.kind == crate::ErrorKind::Cancelled {
                                        EncoderStatus::Cancelled
                                    } else {
                                        EncoderStatus::Failed(error)
                                    },
                                );
                                return;
                            }
                        },
                        Ok(Command::Finish) => {
                            set_status(&thread_status, &thread_waker, EncoderStatus::Draining);
                            match pipeline.drain() {
                                Ok(count) => packets += count,
                                Err(error) => {
                                    drop(pipeline);
                                    let _ = fs::remove_file(&temp);
                                    set_status(
                                        &thread_status,
                                        &thread_waker,
                                        if error.kind == crate::ErrorKind::Cancelled {
                                            EncoderStatus::Cancelled
                                        } else {
                                            EncoderStatus::Failed(error)
                                        },
                                    );
                                    return;
                                }
                            }
                            let report = EncoderReport {
                                input_frames: inputs,
                                packets,
                                hardware: true,
                                encoder: format!("{} (allow_sw=0, require_sw=0)", profile.codec()),
                                time_base: thread_config.time_base,
                            };
                            drop(pipeline);
                            if let Err(error) = temp_guard.sync_all() {
                                let _ = fs::remove_file(&temp);
                                set_status(
                                    &thread_status,
                                    &thread_waker,
                                    EncoderStatus::Failed(
                                        format!("Cannot fsync MP4 temporary: {error}").into(),
                                    ),
                                );
                                return;
                            }
                            set_status(
                                &thread_status,
                                &thread_waker,
                                EncoderStatus::ReadyToCommit(report.clone()),
                            );
                            loop {
                                if thread_lifecycle.load(Ordering::Acquire) == 1 {
                                    let _ = fs::remove_file(&temp);
                                    set_status(
                                        &thread_status,
                                        &thread_waker,
                                        EncoderStatus::Cancelled,
                                    );
                                    return;
                                }
                                match rx.recv_timeout(Duration::from_millis(20)) {
                                    Ok(Command::Commit) => {
                                        if thread_lifecycle
                                            .compare_exchange(
                                                0,
                                                2,
                                                Ordering::AcqRel,
                                                Ordering::Acquire,
                                            )
                                            .is_err()
                                        {
                                            let _ = fs::remove_file(&temp);
                                            set_status(
                                                &thread_status,
                                                &thread_waker,
                                                EncoderStatus::Cancelled,
                                            );
                                            return;
                                        }
                                        if let Err(error) = atomic_commit(
                                            &temp,
                                            &thread_target,
                                            thread_config.overwrite,
                                            &temp_guard,
                                        ) {
                                            thread_lifecycle.store(3, Ordering::Release);
                                            let _ = fs::remove_file(&temp);
                                            set_status(
                                                &thread_status,
                                                &thread_waker,
                                                EncoderStatus::Failed(error.clone()),
                                            );
                                            return;
                                        }
                                        thread_lifecycle.store(3, Ordering::Release);
                                        set_status(
                                            &thread_status,
                                            &thread_waker,
                                            EncoderStatus::Completed(report),
                                        );
                                        return;
                                    }
                                    Ok(_) => {
                                        let _ = fs::remove_file(&temp);
                                        set_status(
                                            &thread_status,
                                            &thread_waker,
                                            EncoderStatus::Failed(
                                                "Invalid encoder command after drain".into(),
                                            ),
                                        );
                                        return;
                                    }
                                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                                        let _ = fs::remove_file(&temp);
                                        set_status(
                                            &thread_status,
                                            &thread_waker,
                                            EncoderStatus::Cancelled,
                                        );
                                        return;
                                    }
                                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                                }
                            }
                        }
                        Ok(Command::Commit) => {
                            let _ = fs::remove_file(&temp);
                            set_status(
                                &thread_status,
                                &thread_waker,
                                EncoderStatus::Failed("Cannot commit before encoder drain".into()),
                            );
                            return;
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            drop(pipeline);
                            let _ = fs::remove_file(&temp);
                            set_status(&thread_status, &thread_waker, EncoderStatus::Cancelled);
                            return;
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                }
            })
            .map_err(|e| format!("Cannot start encoder worker: {e}"))?;
        match ready_rx
            .recv()
            .map_err(|_| MediaError::from("Encoder worker exited during startup"))?
        {
            Ok(()) => Ok(Self {
                tx,
                status,
                lifecycle,
                inflight,
                waker,
                worker: Some(worker),
                config,
                target,
                last_pts: None,
                finish_sent: false,
            profile,
            }),
            Err(error) => {
                let _ = worker.join();
                Err(error.into())
            }
        }
    }
    pub fn submit(&mut self, frame: EncodeFrame) -> Result<EncodeAdmission, MediaError> {
        if frame.surface.profile() != self.profile {
            return Err("Encoder surface does not match the selected profile".into());
        }
        if self.lifecycle.load(Ordering::Acquire) != 0 {
            return Err("Encoder is cancelled or committing".into());
        }
        if self.finish_sent {
            return Err("Cannot submit after finish".into());
        }
        if frame.surface.width() != self.config.width
            || frame.surface.height() != self.config.height
        {
            return Err("Encoder surface dimensions do not match configuration".into());
        }
        if frame.duration <= 0 || frame.pts < 0 || self.last_pts.is_some_and(|pts| frame.pts <= pts)
        {
            return Err("Encoder timestamps must be non-negative and strictly increasing with positive durations".into());
        }
        frame
            .pts
            .checked_add(frame.duration)
            .and_then(|end| end.checked_mul(i64::from(self.config.time_base.numerator)))
            .ok_or("Encoder timestamp exceeds CoreMedia capacity")?;
        let mut current = self.inflight.load(Ordering::Acquire);
        loop {
            if current >= self.config.queue_capacity {
                return Ok(EncodeAdmission::WouldBlock(frame));
            }
            match self.inflight.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
        let credit = InputCredit {
            count: self.inflight.clone(),
            waker: self.waker.clone(),
        };
        let pts = frame.pts;
        match self.tx.try_send(Command::Frame(frame, credit)) {
            Ok(()) => {
                self.last_pts = Some(pts);
                Ok(EncodeAdmission::Accepted)
            }
            Err(mpsc::TrySendError::Full(Command::Frame(frame, credit))) => {
                drop(credit);
                Ok(EncodeAdmission::WouldBlock(frame))
            }
            Err(mpsc::TrySendError::Disconnected(Command::Frame(_, credit))) => {
                drop(credit);
                Err("Encoder worker is unavailable".into())
            }
            Err(mpsc::TrySendError::Full(_)) | Err(mpsc::TrySendError::Disconnected(_)) => {
                unreachable!()
            }
        }
    }
    pub fn in_flight(&self) -> usize {
        self.inflight.load(Ordering::Acquire)
    }
    pub fn finish(&mut self) -> Result<bool, MediaError> {
        if self.finish_sent {
            return Ok(true);
        }
        match self.tx.try_send(Command::Finish) {
            Ok(()) => {
                self.finish_sent = true;
                Ok(true)
            }
            Err(mpsc::TrySendError::Full(_)) => Ok(false),
            Err(mpsc::TrySendError::Disconnected(_)) => Err("Encoder worker is unavailable".into()),
        }
    }
    pub fn poll(&self) -> Result<EncoderStatus, MediaError> {
        self.status
            .lock()
            .map(|s| s.clone())
            .map_err(|_| MediaError::from("Encoder status lock is poisoned"))
    }
    pub fn commit(&mut self) -> Result<bool, MediaError> {
        if !matches!(self.poll()?, EncoderStatus::ReadyToCommit(_)) {
            return Ok(false);
        }
        match self.tx.try_send(Command::Commit) {
            Ok(()) => Ok(true),
            Err(mpsc::TrySendError::Full(_)) => Ok(false),
            Err(mpsc::TrySendError::Disconnected(_)) => Err("Encoder worker is unavailable".into()),
        }
    }
    pub fn cancel(&self) {
        let _ = self
            .lifecycle
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire);
    }
    pub fn set_waker(&mut self, waker: Arc<dyn Fn() + Send + Sync>) {
        if let Ok(mut current) = self.waker.lock() {
            *current = Some(waker);
        }
    }
    pub fn target(&self) -> &Path {
        &self.target
    }
}
impl Drop for Mp4Encoder {
    fn drop(&mut self) {
        if !matches!(
            self.status.lock().as_deref(),
            Ok(EncoderStatus::Completed(_) | EncoderStatus::Cancelled | EncoderStatus::Failed(_))
        ) {
            self.cancel();
        }
        // Joining is permitted only on the dedicated export worker, never a render owner.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn validate(c: &EncoderConfig) -> Result<(), MediaError> {
    if c.width == 0
        || c.height == 0
        || c.width > i32::MAX as u32
        || c.height > i32::MAX as u32
        || c.width % 2 != 0
        || c.height % 2 != 0
    {
        return Err("H.264 NV12 dimensions must be positive even signed 32-bit values".into());
    }
    if c.time_base.numerator <= 0 || c.time_base.denominator <= 0 {
        return Err("Encoder time base must be positive".into());
    }
    if c.bitrate == 0 || c.bitrate > i32::MAX as u64 {
        return Err("VideoToolbox bitrate must fit a positive signed 32-bit value".into());
    }
    let reorder_depth = if c.max_b_frames == 0 { 0 } else { 2 };
    if c.max_b_frames > 2 || c.queue_capacity < reorder_depth + 2 {
        return Err("Encoder queue must hold native reorder depth + 2 inputs (B-frame reorder depth is two)".into());
    }
    Ok(())
}
fn create_temp(parent: &Path) -> Result<(PathBuf, File), MediaError> {
    use std::os::unix::fs::OpenOptionsExt;
    for _ in 0..64 {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let name = format!(".oq-export-{}-{id}.mp4", std::process::id());
        let candidate = parent.join(name);
        match OpenOptions::new()
            .read(true)
            .write(true)
            .mode(0o600)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Cannot create export temporary: {e}").into()),
        }
    }
    Err("Cannot reserve a unique export temporary".into())
}
fn set_status(
    status: &Mutex<EncoderStatus>,
    waker: &Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    next: EncoderStatus,
) {
    if let Ok(mut s) = status.lock() {
        *s = next;
    }
    let callback = waker.lock().ok().and_then(|guard| guard.as_ref().cloned());
    if let Some(callback) = callback {
        callback();
    }
}
fn atomic_commit(
    temp: &Path,
    target: &Path,
    overwrite: bool,
    guard: &File,
) -> Result<(), MediaError> {
    use std::os::unix::fs::MetadataExt;
    let opened = guard
        .metadata()
        .map_err(|e| format!("Cannot inspect export temporary: {e}"))?;
    let named = fs::symlink_metadata(temp)
        .map_err(|e| format!("Cannot inspect export temporary path: {e}"))?;
    if !named.file_type().is_file() || opened.dev() != named.dev() || opened.ino() != named.ino() {
        return Err(
            "Export temporary path no longer identifies the exclusively created file".into(),
        );
    }
    use std::os::unix::ffi::OsStrExt;
    let from = CString::new(temp.as_os_str().as_bytes())
        .map_err(|_| MediaError::from("Invalid temporary path"))?;
    let to = CString::new(target.as_os_str().as_bytes())
        .map_err(|_| MediaError::from("Invalid export path"))?;
    unsafe extern "C" {
        fn renamex_np(from: *const i8, to: *const i8, flags: u32) -> i32;
        fn rename(from: *const i8, to: *const i8) -> i32;
    }
    let result = unsafe {
        if overwrite {
            rename(from.as_ptr(), to.as_ptr())
        } else {
            renamex_np(from.as_ptr(), to.as_ptr(), 0x00000004)
        }
    };
    if result != 0 {
        return Err(format!(
            "Atomic MP4 commit failed: {}",
            std::io::Error::last_os_error()
        )
        .into());
    }
    Ok(())
}
struct Pipeline {
    output: format::context::Output,
    encoder: codec::encoder::video::Encoder,
    stream: usize,
    time_base: RationalTimeBase,
    audio: Option<AudioEncoder>,
    end_pts: i64,
    profile: EncoderProfile,
}
impl Pipeline {
    fn open(
        _path: &Path,
        writer: impl std::io::Write + std::io::Seek + Send + 'static,
        c: &EncoderConfig,
        render_audio: Option<AudioRender>,
        profile: EncoderProfile,
    ) -> Result<Self, MediaError> {
        crate::require_media_sdk()?;
        let custom_io = format::context::StreamIo::from_write_seek(writer)
            .map_err(|e| format!("Cannot create MP4 AVIO: {e}"))?;
        let mut output = format::output_to_stream(custom_io, None, Some("mp4"))
            .map_err(|e| format!("Cannot open MP4 muxer: {e}"))?;
        let codec = codec::encoder::find_by_name(profile.codec())
            .ok_or_else(|| format!("FFmpeg {} encoder is unavailable", profile.codec()))?;
        let mut video = codec::context::Context::new_with_codec(codec)
            .encoder()
            .video()
            .map_err(|e| format!("Cannot create VideoToolbox encoder: {e}"))?;
        video.set_width(c.width);
        video.set_height(c.height);
        video.set_format(format::Pixel::VIDEOTOOLBOX);
        video.set_time_base((c.time_base.numerator, c.time_base.denominator));
        video.set_frame_rate(Some((c.time_base.denominator, c.time_base.numerator)));
        video.set_bit_rate(c.bitrate as usize);
        video.set_max_b_frames(c.max_b_frames as usize);
        video.set_color_range(ffmpeg_next::color::Range::MPEG);
        let (space, primaries, transfer) = color_tags(profile);
        video.set_colorspace(space);
        video.set_color_primaries(primaries);
        video.set_color_transfer_characteristic(transfer);
        video.set_flags(codec::Flags::GLOBAL_HEADER);
        unsafe {
            (*video.as_mut_ptr()).sw_pix_fmt = match profile {
                EncoderProfile::Sdr => ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_NV12,
                EncoderProfile::Hdr10 | EncoderProfile::Hlg => ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_P010LE,
            };
        }
        let mut options = Dictionary::new();
        options.set("allow_sw", "0");
        options.set("require_sw", "0");
        if profile.is_hdr() { options.set("profile", "main10"); }
        let encoder = video
            .open_with(options)
            .map_err(|e| format!("Cannot open required VideoToolbox encoder: {e}"))?;
        let mut stream = output
            .add_stream(codec)
            .map_err(|e| format!("Cannot add video stream: {e}"))?;
        stream.set_time_base((c.time_base.numerator, c.time_base.denominator));
        stream.set_parameters(&encoder);
        if profile.is_hdr() {
            unsafe {
                let parameters = (*stream.as_mut_ptr()).codecpar;
                (*parameters).codec_tag = u32::from_le_bytes(*b"hvc1");
                if profile == EncoderProfile::Hdr10 { declare_mastering_display(parameters)?; }
            }
        }
        let stream_index = stream.index();
        drop(stream);
        let audio = render_audio
            .map(|render| AudioEncoder::open(&mut output, render))
            .transpose()?;
        output
            .write_header()
            .map_err(|e| format!("Cannot write MP4 header: {e}"))?;
        Ok(Self {
            output,
            encoder,
            stream: stream_index,
            time_base: c.time_base,
            audio,
            end_pts: 0,
            profile,
        })
    }
    fn encode(&mut self, input: EncodeFrame, credit: InputCredit) -> Result<u64, MediaError> {
        self.end_pts = input
            .pts
            .checked_add(input.duration)
            .ok_or("Encoder timestamp overflow")?;
        let mut frame = ffmpeg_next::util::frame::video::Video::empty();
        frame.set_color_range(ffmpeg_next::color::Range::MPEG);
        let (space, primaries, transfer) = color_tags(self.profile);
        frame.set_color_space(space);
        frame.set_color_primaries(primaries);
        frame.set_color_transfer_characteristic(transfer);
        unsafe {
            let raw = &mut *frame.as_mut_ptr();
            raw.width = input.surface.width() as i32;
            raw.height = input.surface.height() as i32;
            raw.format = ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX as i32;
            raw.pts = input.pts;
            raw.duration = input.duration;
            let owner = Box::into_raw(Box::new((input.surface, credit)));
            let pixel = (*owner).0.pixel_buffer as *mut u8;
            let allocation_size = (*owner).0.allocation_size() as usize;
            let buf = ffmpeg_next::ffi::av_buffer_create(
                pixel,
                allocation_size,
                Some(release_surface),
                owner.cast(),
                0,
            );
            if buf.is_null() {
                drop(Box::from_raw(owner));
                return Err("Cannot retain encoder surface in AVFrame".into());
            }
            raw.buf[0] = buf;
            raw.data[3] = pixel;
        }
        let mut packets = 0;
        loop {
            match self.encoder.send_frame(&frame) {
                Ok(()) => break,
                Err(ffmpeg_next::Error::Other {
                    errno: ffmpeg_next::error::EAGAIN,
                }) => {
                    packets += self.packets()?;
                }
                Err(e) => return Err(format!("VideoToolbox rejected frame: {e}").into()),
            }
        }
        packets += self.packets()?;
        if let Some(audio) = &mut self.audio {
            let end = crate::audio_sample(self.end_pts, self.time_base)?;
            audio.encode_until(&mut self.output, end, false)?;
        }
        Ok(packets)
    }
    fn packets(&mut self) -> Result<u64, MediaError> {
        let mut count = 0;
        loop {
            let mut packet = Packet::empty();
            match self.encoder.receive_packet(&mut packet) {
                Ok(()) => {
                    packet.set_stream(self.stream);
                    let output_time_base = self
                        .output
                        .stream(self.stream)
                        .ok_or("Missing MP4 stream")?
                        .time_base();
                    let output_tb = RationalTimeBase {
                        numerator: output_time_base.0,
                        denominator: output_time_base.1,
                    };
                    let pts = packet.pts().ok_or("H.264 encoder packet has no PTS")?;
                    let dts = packet.dts().ok_or("H.264 encoder packet has no DTS")?;
                    packet.set_pts(Some(checked_rescale(pts, self.time_base, output_tb)?));
                    packet.set_dts(Some(checked_rescale(dts, self.time_base, output_tb)?));
                    let duration = packet.duration();
                    packet.set_duration(if duration <= 0 {
                        checked_rescale(1, self.time_base, output_tb)?.max(1)
                    } else {
                        checked_rescale(duration, self.time_base, output_tb)?
                    });
                    packet
                        .write_interleaved(&mut self.output)
                        .map_err(|e| format!("Cannot mux encoded packet: {e}"))?;
                    count += 1;
                }
                Err(ffmpeg_next::Error::Other {
                    errno: ffmpeg_next::error::EAGAIN,
                })
                | Err(ffmpeg_next::Error::Eof) => break,
                Err(e) => return Err(format!("Cannot receive H.264 packet: {e}").into()),
            }
        }
        Ok(count)
    }
    fn drain(&mut self) -> Result<u64, MediaError> {
        let mut packets = 0;
        loop {
            match self.encoder.send_eof() {
                Ok(()) => break,
                Err(ffmpeg_next::Error::Other {
                    errno: ffmpeg_next::error::EAGAIN,
                }) => {
                    packets += self.packets()?;
                }
                Err(ffmpeg_next::Error::Eof) => break,
                Err(e) => return Err(format!("Cannot send encoder EOF: {e}").into()),
            }
        }
        loop {
            let count = self.packets()?;
            packets += count;
            if count == 0 {
                break;
            }
        }
        if let Some(audio) = &mut self.audio {
            let end = crate::audio_sample(self.end_pts, self.time_base)?;
            audio.encode_until(&mut self.output, end, true)?;
            audio.finish(&mut self.output)?;
        }
        self.output
            .write_trailer()
            .map_err(|e| format!("Cannot finish MP4 trailer: {e}"))?;
        Ok(packets)
    }
}
fn color_tags(profile: EncoderProfile) -> (ffmpeg_next::color::Space, ffmpeg_next::color::Primaries, ffmpeg_next::color::TransferCharacteristic) {
    use ffmpeg_next::color::{Primaries, Space, TransferCharacteristic as Transfer};
    match profile {
        EncoderProfile::Sdr => (Space::BT709, Primaries::BT709, Transfer::BT709),
        EncoderProfile::Hdr10 => (Space::BT2020NCL, Primaries::BT2020, Transfer::SMPTE2084),
        EncoderProfile::Hlg => (Space::BT2020NCL, Primaries::BT2020, Transfer::ARIB_STD_B67),
    }
}

unsafe fn declare_mastering_display(parameters: *mut ffmpeg_next::ffi::AVCodecParameters) -> Result<(), MediaError> {
    use ffmpeg_next::ffi as av;
    // The HDR10 preset explicitly declares a BT.2020/D65 mastering target.
    // Content-light statistics have an independent meaning and are left unset.
    #[repr(C)]
    struct MasteringDisplay {
        display_primaries: [[av::AVRational; 2]; 3],
        white_point: [av::AVRational; 2],
        min_luminance: av::AVRational,
        max_luminance: av::AVRational,
        has_primaries: std::ffi::c_int,
        has_luminance: std::ffi::c_int,
    }
    unsafe extern "C" {
        fn av_mastering_display_metadata_alloc_size(size: *mut usize) -> *mut MasteringDisplay;
    }
    let mut size = 0;
    let buffer = unsafe { av_mastering_display_metadata_alloc_size(&mut size) };
    if buffer.is_null() { return Err("Cannot allocate HDR10 mastering metadata".into()); }
    let metadata = unsafe { &mut *buffer };
    let xy = |num| av::AVRational { num, den: 50_000 };
    metadata.display_primaries = [[xy(35400), xy(14600)], [xy(8500), xy(39850)], [xy(6550), xy(2300)]];
    metadata.white_point = [xy(15635), xy(16450)];
    metadata.min_luminance = av::AVRational { num: 5, den: 1000 };
    metadata.max_luminance = av::AVRational { num: 1000, den: 1 };
    metadata.has_primaries = 1;
    metadata.has_luminance = 1;
    let attached = unsafe { av::av_packet_side_data_add(
        &mut (*parameters).coded_side_data,
        &mut (*parameters).nb_coded_side_data,
        av::AVPacketSideDataType::AV_PKT_DATA_MASTERING_DISPLAY_METADATA,
        buffer.cast(), size, 0,
    ) };
    if attached.is_null() {
        unsafe { av::av_free(buffer.cast()); }
        return Err("Cannot attach HDR10 mastering metadata".into());
    }
    Ok(())
}

unsafe extern "C" fn release_surface(opaque: *mut std::ffi::c_void, _data: *mut u8) {
    drop(unsafe { Box::from_raw(opaque.cast::<(EncoderSurface, InputCredit)>()) });
}

fn checked_rescale(
    value: i64,
    source: RationalTimeBase,
    destination: RationalTimeBase,
) -> Result<i64, MediaError> {
    let numerator = i128::from(value)
        .checked_mul(i128::from(source.numerator))
        .and_then(|v| v.checked_mul(i128::from(destination.denominator)))
        .ok_or("Timestamp rescaling overflow")?;
    let denominator = i128::from(source.denominator)
        .checked_mul(i128::from(destination.numerator))
        .ok_or("Timestamp rescaling overflow")?;
    if denominator <= 0 {
        return Err("Timestamp rescaling requires positive time bases".into());
    }
    let rounded = numerator
        .checked_abs()
        .and_then(|v| v.checked_add(denominator / 2))
        .ok_or("Timestamp rescaling overflow")?
        / denominator;
    i64::try_from(if numerator < 0 { -rounded } else { rounded })
        .map_err(|_| "Timestamp rescaling exceeds i64".into())
}

#[cfg(test)]
#[path = "mp4_tests.rs"]
mod tests;
