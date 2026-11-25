//! Windows hardware encoder and its native NV12/P010 input pool.
use crate::{MediaError, RationalTimeBase};
use ffmpeg_next::{codec, color, ffi, format::Pixel, frame::Video, Dictionary, Packet};
use std::{collections::BTreeMap, ffi::c_void, ptr, ptr::NonNull};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Profile {
    Sdr,
    Hdr10,
    Hlg,
}

impl Profile {
    pub fn codec_name(self) -> &'static str {
        match self {
            Self::Sdr => "h264_d3d12va",
            Self::Hdr10 | Self::Hlg => "hevc_d3d12va",
        }
    }

    fn input_format(self) -> Pixel {
        match self {
            Self::Sdr => Pixel::NV12,
            Self::Hdr10 | Self::Hlg => Pixel::P010LE,
        }
    }

    fn color(
        self,
    ) -> (
        color::Space,
        color::Primaries,
        color::TransferCharacteristic,
    ) {
        use color::{Primaries, Space, TransferCharacteristic};
        match self {
            Self::Sdr => (
                Space::BT709,
                Primaries::BT709,
                TransferCharacteristic::BT709,
            ),
            Self::Hdr10 => (
                Space::BT2020NCL,
                Primaries::BT2020,
                TransferCharacteristic::SMPTE2084,
            ),
            Self::Hlg => (
                Space::BT2020NCL,
                Primaries::BT2020,
                TransferCharacteristic::ARIB_STD_B67,
            ),
        }
    }
}

struct Buffer(NonNull<ffi::AVBufferRef>);

impl Drop for Buffer {
    fn drop(&mut self) {
        unsafe { ffi::av_buffer_unref(&mut self.0.as_ptr()) }
    }
}

pub struct D3d12Encoder {
    encoder: codec::encoder::video::Encoder,
    frames: Buffer,
    profile: Profile,
    durations: BTreeMap<i64, i64>,
}

pub struct InputSurface(Video);

impl InputSurface {
    /// Borrow the AVD3D12VAFrame descriptor for the life of this surface.
    /// GPU writes must complete before the surface is sent to the encoder.
    pub fn descriptor(&self) -> *const c_void {
        unsafe { (*self.0.as_ptr()).data[0].cast() }
    }
}

impl D3d12Encoder {
    pub fn open(
        width: u32,
        height: u32,
        time_base: RationalTimeBase,
        bitrate: usize,
        profile: Profile,
    ) -> Result<Self, MediaError> {
        if width == 0
            || height == 0
            || width % 2 != 0
            || height % 2 != 0
            || width > i32::MAX as u32
            || height > i32::MAX as u32
            || time_base.numerator <= 0
            || time_base.denominator <= 0
            || bitrate == 0
        {
            return Err("Invalid D3D12 encoder dimensions, time base or bitrate".into());
        }
        ffmpeg_next::init().map_err(|error| format!("FFmpeg init failed: {error}"))?;
        let mut raw_device = ptr::null_mut();
        let result = unsafe {
            ffi::av_hwdevice_ctx_create(
                &mut raw_device,
                ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D12VA,
                ptr::null(),
                ptr::null_mut(),
                0,
            )
        };
        check(result, "Create D3D12 encoder device")?;
        let device = Buffer(NonNull::new(raw_device).ok_or("Missing D3D12 device")?);
        let frames = Buffer(
            NonNull::new(unsafe { ffi::av_hwframe_ctx_alloc(device.0.as_ptr()) })
                .ok_or("Cannot allocate D3D12 encoder frames context")?,
        );
        let context = unsafe { &mut *frames.0.as_ref().data.cast::<ffi::AVHWFramesContext>() };
        context.format = Pixel::D3D12.into();
        context.sw_format = profile.input_format().into();
        context.width = width as i32;
        context.height = height as i32;
        check(
            unsafe { ffi::av_hwframe_ctx_init(frames.0.as_ptr()) },
            "Initialize D3D12 frames",
        )?;

        let codec = codec::encoder::find_by_name(profile.codec_name())
            .ok_or_else(|| format!("FFmpeg {} encoder is unavailable", profile.codec_name()))?;
        let mut video = codec::context::Context::new_with_codec(codec)
            .encoder()
            .video()
            .map_err(|error| format!("Create D3D12 encoder context: {error}"))?;
        video.set_width(width);
        video.set_height(height);
        video.set_format(Pixel::D3D12);
        video.set_time_base((time_base.numerator, time_base.denominator));
        video.set_frame_rate(Some((time_base.denominator, time_base.numerator)));
        video.set_bit_rate(bitrate);
        video.set_flags(codec::Flags::GLOBAL_HEADER);
        let (space, primaries, transfer) = profile.color();
        video.set_color_range(color::Range::MPEG);
        video.set_colorspace(space);
        video.set_color_primaries(primaries);
        video.set_color_transfer_characteristic(transfer);
        unsafe {
            let raw = &mut *video.as_mut_ptr();
            raw.sw_pix_fmt = profile.input_format().into();
            raw.hw_frames_ctx = ffi::av_buffer_ref(frames.0.as_ptr());
            if raw.hw_frames_ctx.is_null() {
                return Err("Cannot retain D3D12 frames context".into());
            }
        }
        let mut options = Dictionary::new();
        if profile != Profile::Sdr {
            options.set("profile", "main10");
        }
        let encoder = video
            .open_with(options)
            .map_err(|error| format!("Open required {} encoder: {error}", profile.codec_name()))?;
        Ok(Self {
            encoder,
            frames,
            profile,
            durations: BTreeMap::new(),
        })
    }

