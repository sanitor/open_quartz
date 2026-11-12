use crate::{
    ColorInfo, ColorMatrix, ColorPrimaries, ErrorKind, FrameSelection, MediaError, PixelFormat,
    RationalTimeBase, SelectedFrame, SourceInfo, SourceTimestamp, TransferFunction,
};
use ffmpeg_next::ffi;
use ffmpeg_next::{codec, format, media::Type, util::frame::video::Video, Packet};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};
use std::{
    ffi::{c_void, CStr, CString},
    path::{Path, PathBuf},
    ptr,
};
static NEXT_SOURCE_EPOCH: AtomicU64 = AtomicU64::new(1);

#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
    fn CVPixelBufferGetPixelFormatType(buffer: *mut c_void) -> u32;
    fn CVPixelBufferGetIOSurface(buffer: *mut c_void) -> *mut c_void;
    fn CVPixelBufferGetDataSize(buffer: *mut c_void) -> usize;
}
#[link(name = "IOSurface", kind = "framework")]
extern "C" {
    fn IOSurfaceGetAllocSize(surface: *mut c_void) -> usize;
}

/// Owns the FFmpeg frame reference, and therefore its immutable VideoToolbox surface.
pub struct VideoToolboxFrame {
    raw: *mut ffi::AVFrame,
    format: PixelFormat,
    color: ColorInfo,
    pts_ns: Option<i64>,
    timestamp: Option<SourceTimestamp>,
}
// The retained AVFrame and CVPixelBuffer are immutable after publication. FFmpeg's
// buffer references are thread-safe and the consumer never receives mutable access.
unsafe impl Send for VideoToolboxFrame {}
impl Drop for VideoToolboxFrame {
    fn drop(&mut self) {
        unsafe { ffi::av_frame_free(&mut self.raw) }
    }
}
impl VideoToolboxFrame {
    pub fn try_clone(&self) -> Result<Self, String> {
        let raw = unsafe { ffi::av_frame_clone(self.raw) };
        if raw.is_null() {
            return Err("Cannot retain VideoToolbox frame".into());
        }
        Ok(Self {
            raw,
            format: self.format,
            color: self.color,
            pts_ns: self.pts_ns,
            timestamp: self.timestamp,
        })
    }
    pub fn pixel_buffer(&self) -> *mut c_void {
        unsafe { (*self.raw).data[3].cast() }
    }
    /// Physical allocation shared by all retained references to this pixel buffer.
    pub fn allocation_size(&self) -> u64 {
        let buffer = self.pixel_buffer();
        if buffer.is_null() { return 0; }
        unsafe {
            let surface = CVPixelBufferGetIOSurface(buffer);
            if surface.is_null() {
                CVPixelBufferGetDataSize(buffer) as u64
            } else {
                IOSurfaceGetAllocSize(surface) as u64
            }
        }
    }
    pub fn width(&self) -> u32 {
        unsafe { (*self.raw).width as u32 }
    }
    pub fn height(&self) -> u32 {
        unsafe { (*self.raw).height as u32 }
    }
    pub fn format(&self) -> PixelFormat {
        self.format
    }
    pub fn color(&self) -> ColorInfo {
        self.color
    }
    pub fn pts_ns(&self) -> Option<i64> {
        self.pts_ns
    }
    pub fn timestamp(&self) -> Option<SourceTimestamp> {
        self.timestamp
    }
}
pub struct FrameLease {
    pub frame: VideoToolboxFrame,
    pub source_epoch: u64,
    pub timeline_epoch: u64,
    pub sequence: u64,
}
impl FrameLease {
    pub fn try_clone(&self) -> Result<Self, MediaError> {
        Ok(Self {
            frame: self.frame.try_clone()?,
            source_epoch: self.source_epoch,
            timeline_epoch: self.timeline_epoch,
            sequence: self.sequence,
        })
    }
}
#[derive(Clone, Debug)]
pub struct ActualPipeline {
    pub decoder: String,
    pub hardware: Option<bool>,
    pub fallback_reason: Option<String>,
}

