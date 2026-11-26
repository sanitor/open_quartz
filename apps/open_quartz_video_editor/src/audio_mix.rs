use crate::{audio_rate::RateReader, document::ProjectData};
use oq_media::{MediaError, StereoSample, MAX_AUDIO_BLOCK};
use std::collections::{BTreeSet, HashMap};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

struct MixClip {
    track: usize,
    start: u64,
    end: u64,
    source: i64,
    speed_percent: u16,
    path: PathBuf,
    stream: usize,
    gain: f32,
    fade_in: u64,
    fade_out: u64,
    body_start: u64,
    body_end: u64,
    cross_in: Option<Range<u64>>,
    cross_out: Option<Range<u64>>,
}

struct NestedMix {
    mixer: Box<AudioMix>,
    start: u64,
    end: u64,
    source: u128,
    speed: u16,
}

pub struct AudioMix {
    clips: Vec<MixClip>,
    readers: HashMap<usize, RateReader>,
    nested: Vec<NestedMix>,
    cancel: Arc<AtomicBool>,
    scratch: Vec<StereoSample>,
    track_output: Vec<StereoSample>,
    pub track_peaks: Vec<f32>,
    pub output_peak: f32,
}

impl AudioMix {
    pub fn new(
        project: &ProjectData,
        cancel: Arc<AtomicBool>,
        samples: Option<&Range<u64>>,
    ) -> Result<Option<Self>, String> {
        let has_audio_track = project.audio_tracks.iter().any(|track| !track.clips.is_empty());
        let mut nested = Vec::new();
        for track in project.tracks.iter().filter(|track| track.visible) {
            for clip in &track.clips {
                let Some(asset) = project.assets.iter().find(|asset| asset.id == clip.asset_id) else { continue; };
                let Some(id) = asset.sequence_source() else { continue; };
                let start = project.fps.frame_to_sample(clip.timeline_start)?;
                let end = project.fps.frame_to_sample(clip.timeline_end()?)?;
                let from = samples.map_or(start, |range| start.max(range.start));
                let to = samples.map_or(end, |range| end.min(range.end));
                if from >= to { continue; }
                let source_ns = crate::document::source_time_ns(asset, clip, clip.timeline_start, project.fps)?;
                let source = (u128::from(source_ns) * 4_800_000 + 500_000_000) / 1_000_000_000;
                let first = source + u128::from(from - start) * u128::from(clip.speed_percent);
                let last = source + u128::from(to - 1 - start) * u128::from(clip.speed_percent);
                let range = u64::try_from(first / 100).map_err(|_| "Nested audio offset overflow")?
                    ..u64::try_from(last.div_ceil(100) + 1).map_err(|_| "Nested audio end overflow")?;
                let child = project.sequence_project(id)?;
                if let Some(mixer) = Self::new(&child, cancel.clone(), Some(&range))? {
                    nested.push(NestedMix { mixer: Box::new(mixer), start, end, source, speed: clip.speed_percent });
                }
            }
        }
        if !has_audio_track && nested.is_empty() { return Ok(None); }
        let mut clips = Vec::new();
        let has_solo = project.audio_tracks.iter().any(|track| track.solo);
        for (track_index, track) in project.audio_tracks.iter().enumerate() {
            if track.muted || track.gain == 0.0 || (has_solo && !track.solo) {
                continue;
            }
            let transitions = track.blends(project)?;
            for clip in &track.clips {
                if clip.gain == 0.0 {
                    continue;
                }
                let interval = clip.interval(project)?;
                let cross_in = transitions
                    .iter()
                    .find(|fade| fade.incoming == clip.id)
                    .map(|fade| fade.start..fade.end);
                let cross_out = transitions
                    .iter()
                    .find(|fade| fade.outgoing == clip.id)
                    .map(|fade| fade.start..fade.end);
                let start = cross_in.as_ref().map_or(interval.start, |fade| fade.start);
                let end = cross_out.as_ref().map_or(interval.end, |fade| fade.end);
                if samples.is_some_and(|range| {
                    range.is_empty() || start >= range.end || end <= range.start
                }) {
                    continue;
                }
                let asset = project
                    .assets
                    .iter()
                    .find(|asset| asset.id == clip.asset_id)
                    .ok_or("Audio asset is missing")?;
                if !asset.online {
                    return Err(format!("Audio media offline: {} — use Relink", asset.name));
                }
                let stream = asset
                    .audio
                    .as_ref()
                    .ok_or("Audio stream is missing")?
                    .stream;
                clips.push(MixClip {
                    track: track_index,
                    start,
                    end,
                    source: interval
                        .source
                        .checked_sub(
                            i64::try_from(interval.start - start).map_err(|_| {
                                "Audio crossfade start is outside the supported range"
                            })?,
                        )
                        .ok_or("Audio crossfade source sample overflow")?,
                    speed_percent: interval.speed_percent,
                    path: PathBuf::from(&asset.path),
                    stream,
                    gain: track.gain * clip.gain,
                    fade_in: clip.fade_in,
                    fade_out: clip.fade_out,
                    body_start: interval.start,
                    body_end: interval.end,
                    cross_in,
                    cross_out,
                });
            }
        }
        Ok(Some(Self {
            clips,
            readers: HashMap::new(),
            nested,
            cancel,
            scratch: vec![[0.0; 2]; MAX_AUDIO_BLOCK],
            track_output: vec![[0.0; 2]; MAX_AUDIO_BLOCK],
            track_peaks: vec![0.0; project.audio_tracks.len()],
            output_peak: 0.0,
        }))
    }