    pub fn parameters(&self) -> codec::Parameters {
        codec::Parameters::from(&self.encoder)
    }

    pub fn allocate(&self) -> Result<InputSurface, MediaError> {
        let mut frame = Video::empty();
        check(
            unsafe { ffi::av_hwframe_get_buffer(self.frames.0.as_ptr(), frame.as_mut_ptr(), 0) },
            "Allocate D3D12 encoder surface",
        )?;
        Ok(InputSurface(frame))
    }

    pub fn encode(
        &mut self,
        mut surface: InputSurface,
        pts: i64,
        duration: i64,
    ) -> Result<Vec<Packet>, MediaError> {
        let context = unsafe { (*surface.0.as_ptr()).hw_frames_ctx };
        if unsafe { (*context).data != self.frames.0.as_ref().data }
            || duration <= 0
            || self.durations.contains_key(&pts)
        {
            return Err("D3D12 surface pool or frame duration is invalid".into());
        }
        let (space, primaries, transfer) = self.profile.color();
        let frame = &mut surface.0;
        frame.set_color_range(color::Range::MPEG);
        frame.set_color_space(space);
        frame.set_color_primaries(primaries);
        frame.set_color_transfer_characteristic(transfer);
        frame.set_pts(Some(pts));
        unsafe { (*frame.as_mut_ptr()).duration = duration };
        self.encoder
            .send_frame(frame)
            .map_err(|error| format!("D3D12 encoder rejected frame: {error}"))?;
        self.durations.insert(pts, duration);
        self.packets(false)
    }

    pub fn drain(&mut self) -> Result<Vec<Packet>, MediaError> {
        self.encoder
            .send_eof()
            .map_err(|error| format!("Drain D3D12 encoder: {error}"))?;
        self.packets(true)
    }

    fn packets(&mut self, draining: bool) -> Result<Vec<Packet>, MediaError> {
        let mut output = Vec::new();
        loop {
            let mut packet = Packet::empty();
            match self.encoder.receive_packet(&mut packet) {
                Ok(()) => {
                    let duration = packet
                        .pts()
                        .and_then(|pts| self.durations.remove(&pts))
                        .ok_or("D3D12 encoder returned an unknown presentation timestamp")?;
                    packet.set_duration(duration);
                    output.push(packet);
                }
                Err(ffmpeg_next::Error::Eof) if self.durations.is_empty() => return Ok(output),
                Err(ffmpeg_next::Error::Eof) => {
                    return Err("D3D12 encoder ended with pending frames".into())
                }
                Err(ffmpeg_next::Error::Other {
                    errno: ffmpeg_next::error::EAGAIN,
                }) if !draining => return Ok(output),
                Err(error) => return Err(format!("Read D3D12 encoder packet: {error}").into()),
            }
        }
    }
}

