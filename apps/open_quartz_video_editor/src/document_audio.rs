use super::*;
use std::collections::HashSet;

pub const SAMPLE_RATE: u64 = 48_000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AudioAsset {
    pub stream: usize,
    pub time_base: Rational,
    pub start_pts: i64,
    pub duration_pts: Option<i64>,
    pub rate: u32,
    pub channels: u16,
    pub codec: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AudioTrack {
    pub id: String,
    pub name: String,
    pub muted: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub solo: bool,
    pub locked: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub sync_ripple: bool,
    pub gain: f32,
    pub clips: Vec<AudioClip>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) crossfades: Vec<AudioCrossfade>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct AudioCrossfade {
    outgoing: String,
    incoming: String,
    samples: u64,
}

pub struct AudioBlend<'a> {
    pub outgoing: &'a str,
    pub incoming: &'a str,
    pub start: u64,
    pub end: u64,
}

impl AudioTrack {
    pub fn crossfade_samples(&self, clip: &str) -> u64 {
        self.crossfades.iter().find(|fade| fade.outgoing == clip)
            .map_or(0, |fade| fade.samples)
    }

    pub fn blends(&self, data: &ProjectData) -> Result<Vec<AudioBlend<'_>>, String> {
        let mut blends = Vec::with_capacity(self.crossfades.len());
        for fade in &self.crossfades {
            let input = |id: &str| {
                let clip = self.clips.iter().find(|clip| clip.id == id)
                    .ok_or_else(|| format!("Remove the crossfade on {} before deleting its clip", self.name))?;
                let asset = data.assets.iter().find(|asset| asset.id == clip.asset_id)
                    .and_then(|asset| asset.audio.as_ref()).ok_or("Crossfade audio source is missing")?;
                Ok::<_, String>((clip.interval(data)?, asset))
            };
            let (left, left_asset) = input(&fade.outgoing)?;
            let (right, right_asset) = input(&fade.incoming)?;
            if left.end != right.start || left.start >= right.start || fade.samples < 2 {
                return Err(format!("Crossfade on {} requires two adjoining audio clips", self.name));
            }
            if left.speed_percent != normal_speed() || right.speed_percent != normal_speed() {
                return Err("Audio crossfades require clips at 100% speed".into());
            }
            let start = left.end.checked_sub(fade.samples / 2).filter(|start| *start >= left.start)
                .ok_or("Crossfade is longer than the outgoing clip")?;
            let end = start.checked_add(fade.samples).filter(|end| *end <= right.end)
                .ok_or("Crossfade is longer than the incoming clip")?;
            let right_source_start = source_sample(right_asset.start_pts, 0, 1, right_asset.time_base)?;
            if i128::from(right.source) - i128::from(right.start - start) < i128::from(right_source_start) {
                return Err("Crossfade needs more source audio before the incoming clip".into());
            }
            if let Some(duration) = left_asset.duration_pts {
                let pts = left_asset.start_pts.checked_add(duration).ok_or("Audio source duration overflow")?;
                let left_source_end = source_sample(pts, 0, 1, left_asset.time_base)?;
                if i128::from(left.source) + i128::from(end - left.start) > i128::from(left_source_end) {
                    return Err("Crossfade needs more source audio after the outgoing clip".into());
                }
            }
            blends.push(AudioBlend { outgoing: &fade.outgoing, incoming: &fade.incoming, start, end });
        }
        blends.sort_by_key(|blend| blend.start);
        if blends.windows(2).any(|pair| pair[0].end > pair[1].start) {
            return Err(format!("Audio crossfades overlap on {}", self.name));
        }
        Ok(blends)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AudioClip {
    pub id: String,
    pub asset_id: String,
    pub gain: f32,
    #[serde(default, skip_serializing_if = "zero_samples")]
    pub fade_in: u64,
    #[serde(default, skip_serializing_if = "zero_samples")]
    pub fade_out: u64,
    #[serde(default = "normal_speed", skip_serializing_if = "is_normal_speed")]
    pub speed_percent: u16,
    pub placement: AudioPlacement,
}

fn zero_samples(value: &u64) -> bool {
    *value == 0
}

fn limit_fades(clip: &mut AudioClip, duration: u64) {
    clip.fade_in = clip.fade_in.min(duration);
    clip.fade_out = clip.fade_out.min(duration);
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case", tag = "kind", deny_unknown_fields)]
pub enum AudioPlacement {
    Linked {
        video_id: String,
    },
    Independent {
        start: u64,
        duration: u64,
        source: i64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioInterval {
    pub start: u64,
    pub end: u64,
    pub source: i64,
    pub speed_percent: u16,
}

impl Rational {
    pub fn frame_to_sample(self, frame: u64) -> Result<u64, String> {
        Rational::new(self.numerator, self.denominator)?;
        let time =
            u128::from(frame) * u128::from(self.denominator as u32) * u128::from(SAMPLE_RATE);
        let divisor = self.numerator as u128;
        u64::try_from((time + divisor / 2) / divisor)
            .map_err(|_| "Sequence audio sample overflow".into())
    }

    pub fn sample_to_frame_ceil(self, sample: u64) -> Result<u64, String> {
        Rational::new(self.numerator, self.denominator)?;
        let numerator = u128::from(sample) * self.numerator as u128;
        let denominator = self.denominator as u128 * u128::from(SAMPLE_RATE);
        let frame = u64::try_from(numerator / denominator)
            .map_err(|_| "Audio timeline duration overflow")?;
        if self.frame_to_sample(frame)? >= sample {
            Ok(frame)
        } else {
            frame
                .checked_add(1)
                .ok_or_else(|| "Audio timeline duration overflow".into())
        }
    }
}

fn source_sample(pts: i64, subtick: u64, divisor: u64, base: Rational) -> Result<i64, String> {
    if divisor == 0 {
        return Err("Invalid source sub-tick divisor".into());
    }
    let fraction = i128::from(pts)
        .checked_mul(i128::from(divisor))
        .and_then(|value| value.checked_add(i128::from(subtick)))
        .and_then(|value| value.checked_mul(i128::from(base.numerator)))
        .and_then(|value| value.checked_mul(i128::from(SAMPLE_RATE)))
        .ok_or("Audio source sample overflow")?;
    let denominator = i128::from(divisor) * i128::from(base.denominator);
    let magnitude = fraction
        .checked_abs()
        .and_then(|value| value.checked_add(denominator / 2))
        .ok_or("Audio source sample overflow")?;
    i64::try_from(fraction.signum() * (magnitude / denominator))
        .map_err(|_| "Audio source sample overflow".into())
}

impl AudioClip {
    pub fn interval(&self, data: &ProjectData) -> Result<AudioInterval, String> {
        match &self.placement {
            AudioPlacement::Independent {
                start,
                duration,
                source,
            } => Ok(AudioInterval {
                start: *start,
                end: start
                    .checked_add(*duration)
                    .ok_or("Audio timeline range overflow")?,
                source: *source,
                speed_percent: self.speed_percent,
            }),
            AudioPlacement::Linked { video_id } => {
                let (track, index) = data.clip_position(video_id)?;
                let clip = &data.tracks[track].clips[index];
                let asset = data
                    .assets
                    .iter()
                    .find(|asset| asset.id == clip.asset_id && asset.id == self.asset_id)
                    .ok_or("Linked audio and video must reference the same asset")?;
                Ok(AudioInterval {
                    start: data.fps.frame_to_sample(clip.timeline_start)?,
                    end: data.fps.frame_to_sample(clip.timeline_end()?)?,
                    source: source_sample(
                        clip.source_in_pts,
                        clip.source_subtick_numerator,
                        clip.source_subtick_denominator,
                        asset.time_base,
                    )?,
                    speed_percent: clip.speed_percent,
                })
            }
        }
    }
}

fn valid_gain(gain: f32) -> bool {
    (0.0..=2.0).contains(&gain)
}

pub(super) fn validate_audio(data: &ProjectData) -> Result<(), String> {
    let mut tracks = HashSet::new();
    let mut clips = HashSet::new();
    let mut links = HashSet::new();
    for asset in &data.assets {
        if let Some(audio) = &asset.audio {
            Rational::new(audio.time_base.numerator, audio.time_base.denominator)?;
            if !asset.has_audio
                || audio.rate == 0
                || audio.channels == 0
                || audio.duration_pts.is_some_and(|duration| duration <= 0)
            {
                return Err("Invalid audio source metadata".into());
            }
        }
        if !asset.has_visual() && (asset.width != 0 || asset.height != 0 || asset.audio.is_none()) {
            return Err("Asset requires valid visual or audio metadata".into());
        }
    }
    for track in &data.audio_tracks {
        if track.id.is_empty()
            || track.name.trim().is_empty()
            || !tracks.insert(&track.id)
            || !valid_gain(track.gain)
        {
            return Err("Invalid audio track".into());
        }
        let mut intervals = Vec::with_capacity(track.clips.len());
        for clip in &track.clips {
            if clip.id.is_empty()
                || !clips.insert(&clip.id)
                || !valid_gain(clip.gain)
                || !valid_speed(clip.speed_percent)
                || !data
                    .assets
                    .iter()
                    .any(|asset| asset.id == clip.asset_id && asset.audio.is_some())
            {
                return Err("Invalid audio clip or missing source".into());
            }
            if let AudioPlacement::Linked { video_id } = &clip.placement {
                if clip.speed_percent != normal_speed()
                    || clip.fade_in != 0
                    || clip.fade_out != 0
                    || !links.insert(video_id)
                {
                    return Err("Invalid or duplicate linked audio".into());
                }
            }
            let range = clip.interval(data)?;
            if range.end <= range.start
                || i64::try_from(
                    (u128::from(range.end - range.start) * u128::from(range.speed_percent))
                        .div_ceil(100),
                )
                .ok()
                .and_then(|len| range.source.checked_add(len))
                .is_none()
            {
                return Err("Invalid audio clip range".into());
            }
            if clip.fade_in > range.end - range.start || clip.fade_out > range.end - range.start {
                return Err("Audio fade exceeds the clip duration".into());
            }
            data.fps.sample_to_frame_ceil(range.end)?;
            intervals.push(range);
        }
        track.blends(data)?;
        intervals.sort_by_key(|range| range.start);
        if intervals.windows(2).any(|pair| pair[0].end > pair[1].start) {
            return Err(format!("Audio clips overlap on {}", track.name));
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum AudioDestination {
    #[default]
    Automatic,
    VideoOnly,
    Track(String),
}

impl ProjectData {
    fn insert_audio(
        &mut self,
        revision: u64,
        asset_id: &str,
        placement: AudioPlacement,
        destination: &AudioDestination,
    ) -> Result<(), String> {
        let clip = AudioClip {
            id: new_id("audio-clip", revision),
            asset_id: asset_id.into(),
            gain: 1.0,
            fade_in: 0,
            fade_out: 0,
            speed_percent: normal_speed(),
            placement,
        };
        let range = clip.interval(self)?;
        let can_place = |track: &AudioTrack| {
            !track.locked && track.clips.iter().all(|other| {
                other.interval(self)
                    .is_ok_and(|other| range.end <= other.start || range.start >= other.end)
            })
        };
        let target = if let AudioDestination::Track(id) = destination {
            let index = self.audio_tracks.iter().position(|track| track.id == *id)
                .ok_or("The target audio track no longer exists")?;
            let track = &self.audio_tracks[index];
            if !can_place(track) {
                return Err(format!("Cannot place audio on {} because it is locked or overlaps another clip", track.name));
            }
            Some(index)
        } else {
            self.audio_tracks.iter().position(can_place)
        };
        let index = match target {
            Some(index) => index,
            None => {
                self.audio_tracks.push(AudioTrack {
                    id: new_id("audio-track", revision),
                    name: format!("Audio {}", self.audio_tracks.len() + 1),
                    muted: false,
                    solo: false,
                    locked: false,
                    sync_ripple: false,
                    gain: 1.0,
                    clips: Vec::new(),
                    crossfades: Vec::new(),
                });
                self.audio_tracks.len() - 1
            }
        };
        self.audio_tracks[index].clips.push(clip);
        Ok(())
    }

    pub(super) fn link_audio(
        &mut self,
        revision: u64,
        asset_id: &str,
        video_id: String,
        destination: &AudioDestination,
    ) -> Result<(), String> {
        self.insert_audio(revision, asset_id, AudioPlacement::Linked { video_id }, destination)
    }

    pub(super) fn split_link(&mut self, revision: u64, original: &str, right: String) {
        for track in &mut self.audio_tracks {
            if let Some(mut clip) = track.clips.iter().find(|clip| matches!(&clip.placement, AudioPlacement::Linked { video_id } if video_id == original)).cloned() {
                clip.id = new_id("audio-clip", revision);
                clip.placement = AudioPlacement::Linked { video_id: right };
                track.clips.push(clip);
                return;
            }
        }
    }
}

pub(super) fn validate_locked_links(old: &ProjectData, new: &ProjectData) -> Result<(), String> {
    for track in old.audio_tracks.iter().filter(|track| track.locked) {
        for clip in &track.clips {
            if matches!(clip.placement, AudioPlacement::Linked { .. })
                && clip.interval(old)?
                    != clip
                        .interval(new)
                        .map_err(|_| format!("{} is locked", track.name))?
            {
                return Err(format!("{} is locked", track.name));
            }
        }
    }
    Ok(())
}

impl ProjectData {
    pub(super) fn ripple_independent_audio(
        &mut self, frames: Range<u64>, edit: RippleEdit, revision: u64,
    ) -> Result<(), String> {
        let from = self.fps.frame_to_sample(frames.start)?;
        let to = self.fps.frame_to_sample(frames.end)?;
        let duration = to - from;
        let cut = if edit == RippleEdit::Insert { from } else { to };
        for track in self.audio_tracks.iter_mut().filter(|track| track.sync_ripple) {
            let original = std::mem::take(&mut track.clips);
            let mut edited = Vec::with_capacity(original.len());
            for clip in original {
                let AudioPlacement::Independent { start, duration: length, source } = clip.placement else {
                    edited.push(clip);
                    continue;
                };
                let end = start.checked_add(length).ok_or("Audio timeline range overflow")?;
                if end <= from {
                    edited.push(clip);
                    continue;
                }
                if track.locked {
                    return Err(format!("{} is locked", track.name));
                }
                if start < from {
                    let mut left = clip.clone();
                    left.placement = AudioPlacement::Independent { start, duration: from - start, source };
                    left.fade_out = 0;
                    limit_fades(&mut left, from - start);
                    edited.push(left);
                }
                if end > cut {
                    let right_start = start.max(cut);
                    let right_length = end - right_start;
                    let position = if edit == RippleEdit::Insert {
                        right_start.checked_add(duration).ok_or("Audio timeline range overflow")?
                    } else {
                        right_start - duration
                    };
                    let mut right = clip;
                    if start < from {
                        right.id = new_id("audio-clip", revision);
                    }
                    if right_start != start {
                        right.fade_in = 0;
                    }
                    limit_fades(&mut right, right_length);
                    right.placement = AudioPlacement::Independent {
                        start: position,
                        duration: right_length,
                        source: trim_audio_source(source, start, right_start, right.speed_percent)?,
                    };
                    edited.push(right);
                }
            }
            track.clips = edited;
        }
        Ok(())
    }

    fn writable_audio(&self, id: &str) -> Result<(usize, usize), String> {
        for (lane, track) in self.audio_tracks.iter().enumerate() {
            if let Some(index) = track.clips.iter().position(|clip| clip.id == id) {
                if track.locked {
                    return Err(format!("{} is locked", track.name));
                }
                return Ok((lane, index));
            }
        }
        Err("Audio clip is missing".into())
    }
}

fn trim_audio_source(source: i64, previous: u64, start: u64, speed: u16) -> Result<i64, String> {
    let delta = (i128::from(start) - i128::from(previous)) * i128::from(speed);
    i64::try_from(i128::from(source) + (delta + 50).div_euclid(100))
        .map_err(|_| "Audio source range overflow".into())
}

impl EditorDocument {
    pub fn edit_audio_clip(&mut self, id: &str, action: &str, value: &str) -> Result<(), String> {
        if action == "crossfade" {
            let samples = value.trim().parse().map_err(|_| "Enter an audio crossfade length in samples")?;
            return self.set_audio_crossfade(id, samples);
        }
        self.transact(|data| {
            let (lane, index) = data.writable_audio(id)?;
            let interval = data.audio_tracks[lane].clips[index].interval(data)?;
            let clips = &mut data.audio_tracks[lane].clips;
            let clip = &mut clips[index];
            match action {
                "unlink" => {
                    clip.speed_percent = interval.speed_percent;
                    clip.placement = AudioPlacement::Independent {
                        start: interval.start,
                        duration: interval.end - interval.start,
                        source: interval.source,
                    }
                }
                "gain" => {
                    clip.gain = value
                        .trim()
                        .parse::<f32>()
                        .map_err(|_| "Enter a gain percentage from 0 to 200")?
                        / 100.0;
                    if !valid_gain(clip.gain) {
                        return Err("Enter a gain percentage from 0 to 200".into());
                    }
                }
                "delete" if matches!(clip.placement, AudioPlacement::Independent { .. }) => {
                    clips.remove(index);
                }
                "fade-in" | "fade-out" => {
                    if matches!(clip.placement, AudioPlacement::Linked { .. }) {
                        return Err("Unlink audio to edit its fades".into());
                    }
                    let samples = value
                        .trim()
                        .parse()
                        .map_err(|_| "Enter a non-negative fade length in samples")?;
                    if action == "fade-in" {
                        clip.fade_in = samples;
                    } else {
                        clip.fade_out = samples;
                    }
                }
                "start" | "end" | "source" => {
                    let AudioPlacement::Independent {
                        start,
                        duration,
                        source,
                    } = &mut clip.placement
                    else {
                        return Err("Unlink audio to edit it independently".into());
                    };
                    match action {
                        "start" => {
                            *start = value
                                .trim()
                                .parse()
                                .map_err(|_| "Enter a non-negative sequence sample")?
                        }
                        "end" => {
                            let end: u64 = value
                                .trim()
                                .parse()
                                .map_err(|_| "Enter a non-negative sequence sample")?;
                            *duration = end.checked_sub(*start).ok_or("End must follow start")?;
                            let length = *duration;
                            limit_fades(clip, length);
                        }
                        _ => {
                            *source = value
                                .trim()
                                .parse()
                                .map_err(|_| "Enter a source sample position")?
                        }
                    }
                }
                _ => return Err("Unknown audio clip action".into()),
            }
            Ok(())
        })
    }

    pub fn audio_gesture(
        &mut self,
        id: &str,
        kind: &str,
        delta: i32,
        revision: u64,
    ) -> Result<(), String> {
        self.transact_at(revision, |data| {
            let (lane, index) = data.writable_audio(id)?;
            let amount = data.fps.frame_to_sample(u64::from(delta.unsigned_abs()))?;
            let shift = |value: u64| {
                if delta < 0 {
                    value.saturating_sub(amount)
                } else {
                    value.saturating_add(amount)
                }
            };
            let clip = &mut data.audio_tracks[lane].clips[index];
            let AudioPlacement::Independent {
                start,
                duration,
                source,
            } = &mut clip.placement
            else {
                return Err("Unlink audio to edit it independently".into());
            };
            let end = start
                .checked_add(*duration)
                .ok_or("Audio timeline range overflow")?;
            match kind {
                "fade-in" => clip.fade_in = shift(clip.fade_in).min(*duration),
                "fade-out" => {
                    clip.fade_out = if delta < 0 {
                        clip.fade_out.saturating_add(amount).min(*duration)
                    } else {
                        clip.fade_out.saturating_sub(amount)
                    };
                }
                "move" => *start = shift(*start),
                "trim-start" => {
                    let position = shift(*start);
                    *duration = end.checked_sub(position).ok_or("End must follow start")?;
                    *source = trim_audio_source(*source, *start, position, clip.speed_percent)?;
                    *start = position;
                }
                "trim-end" => {
                    *duration = shift(end)
                        .checked_sub(*start)
                        .ok_or("End must follow start")?
                }
                _ => return Err("Unknown audio gesture".into()),
            }
            let length = *duration;
            limit_fades(clip, length);
            Ok(())
        })
    }

    pub fn split_audio(&mut self, id: &str, frame: u64) -> Result<String, String> {
        let right_id = new_id("audio-clip", self.revision);
        self.transact(|data| {
            let (lane, index) = data.writable_audio(id)?;
            let split = data.fps.frame_to_sample(frame)?;
            let clips = &mut data.audio_tracks[lane].clips;
            let left = &mut clips[index];
            let mut right = left.clone();
            let AudioPlacement::Independent {
                start,
                duration,
                source,
            } = &mut left.placement
            else {
                return Err("Unlink audio to edit it independently".into());
            };
            let end = start
                .checked_add(*duration)
                .ok_or("Audio timeline range overflow")?;
            if split <= *start || split >= end {
                return Err("Split point must be inside the selected audio clip".into());
            }
            right.id = right_id.clone();
            right.placement = AudioPlacement::Independent {
                start: split,
                duration: end - split,
                source: trim_audio_source(*source, *start, split, left.speed_percent)?,
            };
            *duration = split - *start;
            let left_length = *duration;
            left.fade_out = 0;
            right.fade_in = 0;
            limit_fades(left, left_length);
            limit_fades(&mut right, end - split);
            clips.insert(index + 1, right);
            Ok(())
        })?;
        Ok(right_id)
    }

    pub fn place_audio(&mut self, asset_id: &str, frame: u64) -> Result<(), String> {
        let audio = self
            .asset(asset_id)
            .and_then(|asset| asset.audio.as_ref())
            .ok_or("Asset has no supported audio stream")?;
        let source = source_sample(audio.start_pts, 0, 1, audio.time_base)?;
        let duration = audio
            .duration_pts
            .ok_or("Audio placement requires a known duration")?;
        let end_pts = audio
            .start_pts
            .checked_add(duration)
            .ok_or("Audio source duration overflow")?;
        let end = source_sample(end_pts, 0, 1, audio.time_base)?;
        let duration = end
            .checked_sub(source)
            .and_then(|duration| u64::try_from(duration).ok())
            .ok_or("Audio source duration overflow")?;
        let start = self.data.fps.frame_to_sample(frame)?;
        let revision = self.revision;
        self.transact(|data| {
            data.insert_audio(
                revision,
                asset_id,
                AudioPlacement::Independent {
                    start,
                    duration,
                    source,
                },
                &AudioDestination::Automatic,
            )
        })
    }

    fn set_audio_crossfade(&mut self, id: &str, samples: u64) -> Result<(), String> {
        self.transact(|data| {
            let (lane, index) = data.writable_audio(id)?;
            let track = &data.audio_tracks[lane];
            let incoming = if samples != 0 {
                let cut = track.clips[index].interval(data)?.end;
                Some(track.clips.iter().find(|clip| clip.id != id
                    && clip.interval(data).is_ok_and(|interval| interval.start == cut))
                    .ok_or("Crossfade requires an adjoining audio clip on the same track")?.id.clone())
            } else {
                None
            };
            let fades = &mut data.audio_tracks[lane].crossfades;
            fades.retain(|fade| fade.outgoing != id);
            if let Some(incoming) = incoming {
                fades.push(AudioCrossfade { outgoing: id.into(), incoming, samples });
            }
            Ok(())
        })
    }

    pub fn audio_track(&mut self, id: &str, action: &str, value: &str) -> Result<(), String> {
        self.transact(|data| {
            let track = data
                .audio_tracks
                .iter_mut()
                .find(|track| track.id == id)
                .ok_or("Audio track is missing")?;
            match action {
                "mute" => track.muted = !track.muted,
                "solo" => track.solo = !track.solo,
                "lock" => track.locked = !track.locked,
                "sync-ripple" => track.sync_ripple = !track.sync_ripple,
                "gain" => {
                    track.gain = value
                        .trim()
                        .parse::<f32>()
                        .map_err(|_| "Enter a gain percentage from 0 to 200")?
                        / 100.0;
                    if !valid_gain(track.gain) {
                        return Err("Enter a gain percentage from 0 to 200".into());
                    }
                }
                _ => return Err("Unknown audio track action".into()),
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synced_audio_uses_sample_boundaries_and_preserves_retimed_source_positions() {
        let mut project = ProjectData::default();
        project.fps = Rational::new(30_000, 1001).unwrap();
        project.audio_tracks.push(AudioTrack {
            id: "sync-audio".into(),
            name: "Audio".into(),
            muted: false,
            solo: false,
            locked: false,
            sync_ripple: true,
            gain: 1.0,
            clips: vec![AudioClip {
                id: "sync-input".into(),
                asset_id: "source".into(),
                gain: 1.0,
                fade_in: 100,
                fade_out: 200,
                speed_percent: 150,
                placement: AudioPlacement::Independent { start: 1, duration: 10_000, source: 20 },
            }],
            crossfades: Vec::new(),
        });
        project.ripple_independent_audio(2..3, RippleEdit::Insert, 0).unwrap();
        let lane = &project.audio_tracks[0];
        let [left, right] = lane.clips.as_slice() else { panic!("Expected two audio fragments"); };
        assert_eq!(left.placement, AudioPlacement::Independent { start: 1, duration: 3202, source: 20 });
        assert_eq!((left.fade_in, left.fade_out), (100, 0));
        assert_eq!(right.placement, AudioPlacement::Independent { start: 4805, duration: 6798, source: 4823 });
        assert_eq!((right.fade_in, right.fade_out), (0, 200));
        project.ripple_independent_audio(2..3, RippleEdit::Remove, 1).unwrap();
        assert_eq!(project.audio_tracks[0].clips[1].placement,
            AudioPlacement::Independent { start: 3203, duration: 6798, source: 4823 });
    }

    #[test]
    fn non_integer_fps_audio_boundaries_are_computed_from_origin() {
        let fps = Rational::new(30_000, 1001).unwrap();
        assert_eq!(fps.frame_to_sample(1).unwrap(), 1602);
        assert_eq!(fps.frame_to_sample(2).unwrap(), 3203);
        assert_eq!(fps.frame_to_sample(30_000).unwrap(), 48_048_000);
        for frame in 1..100 {
            let sample = fps.frame_to_sample(frame).unwrap();
            assert_eq!(fps.sample_to_frame_ceil(sample).unwrap(), frame);
            assert_eq!(fps.sample_to_frame_ceil(sample + 1).unwrap(), frame + 1);
        }
        assert_eq!(
            source_sample(0, 1, 3, Rational::new(1, 30).unwrap()).unwrap(),
            533
        );
    }
}