unsafe extern "C" fn select_format(
    _: *mut ffi::AVCodecContext,
    formats: *const ffi::AVPixelFormat,
) -> ffi::AVPixelFormat {
    let mut p = formats;
    while !p.is_null() && *p != ffi::AVPixelFormat::AV_PIX_FMT_NONE {
        if *p == ffi::AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX {
            return *p;
        }
        p = p.add(1);
    }
    ffi::AVPixelFormat::AV_PIX_FMT_NONE
}

pub struct Session {
    path: PathBuf,
    input: format::context::Input,
    decoder: codec::decoder::Video,
    stream_index: usize,
    info: SourceInfo,
    cancelled: Arc<AtomicBool>,
    request_cancelled: Arc<Mutex<Option<Arc<AtomicBool>>>>,
    io_started: Arc<Mutex<Instant>>,
    draining: bool,
    ended: bool,
    sequence: u64,
    source_epoch: u64,
    timeline_epoch: u64,
    cursor_ns: Option<i64>,
    last_target_ns: Option<i64>,
    selected: Option<FrameLease>,
}
impl Session {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, crate::MediaError> {
        Self::open_with_cancel(path, Arc::new(AtomicBool::new(false)))
    }
    /// Cancellation and a per-I/O deadline interrupt libav open and demux operations.
    pub fn open_with_cancel(
        path: impl AsRef<Path>,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self, crate::MediaError> {
        if cancelled.load(Ordering::Acquire) {
            return Err(crate::MediaError::cancelled());
        }
        let version = unsafe { CStr::from_ptr(ffi::av_version_info()) }.to_string_lossy();
        if !version.contains("oq-vt-hw2-metal") {
            return Err(format!("Unverified FFmpeg build ({version}); VideoToolbox requires the hardware-required, Metal-compatible oq-vt-hw2-metal build").into());
        }
        ffmpeg_next::init().map_err(|e| format!("libav initialization failed: {e}"))?;
        let path = path
            .as_ref()
            .canonicalize()
            .map_err(|e| format!("Cannot access video file: {e}"))?;
        if !path.is_file() {
            return Err("Video source is not a regular file".into());
        }
        let io_started = Arc::new(Mutex::new(Instant::now()));
        let callback_cancelled = cancelled.clone();
        let callback_started = io_started.clone();
        let request_cancelled: Arc<Mutex<Option<Arc<AtomicBool>>>> = Arc::new(Mutex::new(None));
        let callback_request = request_cancelled.clone();
        let input = format::input_with_interrupt(&path, move || {
            callback_cancelled.load(Ordering::Acquire)
                || callback_request.lock().map_or(true, |request| request.as_ref().is_some_and(|request| request.load(Ordering::Acquire)))
                || callback_started
                    .lock()
                    .map_or(true, |started| started.elapsed() > Duration::from_secs(10))
        })
        .map_err(|e| format!("Cannot open video: {e}"))?;
        let stream = input.streams().best(Type::Video).ok_or("No video stream")?;
        if !matches!(stream.parameters().id(), codec::Id::H264 | codec::Id::HEVC) {
            return Err(
                "VideoToolbox hardware guarantee is available only for H.264 and HEVC".into(),
            );
        }
        let stream_index = stream.index();
        let time_base: ffi::AVRational = stream.time_base().into();
        if time_base.num <= 0 || time_base.den <= 0 {
            return Err(MediaError::time(
                ErrorKind::InvalidTimeBase,
                "Video stream has an invalid time base",
            ));
        }
        let start_pts = (stream.start_time() != ffi::AV_NOPTS_VALUE).then_some(stream.start_time());
        let stream_duration = stream.duration();
        let duration_pts = if stream_duration != ffi::AV_NOPTS_VALUE && stream_duration > 0 {
            Some(stream_duration)
        } else {
            let duration_us = input.duration();
            if duration_us <= 0 {
                None
            } else {
                let ticks = i128::from(duration_us)
                    .checked_mul(i128::from(time_base.den))
                    .and_then(|value| value.checked_add(i128::from(time_base.num) * 500_000))
                    .ok_or_else(|| {
                        MediaError::time(
                            ErrorKind::TimestampOverflow,
                            "Container duration rescaling overflow",
                        )
                    })?
                    / (i128::from(time_base.num) * 1_000_000);
                let relative_ticks = ticks - i128::from(start_pts.unwrap_or(0));
                Some(i64::try_from(relative_ticks).map_err(|_| {
                    MediaError::time(
                        ErrorKind::TimestampOverflow,
                        "Container duration exceeds stream timestamp range",
                    )
                })?)
            }
        };
        let rate = stream.avg_frame_rate();
        let average_frame_rate =
            (rate.numerator() > 0 && rate.denominator() > 0).then_some(RationalTimeBase {
                numerator: rate.numerator(),
                denominator: rate.denominator(),
            });
        let seekable = unsafe {
            let raw = input.as_ptr();
            !(*raw).pb.is_null()
                && (*(*raw).pb).seekable & ffi::AVIO_SEEKABLE_NORMAL as i32 != 0
                && (*raw).ctx_flags & ffi::AVFMTCTX_UNSEEKABLE as i32 == 0
        };
        let info = SourceInfo {
            stream_index,
            time_base: RationalTimeBase {
                numerator: time_base.num,
                denominator: time_base.den,
            },
            start_pts,
            duration_pts,
            average_frame_rate,
            seekable,
        };
        // Display matrices require transformation, which this slice does not apply.
        if stream
            .side_data()
            .any(|s| s.kind() == ffmpeg_next::codec::packet::side_data::Type::DisplayMatrix)
        {
            return Err(crate::MediaError::unsupported(
                "Video display matrix/rotation is unsupported",
            ));
        }
        let mut context = codec::context::Context::from_parameters(stream.parameters())
            .map_err(|e| format!("Cannot create decoder: {e}"))?;
        let mut device = ptr::null_mut();
        unsafe {
            let name = CString::new("videotoolbox").unwrap();
            let kind = ffi::av_hwdevice_find_type_by_name(name.as_ptr());
            if kind == ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_NONE {
                return Err("FFmpeg has no VideoToolbox hardware device".into());
            }
            let result =
                ffi::av_hwdevice_ctx_create(&mut device, kind, ptr::null(), ptr::null_mut(), 0);
            if result < 0 || device.is_null() {
                return Err(format!("Cannot create VideoToolbox device: {result}").into());
            }
            let raw = context.as_mut_ptr();
            (*raw).pkt_timebase = time_base;
            (*raw).get_format = Some(select_format);
            (*raw).hw_device_ctx = ffi::av_buffer_ref(device);
            ffi::av_buffer_unref(&mut device);
            if (*raw).hw_device_ctx.is_null() {
                return Err("Cannot retain VideoToolbox device".into());
            }
        }
        let decoder = context
            .decoder()
            .video()
            .map_err(|e| format!("Cannot open VideoToolbox decoder: {e}"))?;
        Ok(Self {
            path,
            input,
            decoder,
            stream_index,
            info,
            cancelled,
            request_cancelled,
            io_started,
            draining: false,
            ended: false,
            sequence: 0,
            source_epoch: NEXT_SOURCE_EPOCH.fetch_add(1, Ordering::Relaxed),
            timeline_epoch: 0,
            cursor_ns: None,
            last_target_ns: None,
            selected: None,
        })
    }
    pub fn actual_pipeline(&self) -> ActualPipeline {
        ActualPipeline {
            decoder: "ffmpeg-videotoolbox".into(),
            hardware: (self.sequence > 0).then_some(true),
            fallback_reason: None,
        }
    }
    pub fn source_info(&self) -> &SourceInfo {
        &self.info
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
            || self.request_cancelled.lock().map_or(true, |request| request.as_ref().is_some_and(|request| request.load(Ordering::Acquire)))
    }

    /// Abort one selection without poisoning this reusable decoder session.
    pub fn frame_at_ns_cancellable(
        &mut self,
        target_ns: i64,
        timeline_epoch: u64,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Option<SelectedFrame>, MediaError> {
        *self.request_cancelled.lock().map_err(|_| "Video request cancellation lock is poisoned")? = Some(cancelled.clone());
        let result = self.frame_at_ns(target_ns, timeline_epoch);
        *self.request_cancelled.lock().map_err(|_| "Video request cancellation lock is poisoned")? = None;
        if cancelled.load(Ordering::Acquire) {
            // The decoder may have advanced during preroll. Permit a fresh demand
            // to map against the actual cursor; newer epochs perform a real seek.
            self.last_target_ns = None;
            self.selected = None;
            // AVIO records AVERROR_EXIT as a sticky error. The abort belongs to
            // this demand, not to the input; clear it before the next epoch seeks.
            unsafe {
                let input = self.input.as_mut_ptr();
                if !(*input).pb.is_null() {
                    (*(*input).pb).error = 0;
                    (*(*input).pb).eof_reached = 0;
                }
            }
            return Err(MediaError::cancelled());
        }
        result
    }

    /// Select the earliest decoded presentation frame at/after stream-relative
    /// nanoseconds. Negative targets are invalid. Missing origin/PTS is an error;
    /// missing duration is supported by decoding through EOF, never FPS guessing.
    /// A higher epoch starts a discontinuity; backwards requests require one.
    pub fn frame_at_ns(
        &mut self,
        target_ns: i64,
        timeline_epoch: u64,
    ) -> Result<Option<SelectedFrame>, MediaError> {
        if self.is_cancelled() || timeline_epoch < self.timeline_epoch {
            return Err(MediaError::cancelled());
        }
        if target_ns < 0 {
            return Err(MediaError::time(
                ErrorKind::InvalidTarget,
                "Source target must be nonnegative",
            ));
        }
        let origin = self.info.start_pts.ok_or_else(|| {
            MediaError::time(
                ErrorKind::MissingTimestamp,
                "Video stream has no presentation origin",
            )
        })?;
        let seek_pts = target_pts(target_ns, origin, self.info.time_base)?;
        let backwards = self
            .last_target_ns
            .is_some_and(|previous| target_ns < previous)
            || (self.last_target_ns.is_none()
                && self.cursor_ns.is_some_and(|previous| target_ns <= previous));
        if backwards && timeline_epoch == self.timeline_epoch {
            return Err(MediaError::time(
                ErrorKind::DiscontinuousTarget,
                "Backwards source access requires a newer timeline epoch",
            ));
        }
        if timeline_epoch > self.timeline_epoch {
            if self.sequence > 0 || self.ended || target_ns > 0 {
                self.seek_to(seek_pts)?;
            }
            self.timeline_epoch = timeline_epoch;
            self.selected = None;
            self.last_target_ns = None;
        }
        self.last_target_ns = Some(target_ns);
        if let Some(cached) = &self.selected {
            if cached
                .frame
                .pts_ns()
                .is_some_and(|actual| actual >= target_ns)
            {
                return Ok(Some(selected_frame(cached.try_clone()?, target_ns)?));
            }
        }
        self.selected = None;
        while let Some(frame) = self.decode_next()? {
            let actual_ns = frame.frame.pts_ns().ok_or_else(|| {
                MediaError::time(
                    ErrorKind::MissingTimestamp,
                    "Decoded frame has no original presentation timestamp",
                )
            })?;
            if actual_ns >= target_ns {
                if self.is_cancelled() {
                    return Err(MediaError::cancelled());
                }
                self.selected = Some(frame.try_clone()?);
                return Ok(Some(selected_frame(frame, target_ns)?));
            }
        }
        Ok(None)
    }

    fn seek_to(&mut self, pts: i64) -> Result<(), MediaError> {
        if self.is_cancelled() {
            return Err(MediaError::cancelled());
        }
        if !self.info.seekable {
            return Err(MediaError::time(
                ErrorKind::Unseekable,
                "Source does not support seeking",
            ));
        }
        *self
            .io_started
            .lock()
            .map_err(|_| "Video I/O deadline lock is poisoned")? = Instant::now();
        // Unlike Input::seek (stream_index=-1, AV_TIME_BASE), this call uses
        // the selected stream's rational. BACKWARD without ANY selects a
        // preceding keyframe. Flush delayed B frames before decoding preroll.
        let result = unsafe {
            ffi::av_seek_frame(
                self.input.as_mut_ptr(),
                self.stream_index as i32,
                pts,
                ffi::AVSEEK_FLAG_BACKWARD as i32,
            )
        };
        if self.is_cancelled() {
            return Err(MediaError::cancelled());
        }
        if result < 0 {
            return Err(MediaError::time(
                ErrorKind::SeekFailed,
                format!(
                    "Video keyframe seek failed: {}",
                    ffmpeg_next::Error::from(result)
                ),
            ));
        }
        self.decoder.flush();
        self.draining = false;
        self.ended = false;
        self.cursor_ns = None;
        self.selected = None;
        Ok(())
    }
    pub fn reset(&mut self) -> Result<(), crate::MediaError> {
        let epoch = self.timeline_epoch.checked_add(1).ok_or_else(|| {
            MediaError::time(ErrorKind::TimestampOverflow, "Timeline epoch overflow")
        })?;
        let mut replacement = Self::open_with_cancel(&self.path, self.cancelled.clone())?;
        replacement.source_epoch = self.source_epoch;
        replacement.timeline_epoch = epoch;
        *self = replacement;
        Ok(())
    }
    /// Returns frames in presentation order, draining delayed frames at end of stream.
    pub fn next_frame(&mut self) -> Result<Option<FrameLease>, crate::MediaError> {
        self.selected = None;
        self.last_target_ns = None;
        self.decode_next()
    }
    fn decode_next(&mut self) -> Result<Option<FrameLease>, MediaError> {
        if self.is_cancelled() {
            return Err(MediaError::cancelled());
        }
        if self.ended {
            return Ok(None);
        }
        loop {
            if self.is_cancelled() {
                return Err(crate::MediaError::cancelled());
            }
            let mut frame = Video::empty();
            match self.decoder.receive_frame(&mut frame) {
                Ok(()) => {
                    let raw = unsafe { &*frame.as_ptr() };
                    if raw.format != ffi::AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX as i32
                        || raw.data[3].is_null()
                    {
                        return Err("Decoder did not produce a VideoToolbox surface".into());
                    }
                    if raw.crop_top != 0
                        || raw.crop_bottom != 0
                        || raw.crop_left != 0
                        || raw.crop_right != 0
                    {
                        return Err(crate::MediaError::unsupported(
                            "Cropped video is unsupported",
                        ));
                    }
                    if !unsafe {
                        ffi::av_frame_get_side_data(
                            frame.as_ptr(),
                            ffi::AVFrameSideDataType::AV_FRAME_DATA_DISPLAYMATRIX,
                        )
                    }
                    .is_null()
                    {
                        return Err(crate::MediaError::unsupported(
                            "Video frame rotation is unsupported",
                        ));
                    }
                    if raw.sample_aspect_ratio.num > 0
                        && raw.sample_aspect_ratio.den > 0
                        && raw.sample_aspect_ratio.num != raw.sample_aspect_ratio.den
                    {
                        return Err(crate::MediaError::unsupported(
                            "Non-square video pixels require an explicit presentation transform",
                        ));
                    }
                    if raw.flags & ffi::AV_FRAME_FLAG_INTERLACED as i32 != 0 {
                        return Err(crate::MediaError::unsupported(
                            "Interlaced video requires a deinterlacer",
                        ));
                    }
                    let (primaries, primaries_assumed) = match raw.color_primaries {
                        ffi::AVColorPrimaries::AVCOL_PRI_BT709 => (ColorPrimaries::Bt709, false),
                        ffi::AVColorPrimaries::AVCOL_PRI_BT470BG => {
                            (ColorPrimaries::Bt470Bg, false)
                        }
                        ffi::AVColorPrimaries::AVCOL_PRI_UNSPECIFIED => {
                            (ColorPrimaries::Bt709, true)
                        }
                        other => {
                            return Err(crate::MediaError::unsupported(format!(
                                "Unsupported color primaries {other:?}"
                            )))
                        }
                    };
                    let code = unsafe { CVPixelBufferGetPixelFormatType(raw.data[3].cast()) };
                    let (format, full_range) = match code {
                        0x34323076 => (PixelFormat::Nv12, false), // 420v
                        0x34323066 => (PixelFormat::Nv12, true),  // 420f
                        0x78343230 => (PixelFormat::P010, false), // x420
                        0x78663230 => (PixelFormat::P010, true),  // xf20
                        0x42475241 => (PixelFormat::Bgra8, true),
                        _ => {
                            return Err(format!("Unsupported CVPixelBuffer format {code:#x}").into())
                        }
                    };
                    let (transfer, transfer_assumed) = match raw.color_trc {
                        ffi::AVColorTransferCharacteristic::AVCOL_TRC_BT709
                        | ffi::AVColorTransferCharacteristic::AVCOL_TRC_SMPTE170M => {
                            (TransferFunction::Bt709, false)
                        }
                        ffi::AVColorTransferCharacteristic::AVCOL_TRC_IEC61966_2_1 => {
                            (TransferFunction::Srgb, false)
                        }
                        ffi::AVColorTransferCharacteristic::AVCOL_TRC_UNSPECIFIED => {
                            (TransferFunction::Bt709, true)
                        }
                        other => {
                            return Err(crate::MediaError::unsupported(format!(
                                "Unsupported transfer characteristic {other:?}"
                            )))
                        }
                    };
                    let matrix = match raw.colorspace {
                        ffi::AVColorSpace::AVCOL_SPC_BT709 => ColorMatrix::Bt709,
                        ffi::AVColorSpace::AVCOL_SPC_BT2020_NCL => ColorMatrix::Bt2020,
                        ffi::AVColorSpace::AVCOL_SPC_BT470BG
                        | ffi::AVColorSpace::AVCOL_SPC_SMPTE170M => ColorMatrix::Bt601,
                        ffi::AVColorSpace::AVCOL_SPC_UNSPECIFIED
                        | ffi::AVColorSpace::AVCOL_SPC_RGB => {
                            if raw.width >= 1280 || raw.height > 576 {
                                ColorMatrix::Bt709
                            } else {
                                ColorMatrix::Bt601
                            }
                        }
                        other => {
                            return Err(crate::MediaError::unsupported(format!(
                                "Unsupported color matrix {other:?}"
                            )))
                        }
                    };
                    let matrix_assumed = matches!(
                        raw.colorspace,
                        ffi::AVColorSpace::AVCOL_SPC_UNSPECIFIED | ffi::AVColorSpace::AVCOL_SPC_RGB
                    );
                    // AVFrame.pts is the decoded presentation timestamp. Do not
                    // promote best_effort_timestamp (which FFmpeg may infer from DTS)
                    // into a claimed original source PTS.
                    let timestamp = frame.pts().map(|pts| SourceTimestamp {
                        pts,
                        time_base: self.info.time_base,
                    });
                    let pts = match (timestamp, self.info.start_pts) {
                        (Some(timestamp), Some(origin)) => {
                            Some(normalize_pts(timestamp.pts, origin, timestamp.time_base)?)
                        }
                        _ => None,
                    };
                    if let (Some(previous), Some(current)) = (self.cursor_ns, pts) {
                        if current < previous {
                            return Err(MediaError::time(
                                ErrorKind::InvalidTimestamp,
                                "Decoder output is not in presentation order",
                            ));
                        }
                    }
                    self.cursor_ns = pts;
                    let retained = unsafe { ffi::av_frame_clone(frame.as_ptr()) };
                    if retained.is_null() {
                        return Err("Cannot retain VideoToolbox frame".into());
                    }
                    self.sequence = self.sequence.saturating_add(1);
                    return Ok(Some(FrameLease {
                        frame: VideoToolboxFrame {
                            raw: retained,
                            format,
                            color: ColorInfo {
                                matrix,
                                matrix_assumed,
                                full_range,
                                transfer,
                                transfer_assumed,
                                primaries,
                                primaries_assumed,
                            },
                            pts_ns: pts,
                            timestamp,
                        },
                        source_epoch: self.source_epoch,
                        timeline_epoch: self.timeline_epoch,
                        sequence: self.sequence,
                    }));
                }
                Err(ffmpeg_next::Error::Eof) => {
                    self.ended = true;
                    return Ok(None);
                }
                Err(ffmpeg_next::Error::Other { errno }) if errno == ffmpeg_next::error::EAGAIN => {
                }
                Err(e) => return Err(format!("VideoToolbox receive failed: {e}").into()),
            }
            if self.draining {
                return Err("Decoder stalled while draining".into());
            }
            let mut packet = Packet::empty();
            loop {
                if self.is_cancelled() {
                    return Err(crate::MediaError::cancelled());
                }
                *self
                    .io_started
                    .lock()
                    .map_err(|_| "Video I/O deadline lock is poisoned")? = Instant::now();
                match packet.read(&mut self.input) {
                    Ok(()) if packet.stream() == self.stream_index => break,
                    Ok(()) => continue,
                    Err(ffmpeg_next::Error::Eof) => {
                        self.decoder
                            .send_eof()
                            .map_err(|e| format!("Decoder drain failed: {e}"))?;
                        self.draining = true;
                        break;
                    }
                    Err(_) if self.is_cancelled() => {
                        return Err(MediaError::cancelled())
                    }
                    Err(e) => return Err(format!("Video demux failed: {e}").into()),
                }
            }
            if !self.draining {
                self.decoder
                    .send_packet(&packet)
                    .map_err(|e| format!("VideoToolbox decode failed: {e}"))?;
            }
        }
    }
}

fn selected_frame(frame: FrameLease, target_ns: i64) -> Result<SelectedFrame, MediaError> {
    let actual_ns = frame.frame.pts_ns().ok_or_else(|| {
        MediaError::time(
            ErrorKind::MissingTimestamp,
            "Decoded frame has no presentation timestamp",
        )
    })?;
    Ok(SelectedFrame {
        frame,
        target_ns,
        actual_ns,
        selection: if actual_ns == target_ns {
            FrameSelection::Exact
        } else {
            FrameSelection::Following
        },
    })
}

fn normalize_pts(pts: i64, origin: i64, base: RationalTimeBase) -> Result<i64, MediaError> {
    base.ticks_to_ns(i128::from(pts) - i128::from(origin))
}

fn target_pts(target_ns: i64, origin: i64, base: RationalTimeBase) -> Result<i64, MediaError> {
    if base.numerator <= 0 || base.denominator <= 0 {
        return Err(MediaError::time(
            ErrorKind::InvalidTimeBase,
            "Time base must be positive",
        ));
    }
    // Floor, not nearest: the seek anchor must never pass the requested instant.
    let ticks = i128::from(target_ns) * i128::from(base.denominator)
        / (i128::from(base.numerator) * 1_000_000_000);
    i64::try_from(i128::from(origin) + ticks).map_err(|_| {
        MediaError::time(
            ErrorKind::TimestampOverflow,
            "Source target exceeds stream timestamp range",
        )
    })
}

#[cfg(test)]
mod time_access_tests {
    use super::*;

    #[test]
    fn stream_rescaling_is_checked_and_seek_rounds_down() {
        let base = RationalTimeBase {
            numerator: 1,
            denominator: 90_000,
        };
        assert_eq!(target_pts(500_000_001, 180_000, base).unwrap(), 225_000);
        assert_eq!(normalize_pts(225_000, 180_000, base).unwrap(), 500_000_000);
        assert_eq!(normalize_pts(179_999, 180_000, base).unwrap(), -11_111);
        assert_eq!(
            target_pts(1_000_000_000, i64::MAX, base).unwrap_err().kind,
            ErrorKind::TimestampOverflow
        );
        assert_eq!(
            normalize_pts(i64::MAX, i64::MIN, base).unwrap_err().kind,
            ErrorKind::TimestampOverflow
        );
        let invalid = RationalTimeBase {
            numerator: 0,
            denominator: 1,
        };
        assert_eq!(
            target_pts(1, 0, invalid).unwrap_err().kind,
            ErrorKind::InvalidTimeBase
        );
        assert_eq!(
            normalize_pts(1, 0, invalid).unwrap_err().kind,
            ErrorKind::InvalidTimeBase
        );
    }

    #[test]
    fn unknown_duration_decodes_to_eof_and_unseekable_discontinuity_is_typed() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/source-time.mp4");
        let mut session = Session::open(path).unwrap();
        // Capability metadata is private and immutable to consumers. Exercise
        // the unknown-duration/no-seek branches using the real decoder cursor.
        session.info.duration_pts = None;
        assert_eq!(session.source_info().duration_ns().unwrap(), None);
        session.frame_at_ns(500_000_000, 0).unwrap().unwrap();
        session.info.seekable = false;
        assert_eq!(
            session.frame_at_ns(0, 1).err().unwrap().kind,
            ErrorKind::Unseekable
        );
        assert!(session.frame_at_ns(4_000_000_000, 0).unwrap().is_none());
    }

    #[test]
    fn missing_stream_origin_is_never_replaced_with_an_fps_estimate() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/source-time.mp4");
        let mut session = Session::open(path).unwrap();
        session.info.start_pts = None;
        assert_eq!(
            session.frame_at_ns(0, 0).err().unwrap().kind,
            ErrorKind::MissingTimestamp
        );
    }
}