fn check(result: i32, operation: &str) -> Result<(), MediaError> {
    if result < 0 {
        return Err(format!("{operation}: {}", ffmpeg_next::Error::from(result)).into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires native D3D12 H.264 and Main 10 hardware encoding"]
    fn native_encoder_preserves_pts_and_color_for_sdr_pq_and_hlg() {
        let time_base = RationalTimeBase {
            numerator: 1,
            denominator: 30,
        };
        let tick = ffmpeg_next::Rational(1, 30);
        let directory = tempfile::tempdir().unwrap();
        for profile in [Profile::Sdr, Profile::Hdr10, Profile::Hlg] {
            let mut encoder = D3d12Encoder::open(320, 180, time_base, 1_000_000, profile).unwrap();
            let parameters = encoder.parameters();
            let raw = unsafe { &*parameters.as_ptr() };
            let (space, primaries, transfer) = profile.color();
            assert_eq!(raw.color_space, space.into());
            assert_eq!(raw.color_primaries, primaries.into());
            assert_eq!(raw.color_trc, transfer.into());
            assert!(!raw.extradata.is_null());
            let mut packets = Vec::new();
            let mut fixture = Video::new(profile.input_format(), 320, 180);
            for plane in 0..2 {
                if profile == Profile::Sdr {
                    fixture
                        .data_mut(plane)
                        .fill(if plane == 0 { 96 } else { 128 });
                } else {
                    let code: u16 = if plane == 0 { 384 } else { 512 };
                    for sample in fixture.data_mut(plane).chunks_exact_mut(2) {
                        sample.copy_from_slice(&(code << 6).to_le_bytes());
                    }
                }
            }
            for pts in 0..4 {
                let mut surface = encoder.allocate().unwrap();
                assert!(!surface.descriptor().is_null());
                check(
                    unsafe {
                        ffi::av_hwframe_transfer_data(surface.0.as_mut_ptr(), fixture.as_ptr(), 0)
                    },
                    "Upload test fixture",
                )
                .unwrap();
                packets.extend(encoder.encode(surface, pts, 1).unwrap());
            }
            packets.extend(encoder.drain().unwrap());
            let mut pts: Vec<_> = packets.iter().map(|packet| packet.pts().unwrap()).collect();
            pts.sort_unstable();
            assert_eq!(pts, [0, 1, 2, 3]);
            assert!(packets[0].is_key());

            let path = directory.path().join(format!("{profile:?}.mp4"));
            let mut muxer = ffmpeg_next::format::output(&path).unwrap();
            let mut stream = muxer
                .add_stream(codec::encoder::find(codec::Id::None))
                .unwrap();
            stream.set_parameters(parameters);
            stream.set_time_base(tick);
            muxer.write_header().unwrap();
            let stream_tick = muxer.stream(0).unwrap().time_base();
            for mut packet in packets {
                packet.set_stream(0);
                packet.rescale_ts(tick, stream_tick);
                packet.write_interleaved(&mut muxer).unwrap();
            }
            muxer.write_trailer().unwrap();
            drop(muxer);

            let mut decoded = crate::Session::open(path).unwrap();
            let mut presentation_times = Vec::new();
            while let Some(frame) = decoded.next_frame().unwrap() {
                let frame = frame.frame;
                assert_eq!(
                    frame.format(),
                    if profile == Profile::Sdr {
                        crate::PixelFormat::Nv12
                    } else {
                        crate::PixelFormat::P010
                    }
                );
                assert_eq!(
                    frame.color().transfer,
                    match profile {
                        Profile::Sdr => crate::TransferFunction::Bt709,
                        Profile::Hdr10 => crate::TransferFunction::Pq,
                        Profile::Hlg => crate::TransferFunction::Hlg,
                    }
                );
                presentation_times.push(frame.pts_ns().unwrap());
            }
            assert_eq!(presentation_times, [0, 33_333_333, 66_666_667, 100_000_000]);
        }
    }
}
