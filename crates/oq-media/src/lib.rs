//! Framework-independent media metadata and a strict VideoToolbox file decoder.
//! `oq` records the project's OpenQuartz origin; this crate has no OpenQuartz dependency.

/// Backend availability, unsupported presentation, and intentional cancellation
/// are distinct so hosts never choose fallback by matching a diagnostic string.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ErrorKind { Backend, UnsupportedContent, Cancelled }

/// A media failure; `kind` drives policy, `message` is for diagnostics.
#[derive(Debug)]
pub struct MediaError { pub kind: ErrorKind, pub message: String }
impl MediaError {
    pub fn unsupported(message: impl Into<String>) -> Self { Self { kind: ErrorKind::UnsupportedContent, message: message.into() } }
    pub fn cancelled() -> Self { Self { kind: ErrorKind::Cancelled, message: "Video decoding cancelled".into() } }
}
impl From<String> for MediaError { fn from(message:String)->Self { Self { kind:ErrorKind::Backend, message } } }
impl From<&str> for MediaError { fn from(message:&str)->Self { message.to_owned().into() } }
impl From<MediaError> for String { fn from(error:MediaError)->Self { error.message } }
impl std::fmt::Display for MediaError { fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result { f.write_str(&self.message) } }
impl std::error::Error for MediaError {}

/// Memory layout of the retained decoded surface (not an encoded codec).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PixelFormat { Nv12, P010, Bgra8 }
/// Coefficients used to recover RGB from YUV planes.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ColorMatrix { Bt601, Bt709, Bt2020 }
/// Supported source transfer characteristics. GPU adapters convert Bt709 to
/// the application's SDR sRGB-encoded output; unsupported transfers are rejected.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TransferFunction { Bt709, Srgb }
/// RGB chromaticities, distinct from YUV matrix coefficients.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ColorPrimaries { Bt709, Bt470Bg }
/// Source color metadata. Unspecified matrix/transfer/primaries are reported as
/// assumptions, not as measured properties.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ColorInfo { pub matrix: ColorMatrix, pub matrix_assumed: bool, pub full_range: bool, pub transfer: TransferFunction, pub transfer_assumed: bool, pub primaries: ColorPrimaries, pub primaries_assumed: bool }

#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
mod videotoolbox;
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
pub use videotoolbox::{ActualPipeline, FrameLease, Session, VideoToolboxFrame};
