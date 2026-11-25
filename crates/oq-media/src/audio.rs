//! Bounded, timestamp-addressed audio decode on a 48 kHz stereo bus.
use crate::{MediaError, RationalTimeBase};
use ffmpeg_next::{codec, ffi, format, frame, media::Type, software, ChannelLayout, Error, Packet};
use std::ops::Range;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

pub const AUDIO_RATE: u32 = 48_000;
pub const MAX_AUDIO_BLOCK: usize = 8192;
pub type StereoSample = [f32; 2];
const PACKED: format::Sample = format::Sample::F32(format::sample::Type::Packed);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioSourceInfo {
    pub stream_index: usize,
    pub time_base: RationalTimeBase,
    pub start_pts: i64,
    pub duration_pts: Option<i64>,
    pub sample_rate: u32,
    pub channels: u16,
    pub codec: String,
}

pub struct AudioProbe {
    pub audio: Option<AudioSourceInfo>,
    pub has_video: bool,
}

fn input(path: &Path, cancel: Arc<AtomicBool>) -> Result<format::context::Input, MediaError> {
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    crate::require_media_sdk()?;
    #[cfg(not(all(target_os = "macos", feature = "videotoolbox")))]
    ffmpeg_next::init().map_err(|error| format!("FFmpeg init failed: {error}"))?;
    let path = path
        .canonicalize()
        .map_err(|error| format!("Cannot access audio file: {error}"))?;
    let deadline = Arc::new(std::sync::Mutex::new(None::<Instant>));
    let open_timeout = deadline.clone();
    let interrupt_cancel = cancel.clone();
    *deadline.lock().unwrap() = Some(Instant::now());
    let result = format::input_with_interrupt(&path, move || {
        interrupt_cancel.load(Ordering::Acquire)
            || open_timeout
                .lock()
                .unwrap()
                .is_some_and(|time| time.elapsed().as_secs() >= 15)
    })
    .map_err(|error| format!("Cannot open audio input: {error}"));
    *deadline.lock().unwrap() = None;
    if cancel.load(Ordering::Acquire) {
        return Err(MediaError::time(
            crate::ErrorKind::Cancelled,
            "Audio open cancelled",
        ));
    }
    Ok(result?)
}

fn describe(
    stream: &format::stream::Stream<'_>,
) -> Result<(AudioSourceInfo, codec::decoder::Audio), MediaError> {
    let parameters = stream.parameters();
    let id = parameters.id();
    let context = codec::context::Context::from_parameters(parameters)
        .map_err(|error| format!("Invalid audio stream: {error}"))?;
    let mut decoder = context.decoder();
    decoder.set_packet_time_base(stream.time_base());
    let decoder = decoder
        .audio()
        .map_err(|error| format!("Cannot decode {} audio: {error}", id.name()))?;
    let time_base = RationalTimeBase {
        numerator: stream.time_base().0,
        denominator: stream.time_base().1,
    };
    if decoder.rate() == 0
        || decoder.channels() == 0
        || time_base.numerator <= 0
        || time_base.denominator <= 0
    {
        return Err("Audio stream lacks a sample rate, channel layout or valid time base".into());
    }
    let info = AudioSourceInfo {
        stream_index: stream.index(),
        time_base,
        start_pts: if stream.start_time() == ffi::AV_NOPTS_VALUE {
            0
        } else {
            stream.start_time()
        },
        duration_pts: (stream.duration() > 0).then_some(stream.duration()),
        sample_rate: decoder.rate(),
        channels: decoder.channels(),
        codec: id.name().into(),
    };
    Ok((info, decoder))
}

pub fn probe_audio(path: &Path) -> Result<AudioProbe, MediaError> {
    let input = input(path, Arc::new(AtomicBool::new(false)))?;
    let has_video = input
        .streams()
        .any(|stream| stream.parameters().medium() == Type::Video);
    let audio = input
        .streams()
        .best(Type::Audio)
        .and_then(|stream| describe(&stream).ok().map(|(info, _)| info));
    Ok(AudioProbe { audio, has_video })
}

