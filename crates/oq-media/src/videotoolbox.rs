use crate::{ColorInfo, ColorMatrix, ColorPrimaries, PixelFormat, TransferFunction};
use ffmpeg_next::{codec, format, media::Type, util::frame::video::Video, Packet};
use ffmpeg_next::ffi;
use std::{ffi::{c_void, CStr, CString}, path::{Path, PathBuf}, ptr};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, AtomicU64, Ordering}};
use std::time::{Duration, Instant};
static NEXT_SOURCE_EPOCH: AtomicU64 = AtomicU64::new(1);

#[link(name = "CoreVideo", kind = "framework")]
extern "C" { fn CVPixelBufferGetPixelFormatType(buffer: *mut c_void) -> u32; }

/// Owns the FFmpeg frame reference, and therefore its immutable VideoToolbox surface.
pub struct VideoToolboxFrame { raw: *mut ffi::AVFrame, format: PixelFormat, color: ColorInfo, pts_ns: Option<i64> }
// The retained AVFrame and CVPixelBuffer are immutable after publication. FFmpeg's
// buffer references are thread-safe and the consumer never receives mutable access.
unsafe impl Send for VideoToolboxFrame {}
impl Drop for VideoToolboxFrame { fn drop(&mut self) { unsafe { ffi::av_frame_free(&mut self.raw) } } }
impl VideoToolboxFrame {
    pub fn try_clone(&self) -> Result<Self, String> {
        let raw = unsafe { ffi::av_frame_clone(self.raw) };
        if raw.is_null() { return Err("Cannot retain VideoToolbox frame".into()); }
        Ok(Self { raw, format: self.format, color: self.color, pts_ns: self.pts_ns })
    }
    pub fn pixel_buffer(&self) -> *mut c_void { unsafe { (*self.raw).data[3].cast() } }
    pub fn width(&self) -> u32 { unsafe { (*self.raw).width as u32 } }
    pub fn height(&self) -> u32 { unsafe { (*self.raw).height as u32 } }
    pub fn format(&self) -> PixelFormat { self.format }
    pub fn color(&self) -> ColorInfo { self.color }
    pub fn pts_ns(&self) -> Option<i64> { self.pts_ns }
}
pub struct FrameLease { pub frame: VideoToolboxFrame, pub source_epoch: u64, pub timeline_epoch: u64, pub sequence: u64 }
#[derive(Clone, Debug)]
pub struct ActualPipeline { pub decoder: String, pub hardware: Option<bool>, pub fallback_reason: Option<String> }

unsafe extern "C" fn select_format(_: *mut ffi::AVCodecContext, formats: *const ffi::AVPixelFormat) -> ffi::AVPixelFormat {
    let mut p = formats;
    while !p.is_null() && *p != ffi::AVPixelFormat::AV_PIX_FMT_NONE {
        if *p == ffi::AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX { return *p; }
        p = p.add(1);
    }
    ffi::AVPixelFormat::AV_PIX_FMT_NONE
}