    pub fn sources(&self) -> impl Iterator<Item = &PathBuf> {
        let mut sources = BTreeSet::new();
        self.collect_sources(&mut sources);
        sources.into_iter()
    }

    fn collect_sources<'a>(&'a self, sources: &mut BTreeSet<&'a PathBuf>) {
        sources.extend(self.clips.iter().map(|clip| &clip.path));
        for nested in &self.nested { nested.mixer.collect_sources(sources); }
    }

    pub fn render(&mut self, start: u64, output: &mut [StereoSample]) -> Result<(), MediaError> {
        self.render_block(start, output, true)
    }

    pub fn render_unclipped(&mut self, start: u64, output: &mut [StereoSample]) -> Result<(), MediaError> {
        self.render_block(start, output, false)
    }

    fn render_block(&mut self, start: u64, output: &mut [StereoSample], clip: bool) -> Result<(), MediaError> {
        let end = start
            .checked_add(output.len() as u64)
            .ok_or("Audio render range overflow")?;
        if output.len() > MAX_AUDIO_BLOCK {
            return Err("Audio block is too large".into());
        }
        output.fill([0.0; 2]);
        self.track_peaks.fill(0.0);
        let track_output = &mut self.track_output[..output.len()];
        track_output.fill([0.0; 2]);
        let mut active_track = None;
        self.readers.retain(|index, _| {
            let clip = &self.clips[*index];
            clip.start < end && clip.end > start
        });
        for (index, clip) in self.clips.iter().enumerate() {
            let from = clip.start.max(start);
            let to = clip.end.min(end);
            if from >= to {
                continue;
            }
            if active_track != Some(clip.track) {
                if let Some(previous) = active_track {
                    sum_track(track_output, output, &mut self.track_peaks[previous]);
                    track_output.fill([0.0; 2]);
                }
                active_track = Some(clip.track);
            }
            let reader = match self.readers.entry(index) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => entry.insert(RateReader::open(
                    &clip.path,
                    clip.stream,
                    clip.speed_percent,
                    self.cancel.clone(),
                )?),
            };
            let len = (to - from) as usize;
            let scratch = &mut self.scratch[..len];
            reader.render(clip.source, from - clip.start, scratch)?;
            let destination = (from - start) as usize;
            for (index, (sum, sample)) in track_output[destination..destination + len]
                .iter_mut()
                .zip(scratch)
                .enumerate()
            {
                let at = from + index as u64;
                let fade_in = if clip.fade_in > 0 {
                    (at.saturating_sub(clip.body_start) as f64 / clip.fade_in as f64).min(1.0)
                } else {
                    1.0
                };
                let fade_out = if clip.fade_out > 0 {
                    ((clip.body_end - 1).saturating_sub(at) as f64 / clip.fade_out as f64).min(1.0)
                } else {
                    1.0
                };
                let cross_in = clip
                    .cross_in
                    .as_ref()
                    .map_or(1.0, |fade| fade_progress(fade, at).sqrt());
                let cross_out = clip
                    .cross_out
                    .as_ref()
                    .map_or(1.0, |fade| (1.0 - fade_progress(fade, at)).sqrt());
                let gain = clip.gain * (fade_in * fade_out) as f32 * cross_in * cross_out;
                for channel in 0..2 {
                    if sample[channel].is_finite() {
                        sum[channel] += sample[channel] * gain;
                    }
                }
            }
        }
        if let Some(track) = active_track {
            sum_track(track_output, output, &mut self.track_peaks[track]);
        }
        for nested in &mut self.nested {
            let from = start.max(nested.start);
            let to = end.min(nested.end);
            if from >= to { continue; }
            nested.render(from, &mut output[(from - start) as usize..(to - start) as usize], &mut self.scratch)?;
        }
        self.output_peak = 0.0;
        for sample in output.iter_mut().flatten() {
            self.output_peak = self.output_peak.max(sample.abs());
            if clip { *sample = sample.clamp(-1.0, 1.0); }
        }
        Ok(())
    }
}

fn fade_progress(fade: &Range<u64>, time: u64) -> f32 {
    (time.saturating_sub(fade.start) as f32 / (fade.end - fade.start - 1) as f32).min(1.0)
}

fn sum_track(source: &[StereoSample], output: &mut [StereoSample], peak: &mut f32) {
    for (track, mix) in source.iter().zip(output) {
        for (sample, sum) in track.iter().zip(mix) {
            *peak = peak.max(sample.abs());
            *sum += sample;
        }
    }
}


impl NestedMix {
    fn render(&mut self, from: u64, output: &mut [StereoSample], scratch: &mut [StereoSample]) -> Result<(), MediaError> {
        let speed = u128::from(self.speed);
        let mut position = self.source + u128::from(from - self.start) * speed;
        // Allow one interpolation sample on each side of the requested support.
        let batch = (((MAX_AUDIO_BLOCK - 2) as u128 * 100) / speed).max(1) as usize;
        for destination in output.chunks_mut(batch) {
            let first = position / 100;
            let last = position + (destination.len() - 1) as u128 * speed;
            let len = (last.div_ceil(100) - first + 1) as usize;
            self.mixer.render(u64::try_from(first).map_err(|_| "Nested audio position overflow")?, &mut scratch[..len])?;
            for sample in destination {
                let at = (position / 100 - first) as usize;
                let fraction = (position % 100) as f32 / 100.0;
                for channel in 0..2 {
                    let current = scratch[at][channel];
                    let next = if fraction == 0.0 { current } else { scratch[at + 1][channel] };
                    sample[channel] += current + (next - current) * fraction;
                }
                position += speed;
            }
        }
        Ok(())
    }
}
