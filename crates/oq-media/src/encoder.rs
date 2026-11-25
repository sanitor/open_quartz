#[cfg(target_os = "macos")]
mod mac_surface;
#[cfg(windows)]
pub use crate::d3d12_encoder::{InputSurface as EncoderSurface, Profile as EncoderProfile};
#[cfg(target_os = "macos")]
pub use mac_surface::EncoderSurface;

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EncoderProfile {
    #[default]
    Sdr,
    Hdr10,
    Hlg,
}

#[cfg(target_os = "macos")]
impl EncoderProfile {
    pub fn is_hdr(self) -> bool {
        self != Self::Sdr
    }

    fn pixel_format(self) -> u32 {
        u32::from_be_bytes(if self.is_hdr() { *b"x420" } else { *b"420v" })
    }

    fn codec(self) -> &'static str {
        if self.is_hdr() {
            "hevc_videotoolbox"
        } else {
            "h264_videotoolbox"
        }
    }
}

#[derive(Clone, Debug)]
pub struct EncoderConfig {
    pub width: u32,
    pub height: u32,
    pub time_base: crate::RationalTimeBase,
    pub bitrate: u64,
    pub queue_capacity: usize,
    /// Zero disables reordering; 1 or 2 enables native two-frame reorder depth.
    /// VideoToolbox does not expose an exact maximum B-frame-run control.
    pub max_b_frames: u32,
    pub overwrite: bool,
}
#[cfg_attr(target_os = "macos", derive(Clone))]
pub struct EncodeFrame {
    pub surface: EncoderSurface,
    pub pts: i64,
    pub duration: i64,
}
/// Renders ordered 48 kHz stereo blocks relative to the beginning of the output.
pub type AudioRender =
    Box<dyn FnMut(u64, &mut [crate::StereoSample]) -> Result<(), crate::MediaError> + Send>;
pub enum EncodeAdmission {
    Accepted,
    WouldBlock(EncodeFrame),
}
#[derive(Clone, Debug)]
pub struct EncoderReport {
    pub input_frames: u64,
    pub packets: u64,
    pub hardware: bool,
    pub encoder: String,
    pub time_base: crate::RationalTimeBase,
}
#[derive(Clone, Debug)]
pub enum EncoderStatus {
    Running,
    Draining,
    ReadyToCommit(EncoderReport),
    Completed(EncoderReport),
    Cancelled,
    Failed(MediaError),
}

mod mp4;
pub use mp4::Mp4Encoder;
use crate::MediaError;