pub struct Session {
    path: PathBuf, input: format::context::Input, decoder: codec::decoder::Video,
    stream_index: usize, time_base: ffi::AVRational, start_time: i64,
    cancelled: Arc<AtomicBool>, io_started: Arc<Mutex<Instant>>,
    draining: bool, ended: bool, sequence: u64, source_epoch: u64, timeline_epoch: u64,
}
impl Session {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, crate::MediaError> {
        Self::open_with_cancel(path, Arc::new(AtomicBool::new(false)))
    }
    /// Cancellation and a per-I/O deadline interrupt libav open and demux operations.
    pub fn open_with_cancel(path: impl AsRef<Path>, cancelled: Arc<AtomicBool>) -> Result<Self, crate::MediaError> {
        if cancelled.load(Ordering::Acquire) { return Err(crate::MediaError::cancelled()); }
        let version = unsafe { CStr::from_ptr(ffi::av_version_info()) }.to_string_lossy();
        if !version.contains("oq-vt-hw2-metal") {
            return Err(format!("Unverified FFmpeg build ({version}); VideoToolbox requires the hardware-required, Metal-compatible oq-vt-hw2-metal build").into());
        }
        ffmpeg_next::init().map_err(|e| format!("libav initialization failed: {e}"))?;
        let path = path.as_ref().canonicalize().map_err(|e| format!("Cannot access video file: {e}"))?;
        if !path.is_file() { return Err("Video source is not a regular file".into()); }
        let io_started = Arc::new(Mutex::new(Instant::now()));
        let callback_cancelled = cancelled.clone();
        let callback_started = io_started.clone();
        let input = format::input_with_interrupt(&path, move || {
            callback_cancelled.load(Ordering::Acquire) || callback_started.lock().map_or(true, |started| started.elapsed() > Duration::from_secs(10))
        }).map_err(|e| format!("Cannot open video: {e}"))?;
        let stream = input.streams().best(Type::Video).ok_or("No video stream")?;
        if !matches!(stream.parameters().id(), codec::Id::H264 | codec::Id::HEVC) {
            return Err("VideoToolbox hardware guarantee is available only for H.264 and HEVC".into());
        }
        let stream_index = stream.index();
        let time_base: ffi::AVRational = stream.time_base().into();
        let start_time = if stream.start_time() == ffi::AV_NOPTS_VALUE { 0 } else { stream.start_time() };
        // Display matrices require transformation, which this slice does not apply.
        if stream.side_data().any(|s| s.kind() == ffmpeg_next::codec::packet::side_data::Type::DisplayMatrix) {
            return Err(crate::MediaError::unsupported("Video display matrix/rotation is unsupported"));
        }
        let mut context = codec::context::Context::from_parameters(stream.parameters()).map_err(|e| format!("Cannot create decoder: {e}"))?;
        let mut device = ptr::null_mut();
        unsafe {
            let name = CString::new("videotoolbox").unwrap();
            let kind = ffi::av_hwdevice_find_type_by_name(name.as_ptr());
            if kind == ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_NONE { return Err("FFmpeg has no VideoToolbox hardware device".into()); }
            let result = ffi::av_hwdevice_ctx_create(&mut device, kind, ptr::null(), ptr::null_mut(), 0);
            if result < 0 || device.is_null() { return Err(format!("Cannot create VideoToolbox device: {result}").into()); }
            let raw = context.as_mut_ptr();
            (*raw).get_format = Some(select_format);
            (*raw).hw_device_ctx = ffi::av_buffer_ref(device);
            ffi::av_buffer_unref(&mut device);
            if (*raw).hw_device_ctx.is_null() { return Err("Cannot retain VideoToolbox device".into()); }
        }
        let decoder = context.decoder().video().map_err(|e| format!("Cannot open VideoToolbox decoder: {e}"))?;
        Ok(Self { path, input, decoder, stream_index, time_base, start_time, cancelled, io_started, draining: false, ended: false, sequence: 0, source_epoch: NEXT_SOURCE_EPOCH.fetch_add(1, Ordering::Relaxed), timeline_epoch: 0 })
    }
    pub fn actual_pipeline(&self) -> ActualPipeline { ActualPipeline { decoder: "ffmpeg-videotoolbox".into(), hardware: (self.sequence > 0).then_some(true), fallback_reason: None } }
    pub fn reset(&mut self) -> Result<(), crate::MediaError> {
        let epoch = self.timeline_epoch.saturating_add(1);
        let mut replacement = Self::open_with_cancel(&self.path, self.cancelled.clone())?;
        replacement.source_epoch = self.source_epoch;
        replacement.timeline_epoch = epoch;
        *self = replacement;
        Ok(())
    }
    /// Returns frames in presentation order, draining delayed frames at end of stream.
    pub fn next_frame(&mut self) -> Result<Option<FrameLease>, crate::MediaError> {
        if self.ended { return Ok(None); }
        loop {
            if self.cancelled.load(Ordering::Acquire) { return Err(crate::MediaError::cancelled()); }
            let mut frame = Video::empty();
            match self.decoder.receive_frame(&mut frame) {
                Ok(()) => {
                    let raw = unsafe { &*frame.as_ptr() };
                    if raw.format != ffi::AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX as i32 || raw.data[3].is_null() { return Err("Decoder did not produce a VideoToolbox surface".into()); }
                    if raw.crop_top != 0 || raw.crop_bottom != 0 || raw.crop_left != 0 || raw.crop_right != 0 { return Err(crate::MediaError::unsupported("Cropped video is unsupported")); }
                    if !unsafe { ffi::av_frame_get_side_data(frame.as_ptr(), ffi::AVFrameSideDataType::AV_FRAME_DATA_DISPLAYMATRIX) }.is_null() { return Err(crate::MediaError::unsupported("Video frame rotation is unsupported")); }
                    if raw.sample_aspect_ratio.num > 0 && raw.sample_aspect_ratio.den > 0 && raw.sample_aspect_ratio.num != raw.sample_aspect_ratio.den {
                        return Err(crate::MediaError::unsupported("Non-square video pixels require an explicit presentation transform"));
                    }
                    if raw.flags & ffi::AV_FRAME_FLAG_INTERLACED as i32 != 0 {
                        return Err(crate::MediaError::unsupported("Interlaced video requires a deinterlacer"));
                    }
                    let (primaries, primaries_assumed) = match raw.color_primaries {
                        ffi::AVColorPrimaries::AVCOL_PRI_BT709 => (ColorPrimaries::Bt709, false),
                        ffi::AVColorPrimaries::AVCOL_PRI_BT470BG => (ColorPrimaries::Bt470Bg, false),
                        ffi::AVColorPrimaries::AVCOL_PRI_UNSPECIFIED => (ColorPrimaries::Bt709, true),
                        other => return Err(crate::MediaError::unsupported(format!("Unsupported color primaries {other:?}"))),
                    };
                    let code = unsafe { CVPixelBufferGetPixelFormatType(raw.data[3].cast()) };
                    let (format, full_range) = match code {
                        0x34323076 => (PixelFormat::Nv12, false), // 420v
                        0x34323066 => (PixelFormat::Nv12, true),  // 420f
                        0x78343230 => (PixelFormat::P010, false), // x420
                        0x78663230 => (PixelFormat::P010, true),  // xf20
                        0x42475241 => (PixelFormat::Bgra8, true),
                        _ => return Err(format!("Unsupported CVPixelBuffer format {code:#x}").into()),
                    };
                    let (transfer, transfer_assumed) = match raw.color_trc {
                        ffi::AVColorTransferCharacteristic::AVCOL_TRC_BT709 | ffi::AVColorTransferCharacteristic::AVCOL_TRC_SMPTE170M => (TransferFunction::Bt709, false),
                        ffi::AVColorTransferCharacteristic::AVCOL_TRC_IEC61966_2_1 => (TransferFunction::Srgb, false),
                        ffi::AVColorTransferCharacteristic::AVCOL_TRC_UNSPECIFIED => (TransferFunction::Bt709, true),
                        other => return Err(crate::MediaError::unsupported(format!("Unsupported transfer characteristic {other:?}"))),
                    };
                    let matrix = match raw.colorspace {
                        ffi::AVColorSpace::AVCOL_SPC_BT709 => ColorMatrix::Bt709,
                        ffi::AVColorSpace::AVCOL_SPC_BT2020_NCL => ColorMatrix::Bt2020,
                        ffi::AVColorSpace::AVCOL_SPC_BT470BG | ffi::AVColorSpace::AVCOL_SPC_SMPTE170M => ColorMatrix::Bt601,
                        ffi::AVColorSpace::AVCOL_SPC_UNSPECIFIED | ffi::AVColorSpace::AVCOL_SPC_RGB => if raw.width >= 1280 || raw.height > 576 { ColorMatrix::Bt709 } else { ColorMatrix::Bt601 },
                        other => return Err(crate::MediaError::unsupported(format!("Unsupported color matrix {other:?}"))),
                    };
                    let matrix_assumed = matches!(raw.colorspace, ffi::AVColorSpace::AVCOL_SPC_UNSPECIFIED | ffi::AVColorSpace::AVCOL_SPC_RGB);
                    let pts = frame.timestamp().map(|v| unsafe { ffi::av_rescale_q(v.saturating_sub(self.start_time), self.time_base, ffi::AVRational { num: 1, den: 1_000_000_000 }) });
                    let retained = unsafe { ffi::av_frame_clone(frame.as_ptr()) };
                    if retained.is_null() { return Err("Cannot retain VideoToolbox frame".into()); }
                    self.sequence = self.sequence.saturating_add(1);
                    return Ok(Some(FrameLease { frame: VideoToolboxFrame { raw: retained, format, color: ColorInfo { matrix, matrix_assumed, full_range, transfer, transfer_assumed, primaries, primaries_assumed }, pts_ns: pts }, source_epoch: self.source_epoch, timeline_epoch: self.timeline_epoch, sequence: self.sequence }));
                }
                Err(ffmpeg_next::Error::Eof) => { self.ended = true; return Ok(None); }
                Err(ffmpeg_next::Error::Other { errno }) if errno == ffmpeg_next::error::EAGAIN => {}
                Err(e) => return Err(format!("VideoToolbox receive failed: {e}").into()),
            }
            if self.draining { return Err("Decoder stalled while draining".into()); }
            let mut packet = Packet::empty();
            loop {
                if self.cancelled.load(Ordering::Acquire) { return Err(crate::MediaError::cancelled()); }
                *self.io_started.lock().map_err(|_| "Video I/O deadline lock is poisoned")? = Instant::now();
                match packet.read(&mut self.input) {
                    Ok(()) if packet.stream() == self.stream_index => break,
                    Ok(()) => continue,
                    Err(ffmpeg_next::Error::Eof) => {
                        self.decoder.send_eof().map_err(|e| format!("Decoder drain failed: {e}"))?;
                        self.draining = true;
                        break;
                    }
                    Err(e) => return Err(format!("Video demux failed: {e}").into()),
                }
            }
            if !self.draining { self.decoder.send_packet(&packet).map_err(|e| format!("VideoToolbox decode failed: {e}"))?; }
        }
    }
}