/// Rescale absolute stream PTS to the signed sample grid, rounding once from origin.
pub fn audio_sample(pts: i64, time_base: RationalTimeBase) -> Result<i64, MediaError> {
    if time_base.numerator <= 0 || time_base.denominator <= 0 {
        return Err("Invalid audio time base".into());
    }
    let value = i128::from(pts) * i128::from(time_base.numerator) * i128::from(AUDIO_RATE);
    let denominator = i128::from(time_base.denominator);
    let rounded = (value.abs() + denominator / 2) / denominator * value.signum();
    i64::try_from(rounded).map_err(|_| "Audio time exceeds sample range".into())
}

struct Chunk {
    start: i64,
    samples: Vec<StereoSample>,
}

struct Segment {
    resampler: software::resampling::Context,
    start: i64,
    input_samples: i64,
    next_output: i64,
}

impl Segment {
    fn new(audio: &frame::Audio, start: i64) -> Result<Self, MediaError> {
        let resampler = software::resampling::Context::get(
            audio.format(),
            audio.channel_layout(),
            audio.rate(),
            PACKED,
            ChannelLayout::STEREO,
            AUDIO_RATE,
        )
        .map_err(|error| format!("Cannot configure audio resampling: {error}"))?;
        Ok(Self {
            resampler,
            start,
            input_samples: 0,
            next_output: start,
        })
    }

    fn accepts(&self, audio: &frame::Audio, pts: i64, tolerance: i64) -> bool {
        let definition = self.resampler.input();
        let rate = i128::from(definition.rate);
        let expected = i128::from(self.start)
            + (i128::from(self.input_samples) * i128::from(AUDIO_RATE) + rate / 2) / rate;
        audio.rate() == definition.rate
            && audio.format() == definition.format
            && audio.channel_layout() == definition.channel_layout
            && (expected - i128::from(pts)).abs() <= i128::from(tolerance)
    }

    fn convert(&mut self, audio: Option<&frame::Audio>) -> Result<Chunk, MediaError> {
        let block = audio.map_or(0, frame::Audio::samples);
        let input_len = i32::try_from(block).map_err(|_| "Audio frame exceeds supported size")?;
        let resampler = unsafe { self.resampler.as_mut_ptr() };
        let capacity = unsafe { ffi::swr_get_out_samples(resampler, input_len) };
        if capacity < 0 || capacity as usize > MAX_AUDIO_BLOCK * 4 {
            return Err("Audio frame exceeds the resampler block budget".into());
        }
        let mut samples = vec![[0.0; 2]; capacity.max(1) as usize];
        let mut output = [samples.as_mut_ptr().cast::<u8>()];
        // Input AVFrame owns extended_data, output is a writable packed stereo buffer.
        let n = unsafe {
            let input = audio.map_or(std::ptr::null_mut(), |frame| {
                (*frame.as_ptr()).extended_data as *mut *const u8
            });
            ffi::swr_convert(
                resampler,
                output.as_mut_ptr(),
                samples.len() as i32,
                input,
                input_len,
            )
        };
        if n < 0 {
            return Err(format!("Cannot resample audio: {}", Error::from(n)).into());
        }
        samples.truncate(n as usize);
        let start = self.next_output;
        self.next_output = start
            .checked_add(i64::from(n))
            .ok_or("Audio output timestamp overflow")?;
        self.input_samples = self
            .input_samples
            .checked_add(i64::from(input_len))
            .ok_or("Audio input timestamp overflow")?;
        Ok(Chunk { start, samples })
    }
}

pub struct AudioReader {
    input: format::context::Input,
    decoder: codec::decoder::Audio,
    info: AudioSourceInfo,
    cancel: Arc<AtomicBool>,
    draining: bool,
    segment: Option<Segment>,
    deferred: Option<(frame::Audio, i64)>,
    pending: Option<Chunk>,
    next_request: Option<i64>,
}

