use crate::document::normal_speed;
use oq_media::{AudioReader, MediaError, StereoSample, MAX_AUDIO_BLOCK};
use std::path::Path;
use std::sync::{atomic::AtomicBool, Arc};

const HALF: i64 = 12;
const TAPS: usize = (2 * HALF) as usize;
const OUTPUT_BLOCK: usize = 1024;
const PHASES: u128 = 100;

pub struct RateReader {
    reader: AudioReader,
    percent: u16,
    kernel: Vec<[f32; TAPS]>,
    input_start: i64,
    input: Vec<StereoSample>,
}

impl RateReader {
    pub fn open(
        path: &Path,
        stream: usize,
        percent: u16,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self, MediaError> {
        let reader = AudioReader::open(path, stream, cancel)?;
        let kernel = if percent == normal_speed() {
            Vec::new()
        } else {
            let cutoff = (100.0 / f64::from(percent)).min(1.0);
            (0..PHASES)
                .map(|phase| {
                    let fraction = phase as f64 / PHASES as f64;
                    let mut taps = std::array::from_fn(|index| {
                        let distance = (index as i64 - HALF + 1) as f64 - fraction;
                        (cutoff * sinc(distance * cutoff) * sinc(distance / HALF as f64)) as f32
                    });
                    let gain: f32 = taps.iter().sum();
                    taps.iter_mut().for_each(|tap| *tap /= gain);
                    taps
                })
                .collect()
        };
        Ok(Self {
            reader,
            percent,
            kernel,
            input_start: 0,
            input: Vec::new(),
        })
    }

    pub fn render(
        &mut self,
        source: i64,
        offset: u64,
        output: &mut [StereoSample],
    ) -> Result<(), MediaError> {
        if self.kernel.is_empty() {
            let start = source_index(source, u128::from(offset))?;
            let end = start
                .checked_add(output.len() as i64)
                .ok_or("Audio source range overflow")?;
            return self.reader.read(start..end, output);
        }
        let mut position = u128::from(offset) * u128::from(self.percent);
        for block in output.chunks_mut(OUTPUT_BLOCK) {
            let last = position + (block.len() - 1) as u128 * u128::from(self.percent);
            let first_sample = source_index(source, position / PHASES)?;
            let start = first_sample
                .checked_sub(HALF - 1)
                .ok_or("Audio filter start overflow")?;
            let end = source_index(source, last / PHASES)?
                .checked_add(HALF + 1)
                .ok_or("Audio filter end overflow")?;
            self.buffer(start, end)?;
            let first_position = position / PHASES;
            for frame in block {
                let index = (position / PHASES - first_position) as usize;
                let coefficients = &self.kernel[(position % PHASES) as usize];
                let mut sum = [0.0; 2];
                for (value, gain) in self.input[index..index + TAPS].iter().zip(coefficients) {
                    for channel in 0..2 {
                        if value[channel].is_finite() {
                            sum[channel] += value[channel] * gain;
                        }
                    }
                }
                *frame = sum;
                position += u128::from(self.percent);
            }
        }
        Ok(())
    }

    fn buffer(&mut self, start: i64, end: i64) -> Result<(), MediaError> {
        let count = usize::try_from(end.checked_sub(start).ok_or("Audio input span overflow")?)
            .map_err(|_| "Audio input span overflow")?;
        if count > MAX_AUDIO_BLOCK {
            return Err("Audio input span exceeds decoder block size".into());
        }
        let mut retained = 0;
        // Adjacent filter windows share samples. Keep the overlap and advance
        // the decoder cursor only for the missing suffix.
        if let Some(offset) = start
            .checked_sub(self.input_start)
            .and_then(|diff| usize::try_from(diff).ok())
        {
            if offset < self.input.len() {
                retained = (self.input.len() - offset).min(count);
                self.input.copy_within(offset..offset + retained, 0);
            }
        }
        self.input_start = start;
        self.input.resize(count, [0.0; 2]);
        if retained < count {
            self.reader
                .read(start + retained as i64..end, &mut self.input[retained..])?;
        }
        Ok(())
    }
}

fn source_index(source: i64, offset: u128) -> Result<i64, MediaError> {
    let offset = i128::try_from(offset).map_err(|_| "Audio source offset overflow")?;
    i64::try_from(i128::from(source) + offset).map_err(|_| "Audio source position overflow".into())
}

fn sinc(value: f64) -> f64 {
    if value.abs() < f64::EPSILON {
        1.0
    } else {
        let radians = std::f64::consts::PI * value;
        radians.sin() / radians
    }
}
