use crate::document::{ProjectData, Rational};
use std::ops::Range;
use std::path::Path;

#[derive(Clone, Copy)]
pub struct ExportOptions {
    pub resolution: i32,
    pub frame_rate: i32,
    pub quality: i32,
    pub hdr10: bool,
}

impl ExportOptions {
    pub fn extent(self, width: u32, height: u32) -> (u32, u32) {
        let (max_w, max_h) = match self.resolution {
            1 => (1280, 720),
            2 => (1920, 1080),
            3 => (3840, 2160),
            _ => return (width, height),
        };
        let scale = (f64::from(max_w) / f64::from(width)).min(f64::from(max_h) / f64::from(height));
        let even = |value: u32| (((f64::from(value) * scale / 2.0).round() as u32).max(1)) * 2;
        (even(width), even(height))
    }

    pub fn fps(self, original: Rational) -> Rational {
        let fps = match self.frame_rate {
            1 => 24,
            2 => 25,
            3 => 30,
            4 => 60,
            _ => return original,
        };
        Rational {
            numerator: fps,
            denominator: 1,
        }
    }

    pub fn bitrate(self, width: u32, height: u32) -> u64 {
        let quality = match self.quality {
            1 => 2,
            2 => 8,
            _ => 4,
        };
        (u64::from(width) * u64::from(height))
            .saturating_mul(quality)
            .clamp(2_000_000, 80_000_000)
    }
}

/// Output frame zero samples the start of the selected sequence range.
pub struct Cadence {
    start: u64,
    input: u128,
    output: u128,
}

impl Cadence {
    pub fn new(start: u64, source: Rational, output: Rational) -> Self {
        Self {
            start,
            input: source.numerator as u128 * output.denominator as u128,
            output: output.numerator as u128 * source.denominator as u128,
        }
    }

    pub fn sample(&self, frame: u64) -> Result<u64, String> {
        let offset = u128::from(frame) * self.input / self.output;
        u64::try_from(offset + u128::from(self.start))
            .map_err(|_| "Export source position overflow".into())
    }

    pub fn boundary(&self, frame: u64) -> Result<u64, String> {
        let offset = frame
            .checked_sub(self.start)
            .ok_or("Export boundary precedes its range")?;
        let length = (u128::from(offset) * self.output).div_ceil(self.input);
        u64::try_from(length).map_err(|_| "Export frame count overflow".into())
    }
}

pub fn export_srt(project: &ProjectData, range: Range<u64>, path: &Path) -> Result<(), String> {
    use std::fmt::Write;
    let mut text = String::new();
    let mut number = 0;
    for clip in project
        .tracks
        .iter()
        .filter(|track| track.subtitles)
        .flat_map(|track| &track.clips)
    {
        let start = clip.timeline_start.max(range.start);
        let end = clip.timeline_end()?.min(range.end);
        if start >= end {
            continue;
        }
        let relative_ms = |frame: u64| {
            u128::from(frame - range.start) * project.fps.denominator as u128 * 1000
                / project.fps.numerator as u128
        };
        let (start, end) = (relative_ms(start), relative_ms(end));
        if start >= end {
            return Err("A subtitle is shorter than SRT timestamp precision".into());
        }
        number += 1;
        let title = clip.title.as_ref().ok_or("Subtitle cue has no text")?;
        let cue = title
            .text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        writeln!(
            text,
            "{number}\n{} --> {}\n{cue}\n",
            srt_time(start),
            srt_time(end),
        )
        .map_err(|error| error.to_string())?;
    }
    if number == 0 {
        return Err("The selected range contains no subtitle cues".into());
    }
    crate::document::write_new_artifact(path, text.as_bytes())
}

fn srt_time(ms: u128) -> String {
    format!(
        "{:02}:{:02}:{:02},{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cadence_repeats_or_drops_frames_without_sampling_past_the_range() {
        let fps = |numerator| Rational {
            numerator,
            denominator: 1,
        };
        let slower = Cadence::new(17, fps(30), fps(24));
        assert_eq!(slower.boundary(29), Ok(10));
        assert_eq!(slower.sample(0), Ok(17));
        assert_eq!(slower.sample(9), Ok(28));
        let faster = Cadence::new(17, fps(25), fps(60));
        assert_eq!(faster.boundary(20), Ok(8));
        assert_eq!(faster.sample(7), Ok(19));
    }
}
