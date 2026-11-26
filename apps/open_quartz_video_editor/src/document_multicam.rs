use super::*;

pub const MAX_ANGLES: usize = 4;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MulticamAngle {
    pub asset_id: String,
    sync_ns: u64,
}

impl ProjectData {
    // Find the start on the target angle in units of the current sequence clock.
    fn aligned_start(&self, clip: &Clip, angle: &MulticamAngle) -> Result<i128, String> {
        let current = self
            .multicam
            .iter()
            .find(|angle| angle.asset_id == clip.asset_id)
            .ok_or("Select a clip from a registered camera angle")?;
        if clip.speed_percent != normal_speed() {
            return Err("Multicam requires 100% clip speed".into());
        }
        let source = i128::from(self.clip_source_time(clip, clip.timeline_start)?);
        let target = source + i128::from(angle.sync_ns) - i128::from(current.sync_ns);
        let numerator = target * i128::from(self.fps.numerator);
        let denominator = i128::from(self.fps.denominator) * 1_000_000_000;
        Ok(numerator.div_euclid(denominator)
            + i128::from(numerator.rem_euclid(denominator) * 2 >= denominator))
    }

    pub fn angle_preview(&self, clip_id: &str) -> Option<Self> {
        let base = self.clips().find(|clip| clip.id == clip_id)?;
        let mut preview = self.clone();
        preview.tracks.clear();
        for (index, angle) in self.multicam.iter().enumerate() {
            let asset = self
                .assets
                .iter()
                .find(|asset| asset.id == angle.asset_id && asset.online)?;
            let aligned = self.aligned_start(base, angle).ok()?;
            let available = i128::from(asset.source_duration_frames(self.fps).ok()?);
            let first = 0.max(-aligned);
            let end = i128::from(base.duration).min(available - aligned);
            if first >= end {
                continue;
            }
            let mut clip = base.clone();
            clip.id = format!("angle-{index}");
            clip.timeline_start += u64::try_from(first).ok()?;
            clip.duration = u64::try_from(end - first).ok()?;
            set_source(&mut clip, asset, aligned + first, self.fps).ok()?;
            clip.pipeline = ClipPipeline::default();
            clip.effect_keys.clear();
            clip.spatial_keys.clear();
            clip.mask = None;
            clip.mask_tracking.clear();
            clip.crop = Crop::default();
            clip.crop_keys.clear();
            clip.opacity_keys.clear();
            clip.transform = ClipTransform {
                scale: 0.5,
                x: if index % 2 == 0 { -0.25 } else { 0.25 },
                y: if index < 2 { -0.25 } else { 0.25 },
                ..ClipTransform::default()
            };
            let mut track = VideoTrack::new(format!("angle-track-{index}"), asset.name.clone());
            track.clips.push(clip);
            preview.tracks.push(track);
        }
        // The monitoring project keeps the clock running across the full program.
        preview.audio_tracks.clear();
        preview.markers.clear();
        (!preview.tracks.is_empty()).then_some(preview)
    }
}

fn set_source(
    clip: &mut Clip,
    asset: &MediaAsset,
    frame: i128,
    fps: Rational,
) -> Result<(), String> {
    if frame < 0
        || frame + i128::from(clip.duration) > i128::from(asset.source_duration_frames(fps)?)
    {
        return Err("The target camera does not cover the selected range".into());
    }
    clip.asset_id = asset.id.clone();
    clip.source_in_pts = asset.start_pts.ok_or("Camera source time is unavailable")?;
    clip.source_subtick_numerator = 0;
    clip.source_subtick_denominator = 1;
    advance_source(clip, frame, fps, asset.time_base)
}

impl EditorDocument {
    pub fn clear_angles(&mut self) -> Result<(), String> {
        self.transact(|data| {
            data.multicam.clear();
            Ok(())
        })
    }

    pub fn register_angle(&mut self, asset_id: &str, frame: u64) -> Result<(), String> {
        let asset = self
            .asset(asset_id)
            .filter(|asset| asset.has_video() && asset.online && asset.sequence_source().is_none())
            .ok_or("Choose online camera footage")?;
        if frame >= asset.source_duration_frames(self.data.fps)? {
            return Err("Choose a sync frame inside the source".into());
        }
        let sync_ns = self.data.fps.frames_to_ns(frame)?;
        self.transact(|data| {
            if let Some(angle) = data
                .multicam
                .iter_mut()
                .find(|angle| angle.asset_id == asset_id)
            {
                angle.sync_ns = sync_ns;
            } else {
                if data.multicam.len() >= MAX_ANGLES {
                    return Err("Multicam supports four angles".into());
                }
                data.multicam.push(MulticamAngle {
                    asset_id: asset_id.into(),
                    sync_ns,
                });
            }
            Ok(())
        })
    }

    pub fn cut_angle(
        &mut self,
        id: &str,
        frame: u64,
        index: usize,
        split: bool,
    ) -> Result<String, String> {
        let revision = self.revision;
        let mut result = id.to_owned();
        self.transact(|data| {
            let (track, position) = data.writable_clip(id)?;
            let clip = &data.tracks[track].clips[position];
            let angle = data.multicam.get(index).ok_or("Unknown camera angle")?;
            if clip.asset_id == angle.asset_id { return Ok(()); }
            let asset = data.assets.iter().find(|asset| asset.id == angle.asset_id && asset.online)
                .cloned().ok_or("Camera source is offline")?;
            let aligned = data.aligned_start(clip, angle)?;
            let start = clip.timeline_start;
            if split && !(start..clip.timeline_end()?).contains(&frame) {
                return Err("Move the playhead inside the selected camera clip".into());
            }
            let mut audio = Vec::new();
            for (lane, track) in data.audio_tracks.iter().enumerate() {
                for (index, item) in track.clips.iter().enumerate() {
                    if matches!(&item.placement, AudioPlacement::Linked { video_id } if video_id == id) {
                        if track.locked { return Err(format!("{} is locked", track.name)); }
                        audio.push((lane, index, item.interval(data)?));
                    }
                }
            }
            for (lane, index, interval) in audio {
                data.audio_tracks[lane].clips[index].placement = AudioPlacement::Independent {
                    start: interval.start, duration: interval.end - interval.start, source: interval.source,
                };
            }
            let offset = if split { frame - start } else { 0 };
            let position = if offset > 0 {
                result = data.split_video(track, position, frame, revision)?;
                position + 1
            } else { position };
            let clip = &mut data.tracks[track].clips[position];
            set_source(clip, &asset, aligned + i128::from(offset), data.fps)?;
            clip.mask_tracking.clear();
            Ok(())
        })?;
        Ok(result)
    }
}
