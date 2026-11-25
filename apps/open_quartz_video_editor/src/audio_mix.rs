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

pub struct AudioMix {
    clips: Vec<MixClip>,
    readers: HashMap<usize, RateReader>,
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
        if !project
            .audio_tracks
            .iter()
            .any(|track| !track.clips.is_empty())
        {
            return Ok(None);
        }
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
            cancel,
            scratch: vec![[0.0; 2]; MAX_AUDIO_BLOCK],
            track_output: vec![[0.0; 2]; MAX_AUDIO_BLOCK],
            track_peaks: vec![0.0; project.audio_tracks.len()],
            output_peak: 0.0,
        }))
    }

    pub fn sources(&self) -> impl Iterator<Item = &PathBuf> {
        self.clips
            .iter()
            .map(|clip| &clip.path)
            .collect::<BTreeSet<_>>()
            .into_iter()
    }

    pub fn render(&mut self, start: u64, output: &mut [StereoSample]) -> Result<(), MediaError> {
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
        self.output_peak = 0.0;
        for sample in output.iter_mut().flatten() {
            self.output_peak = self.output_peak.max(sample.abs());
            *sample = sample.clamp(-1.0, 1.0);
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