impl AudioReader {
    pub fn open(
        path: &Path,
        stream_index: usize,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self, MediaError> {
        let input = input(path, cancel.clone())?;
        let stream = input
            .stream(stream_index)
            .ok_or("Audio stream is missing")?;
        let (info, decoder) = describe(&stream)?;
        Ok(Self {
            input,
            decoder,
            info,
            cancel,
            draining: false,
            segment: None,
            deferred: None,
            pending: None,
            next_request: None,
        })
    }

    fn seek(&mut self, sample: i64) -> Result<(), MediaError> {
        let preroll = sample.saturating_sub(i64::from(AUDIO_RATE / 2));
        let tb = self.info.time_base;
        let pts = i128::from(preroll) * i128::from(tb.denominator)
            / (i128::from(AUDIO_RATE) * i128::from(tb.numerator));
        let target = i64::try_from(pts).map_err(|_| "Audio seek exceeds supported range")?;
        let target = target.max(self.info.start_pts);
        let result = unsafe {
            ffi::avformat_seek_file(
                self.input.as_mut_ptr(),
                self.info.stream_index as i32,
                i64::MIN,
                target,
                target,
                0,
            )
        };
        if result < 0 {
            return Err(format!("Cannot seek audio: {}", Error::from(result)).into());
        }
        self.decoder.flush();
        self.draining = false;
        self.segment = None;
        self.deferred = None;
        self.pending = None;
        Ok(())
    }

    /// Read a half-open interval. Samples outside source coverage are silence.
    pub fn read(
        &mut self,
        range: Range<i64>,
        output: &mut [StereoSample],
    ) -> Result<(), MediaError> {
        if output.len() > MAX_AUDIO_BLOCK
            || range.end.checked_sub(range.start) != Some(output.len() as i64)
        {
            return Err("Invalid PCM request range".into());
        }
        output.fill([0.0; 2]);
        if self.cancel.load(Ordering::Acquire) {
            return Err(MediaError::time(crate::ErrorKind::Cancelled, "Audio read cancelled"));
        }
        if let Some(end) = self.info.duration_pts.and_then(|duration| self.info.start_pts.checked_add(duration)) {
            if range.start >= audio_sample(end, self.info.time_base)? {
                return Ok(());
            }
        }
        if self.next_request.is_some_and(|next| next != range.start)
            || self.next_request.is_none()
                && range.start
                    > audio_sample(self.info.start_pts, self.info.time_base)?
                        .saturating_add(i64::from(AUDIO_RATE / 2))
        {
            self.seek(range.start)?;
        }
        loop {
            if self.cancel.load(Ordering::Acquire) {
                return Err(MediaError::time(
                    crate::ErrorKind::Cancelled,
                    "Audio read cancelled",
                ));
            }
            let Some(chunk) = self
                .pending
                .take()
                .map_or_else(|| self.next_chunk(), |chunk| Ok(Some(chunk)))?
            else {
                break;
            };
            let end = chunk
                .start
                .checked_add(chunk.samples.len() as i64)
                .ok_or("Audio chunk timestamp overflow")?;
            let start = range.start.max(chunk.start);
            let stop = range.end.min(end);
            if stop > start {
                let from = (start - chunk.start) as usize;
                let to = (start - range.start) as usize;
                let count = (stop - start) as usize;
                output[to..to + count].copy_from_slice(&chunk.samples[from..from + count]);
            }
            if end >= range.end {
                self.pending = (end > range.end).then_some(chunk);
                break;
            }
        }
        self.next_request = Some(range.end);
        Ok(())
    }

    fn next_chunk(&mut self) -> Result<Option<Chunk>, MediaError> {
        loop {
            let next = match self.deferred.take() {
                Some(frame) => Some(frame),
                None => self.next_frame()?,
            };
            if let Some((audio, pts)) = next {
                let tolerance = audio_sample(1, self.info.time_base)?.max(1);
                if self
                    .segment
                    .as_ref()
                    .is_some_and(|segment| !segment.accepts(&audio, pts, tolerance))
                {
                    self.deferred = Some((audio, pts));
                    if let Some(chunk) = self.flush_segment()? {
                        return Ok(Some(chunk));
                    }
                    continue;
                }
                if self.segment.is_none() {
                    self.segment = Some(Segment::new(&audio, pts)?);
                }
                let chunk = self
                    .segment
                    .as_mut()
                    .expect("segment opened")
                    .convert(Some(&audio))?;
                if !chunk.samples.is_empty() {
                    return Ok(Some(chunk));
                }
            } else {
                return self.flush_segment();
            }
        }
    }

    fn flush_segment(&mut self) -> Result<Option<Chunk>, MediaError> {
        let Some(mut segment) = self.segment.take() else {
            return Ok(None);
        };
        let chunk = segment.convert(None)?;
        if chunk.samples.is_empty() {
            return Ok(None);
        }
        self.segment = Some(segment);
        Ok(Some(chunk))
    }

    fn next_frame(&mut self) -> Result<Option<(frame::Audio, i64)>, MediaError> {
        let mut audio = frame::Audio::empty();
        loop {
            if self.cancel.load(Ordering::Acquire) {
                return Err(MediaError::time(
                    crate::ErrorKind::Cancelled,
                    "Audio read cancelled",
                ));
            }
            match self.decoder.receive_frame(&mut audio) {
                Ok(()) => {
                    let pts = audio
                        .timestamp()
                        .or_else(|| audio.pts())
                        .ok_or("Decoded audio frame has no presentation timestamp")?;
                    if audio.channel_layout().is_empty() {
                        audio.set_channel_layout(ChannelLayout::default(i32::from(
                            audio.channels(),
                        )));
                    }
                    return Ok(Some((audio, audio_sample(pts, self.info.time_base)?)));
                }
                Err(Error::Eof) => return Ok(None),
                Err(Error::Other {
                    errno: ffmpeg_next::error::EAGAIN,
                }) if !self.draining => {}
                Err(error) => return Err(format!("Cannot receive decoded audio: {error}").into()),
            }
            let mut packet = Packet::empty();
            match packet.read(&mut self.input) {
                Ok(()) if packet.stream() != self.info.stream_index => continue,
                Ok(()) => self
                    .decoder
                    .send_packet(&packet)
                    .map_err(|error| format!("Cannot submit audio packet: {error}"))?,
                Err(Error::Eof) => {
                    self.decoder
                        .send_eof()
                        .map_err(|error| format!("Cannot drain audio decoder: {error}"))?;
                    self.draining = true;
                }
                Err(error) => return Err(format!("Cannot read audio packet: {error}").into()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_reads_exact_samples_across_forward_and_backward_seeks() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("impulses.wav");
        let count = 96_000u32;
        let mut bytes = Vec::new();
        bytes.extend(b"RIFF");
        bytes.extend((36 + count * 4).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(AUDIO_RATE.to_le_bytes());
        bytes.extend((AUDIO_RATE * 4).to_le_bytes());
        bytes.extend(4u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend((count * 4).to_le_bytes());
        for index in 0..count {
            let left: i16 = if index == 3 { 16384 } else { 0 };
            let right: i16 = if index == 48002 { -16384 } else { 0 };
            bytes.extend(left.to_le_bytes());
            bytes.extend(right.to_le_bytes());
        }
        std::fs::write(&path, bytes).unwrap();
        let info = probe_audio(&path).unwrap().audio.unwrap();
        assert_eq!(info.duration_pts, Some(i64::from(count)));
        let cancel = Arc::new(AtomicBool::new(false));
        let mut reader = AudioReader::open(&path, info.stream_index, cancel.clone()).unwrap();
        let mut block = [[0.0; 2]; 8];
        reader.read(-4..4, &mut block).unwrap();
        assert_eq!(block[7], [0.5, 0.0]);
        reader.read(48000..48008, &mut block).unwrap();
        assert_eq!(block[2], [0.0, -0.5]);
        reader.read(192000..192008, &mut block).unwrap();
        assert_eq!(block, [[0.0; 2]; 8]);
        reader.read(0..8, &mut block).unwrap();
        assert_eq!(block[3], [0.5, 0.0]);
        cancel.store(true, Ordering::Release);
        assert_eq!(
            reader.read(8..16, &mut block).unwrap_err().kind,
            crate::ErrorKind::Cancelled
        );
    }
}
