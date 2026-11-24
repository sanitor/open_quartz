use crate::{AudioRender, MediaError, StereoSample, AUDIO_RATE, MAX_AUDIO_BLOCK};
use ffmpeg_next::{codec, ffi, format, frame, ChannelLayout, Error, Packet};

const PLANAR: format::Sample = format::Sample::F32(format::sample::Type::Planar);
const TIME_BASE: (i32, i32) = (1, AUDIO_RATE as i32);

pub(super) struct AudioEncoder {
    encoder: codec::encoder::audio::Encoder,
    render: AudioRender,
    samples: Vec<StereoSample>,
    next: i64,
    stream: usize,
}

impl AudioEncoder {
    pub(super) fn open(
        output: &mut format::context::Output,
        render: AudioRender,
    ) -> Result<Self, MediaError> {
        let codec =
            codec::encoder::find_by_name("aac").ok_or("FFmpeg AAC encoder is unavailable")?;
        let mut encoder = codec::context::Context::new_with_codec(codec)
            .encoder()
            .audio()
            .map_err(|error| format!("Cannot create AAC encoder: {error}"))?;
        encoder.set_rate(AUDIO_RATE as i32);
        encoder.set_format(PLANAR);
        encoder.set_channel_layout(ChannelLayout::STEREO);
        encoder.set_time_base(TIME_BASE);
        encoder.set_bit_rate(192_000);
        encoder.set_flags(codec::Flags::GLOBAL_HEADER);
        let encoder = encoder
            .open()
            .map_err(|error| format!("Cannot open AAC encoder: {error}"))?;
        let size = encoder.frame_size() as usize;
        if size == 0 || size > MAX_AUDIO_BLOCK {
            return Err("Unsupported AAC frame size".into());
        }
        let mut stream = output
            .add_stream(codec)
            .map_err(|error| format!("Cannot add AAC stream: {error}"))?;
        stream.set_time_base(TIME_BASE);
        stream.set_parameters(&encoder);
        Ok(Self {
            encoder,
            render,
            samples: vec![[0.0; 2]; size],
            next: 0,
            stream: stream.index(),
        })
    }

    pub(super) fn encode_until(
        &mut self,
        output: &mut format::context::Output,
        end: i64,
        final_block: bool,
    ) -> Result<(), MediaError> {
        let size = self.samples.len() as i64;
        while self.next < end {
            let left = end.checked_sub(self.next).ok_or("AAC timestamp overflow")?;
            if left < size && !final_block {
                break;
            }
            let count = left.min(size) as usize;
            let samples = &mut self.samples[..count];
            (self.render)(self.next as u64, samples)?;
            let mut pcm = frame::Audio::empty();
            pcm.set_format(PLANAR);
            pcm.set_channel_layout(ChannelLayout::STEREO);
            pcm.set_rate(AUDIO_RATE);
            pcm.set_samples(count);
            pcm.set_pts(Some(self.next));
            if unsafe { ffi::av_frame_get_buffer(pcm.as_mut_ptr(), 0) } < 0 {
                return Err("Cannot allocate an AAC input block".into());
            }
            for channel in 0..2 {
                for (destination, source) in pcm.plane_mut::<f32>(channel).iter_mut().zip(&*samples)
                {
                    *destination = source[channel];
                }
            }
            self.encoder
                .send_frame(&pcm)
                .map_err(|error| format!("Cannot submit AAC block: {error}"))?;
            self.next += count as i64;
            self.mux(output)?;
        }
        Ok(())
    }

    fn mux(&mut self, output: &mut format::context::Output) -> Result<(), MediaError> {
        let time_base = output
            .stream(self.stream)
            .ok_or("Missing MP4 audio stream")?
            .time_base();
        loop {
            let mut packet = Packet::empty();
            match self.encoder.receive_packet(&mut packet) {
                Ok(()) => {
                    packet.set_stream(self.stream);
                    packet.rescale_ts(TIME_BASE, time_base);
                    packet
                        .write_interleaved(output)
                        .map_err(|error| format!("Cannot mux AAC: {error}"))?;
                }
                Err(Error::Other {
                    errno: ffmpeg_next::error::EAGAIN,
                })
                | Err(Error::Eof) => return Ok(()),
                Err(error) => return Err(format!("Cannot receive AAC packet: {error}").into()),
            }
        }
    }

    pub(super) fn finish(
        &mut self,
        output: &mut format::context::Output,
    ) -> Result<(), MediaError> {
        self.encoder
            .send_eof()
            .map_err(|error| format!("Cannot drain AAC: {error}"))?;
        self.mux(output)
    }
}
