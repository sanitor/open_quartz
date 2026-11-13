//! Framework-independent media metadata and a strict VideoToolbox file decoder.
//! `oq` records the project's OpenQuartz origin; this crate has no OpenQuartz dependency.

/// Backend availability, unsupported presentation, and intentional cancellation
/// are distinct so hosts never choose fallback by matching a diagnostic string.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Backend,
    UnsupportedContent,
    Cancelled,
    InvalidTarget,
    InvalidTimeBase,
    MissingTimestamp,
    TimestampOverflow,
    DiscontinuousTarget,
    Unseekable,
    SeekFailed,
    InvalidTimestamp,
}

/// A media failure; `kind` drives policy, `message` is for diagnostics.
#[derive(Clone, Debug)]
pub struct MediaError {
    pub kind: ErrorKind,
    pub message: String,
}
impl MediaError {
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::UnsupportedContent,
            message: message.into(),
        }
    }
    pub fn cancelled() -> Self {
        Self {
            kind: ErrorKind::Cancelled,
            message: "Video decoding cancelled".into(),
        }
    }
    pub fn time(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}
impl From<String> for MediaError {
    fn from(message: String) -> Self {
        Self {
            kind: ErrorKind::Backend,
            message,
        }
    }
}
impl From<&str> for MediaError {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}
impl From<MediaError> for String {
    fn from(error: MediaError) -> Self {
        error.message
    }
}
impl std::fmt::Display for MediaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for MediaError {}

/// Seconds per stream timestamp tick; both components must be positive.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RationalTimeBase {
    pub numerator: i32,
    pub denominator: i32,
}
impl RationalTimeBase {
    pub(crate) fn ticks_to_ns(self, ticks: i128) -> Result<i64, MediaError> {
        if self.numerator <= 0 || self.denominator <= 0 {
            return Err(MediaError::time(
                ErrorKind::InvalidTimeBase,
                "Time base must be positive",
            ));
        }
        let numerator = ticks
            .checked_mul(i128::from(self.numerator))
            .and_then(|value| value.checked_mul(1_000_000_000))
            .ok_or_else(|| {
                MediaError::time(ErrorKind::TimestampOverflow, "Timestamp rescaling overflow")
            })?;
        let denominator = i128::from(self.denominator);
        let magnitude = numerator
            .checked_abs()
            .and_then(|value| value.checked_add(denominator / 2))
            .ok_or_else(|| {
                MediaError::time(ErrorKind::TimestampOverflow, "Timestamp rounding overflow")
            })?
            / denominator;
        i64::try_from(if numerator < 0 { -magnitude } else { magnitude }).map_err(|_| {
            MediaError::time(
                ErrorKind::TimestampOverflow,
                "Timestamp exceeds nanosecond range",
            )
        })
    }
}
/// Original decoded presentation timestamp, in the selected stream's time base.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SourceTimestamp {
    pub pts: i64,
    pub time_base: RationalTimeBase,
}
/// Container metadata; absent origin/duration/rate is never replaced with a guess.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceInfo {
    pub stream_index: usize,
    pub time_base: RationalTimeBase,
    pub start_pts: Option<i64>,
    pub duration_pts: Option<i64>,
    pub average_frame_rate: Option<RationalTimeBase>,
    pub seekable: bool,
    pub codec: String,
    pub has_audio: bool,
}
impl SourceInfo {
    /// Checked, nearest-rounded duration. None means unknown, not zero or EOF.
    pub fn duration_ns(&self) -> Result<Option<i64>, MediaError> {
        self.duration_pts
            .map(|duration| self.time_base.ticks_to_ns(i128::from(duration)))
            .transpose()
    }
}
/// Exact/Following describe normalized original source PTS. Estimated is reserved
/// for an explicitly estimated backend; the strict file Session never emits it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FrameSelection {
    Exact,
    Following,
    Estimated,
}

/// A retained presentation surface and the requested/actual stream-relative time.

#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
pub struct SelectedFrame {
    pub frame: FrameLease,
    pub target_ns: i64,
    pub actual_ns: i64,
    pub selection: FrameSelection,
}

/// Memory layout of the retained decoded surface (not an encoded codec).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    Nv12,
    P010,
    Bgra8,
}
/// Coefficients used to recover RGB from YUV planes.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ColorMatrix {
    Bt601,
    Bt709,
    Bt2020,
}
/// Supported source transfer characteristics. GPU adapters convert Bt709 to
/// the application's SDR sRGB-encoded output; unsupported transfers are rejected.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TransferFunction {
    Bt709,
    Srgb,
}
/// RGB chromaticities, distinct from YUV matrix coefficients.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ColorPrimaries {
    Bt709,
    Bt470Bg,
}
/// Source color metadata. Unspecified matrix/transfer/primaries are reported as
/// assumptions, not as measured properties.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ColorInfo {
    pub matrix: ColorMatrix,
    pub matrix_assumed: bool,
    pub full_range: bool,
    pub transfer: TransferFunction,
    pub transfer_assumed: bool,
    pub primaries: ColorPrimaries,
    pub primaries_assumed: bool,
}

#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
mod encoder;
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
pub use encoder::{
    EncodeAdmission, EncodeFrame, EncoderConfig, EncoderReport, EncoderStatus, EncoderSurface,
    Mp4Encoder,
};
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
mod videotoolbox;
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
pub use videotoolbox::{ActualPipeline, FrameLease, Session, VideoToolboxFrame};
