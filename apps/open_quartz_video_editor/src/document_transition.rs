use super::{shifted_source, Clip, EditorDocument, ProjectData, VideoTrack};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, ops::Range};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Dissolve {
    pub outgoing: String,
    pub incoming: String,
    pub frames: u64,
}

impl Dissolve {
    pub fn range(&self, cut: u64) -> Option<Range<u64>> {
        let before = self.frames / 2;
        Some(cut.checked_sub(before)?..cut.checked_add(self.frames - before)?)
    }
}

pub struct ActiveDissolve<'a> {
    pub outgoing: &'a Clip,
    pub incoming: &'a Clip,
    pub range: Range<u64>,
}

#[derive(Clone, Copy)]
pub enum DissolveSide {
    Outgoing,
    Incoming,
}

pub struct VideoInput<'a> {
    pub track: &'a VideoTrack,
    pub clip: &'a Clip,
    pub dissolve: Option<(DissolveSide, Range<u64>)>,
}

impl VideoTrack {
    pub fn dissolve_duration(&self, outgoing: &str) -> u64 {
        self.dissolves
            .iter()
            .find(|item| item.outgoing == outgoing)
            .map_or(0, |item| item.frames)
    }

    fn dissolve_after(&self, index: usize) -> Option<ActiveDissolve<'_>> {
        let (outgoing, incoming) = (self.clips.get(index)?, self.clips.get(index + 1)?);
        let dissolve = self
            .dissolves
            .iter()
            .find(|item| item.outgoing == outgoing.id && item.incoming == incoming.id)?;
        Some(ActiveDissolve {
            outgoing,
            incoming,
            range: dissolve.range(incoming.timeline_start)?,
        })
    }

    fn dissolve_at(&self, frame: u64, index: usize) -> Option<ActiveDissolve<'_>> {
        self.dissolve_after(index)
            .filter(|item| item.range.contains(&frame))
            .or_else(|| {
                self.dissolve_after(index.checked_sub(1)?)
                    .filter(|item| item.range.contains(&frame))
            })
    }

    fn video_inputs(&self, frame: u64) -> [Option<VideoInput<'_>>; 2] {
        let next = self
            .clips
            .partition_point(|clip| clip.timeline_start <= frame);
        let Some(index) = next.checked_sub(1) else {
            return [None, None];
        };
        let clip = &self.clips[index];
        if let Some(item) = self.dissolve_at(frame, index) {
            return [
                Some(VideoInput {
                    track: self,
                    clip: item.outgoing,
                    dissolve: Some((DissolveSide::Outgoing, item.range.clone())),
                }),
                Some(VideoInput {
                    track: self,
                    clip: item.incoming,
                    dissolve: Some((DissolveSide::Incoming, item.range)),
                }),
            ];
        }
        [
            clip.timeline_end()
                .ok()
                .filter(|end| frame < *end)
                .map(|_| VideoInput {
                    track: self,
                    clip,
                    dissolve: None,
                }),
            None,
        ]
    }

    pub fn next_video_boundary(&self, frame: u64) -> Option<u64> {
        let next = self
            .clips
            .partition_point(|clip| clip.timeline_start <= frame);
        if let Some(index) = next.checked_sub(1) {
            if let Some(item) = self.dissolve_at(frame, index) {
                return Some(item.range.end);
            }
            if let Some(item) = self
                .dissolve_after(index)
                .filter(|item| item.range.start > frame)
            {
                return Some(item.range.start);
            }
            if let Some(end) = self.clips[index]
                .timeline_end()
                .ok()
                .filter(|end| *end > frame)
            {
                return Some(end);
            }
        }
        self.clips.get(next).map(|clip| clip.timeline_start)
    }

    pub fn dissolve_ranges(&self) -> impl Iterator<Item = (&Dissolve, Range<u64>)> {
        self.dissolves.iter().filter_map(|item| {
            let cut = self
                .clips
                .iter()
                .find(|clip| clip.id == item.incoming)?
                .timeline_start;
            Some((item, item.range(cut)?))
        })
    }
}

impl ProjectData {
    pub fn video_inputs(&self, frame: u64) -> impl Iterator<Item = VideoInput<'_>> {
        self.tracks
            .iter()
            .filter(|track| track.visible)
            .flat_map(move |track| track.video_inputs(frame).into_iter().flatten())
    }
}

impl EditorDocument {
    pub fn can_dissolve_end(&self, id: &str) -> bool {
        self.can_roll_end(id) && self.data.clip_position(id).is_ok_and(|(lane, index)| {
            let clips = &self.data.tracks[lane].clips;
            clips[index].title.is_none() && clips[index + 1].title.is_none()
        })
    }

    /// A zero duration removes the dissolve on the selected outgoing clip.
    pub fn set_dissolve(&mut self, id: &str, frames: u64) -> Result<(), String> {
        self.transact(|data| {
            let (lane, index) = data.writable_clip(id)?;
            let track = &mut data.tracks[lane];
            let existing = track.dissolves.iter().position(|item| item.outgoing == id);
            if frames == 0 {
                if let Some(index) = existing {
                    track.dissolves.remove(index);
                }
                return Ok(());
            }
            if let Some(index) = existing {
                track.dissolves[index].frames = frames;
                return Ok(());
            }
            let next = track
                .clips
                .get(index + 1)
                .ok_or("A dissolve needs a following clip")?;
            track.dissolves.push(Dissolve {
                outgoing: id.into(),
                incoming: next.id.clone(),
                frames,
            });
            Ok(())
        })
    }
}

pub(super) fn validate_dissolves(data: &ProjectData, track: &VideoTrack) -> Result<(), String> {
    if track.dissolves.is_empty() {
        return Ok(());
    }
    let indices: HashMap<_, _> = track
        .clips
        .iter()
        .enumerate()
        .map(|(index, clip)| (clip.id.as_str(), index))
        .collect();
    let mut ranges = Vec::new();
    for item in &track.dissolves {
        let index = *indices
            .get(item.outgoing.as_str())
            .ok_or("Dissolve needs its outgoing clip")?;
        let left = &track.clips[index];
        let right = track
            .clips
            .get(index + 1)
            .filter(|clip| {
                clip.id == item.incoming && left.timeline_end() == Ok(clip.timeline_start)
            })
            .ok_or("A dissolve needs adjacent clips on the same track")?;
        if left.title.is_some() || right.title.is_some() {
            return Err("A dissolve needs two video clips".into());
        }
        if item.frames < 2 {
            return Err("Dissolve must span at least two frames".into());
        }
        let range = item
            .range(right.timeline_start)
            .filter(|range| {
                range.start >= left.timeline_start
                    && right.timeline_end().is_ok_and(|end| range.end <= end)
            })
            .ok_or("Dissolve exceeds the visible duration of the adjacent clips")?;
        if data.fps.frames_to_ns(range.start)? == data.fps.frames_to_ns(range.end)? {
            return Err("Dissolve time range is too short".into());
        }
        let step = |clip: &Clip, frame: u64| {
            let asset = data
                .assets
                .iter()
                .find(|asset| asset.id == clip.asset_id)
                .ok_or("Clip media is missing")?;
            let offset = i128::from(frame) - i128::from(clip.timeline_start);
            Ok::<_, String>((
                asset,
                shifted_source(clip, offset, data.fps, asset.time_base)?,
            ))
        };
        let (asset, (head, _, _)) = step(right, range.start)?;
        if head < asset.start_pts.ok_or("Unknown incoming source origin")? {
            return Err("Not enough source before the incoming clip for this dissolve".into());
        }
        let (asset, (tail, fraction, _)) = step(left, range.end)?;
        let media_end = asset
            .start_pts
            .and_then(|start| start.checked_add(asset.duration_pts?))
            .ok_or("Unknown outgoing source end")?;
        if tail > media_end || (tail == media_end && fraction != 0) {
            return Err("Not enough source after the outgoing clip for this dissolve".into());
        }
        ranges.push(range);
    }
    ranges.sort_by_key(|range| range.start);
    if ranges.windows(2).any(|pair| pair[0].end > pair[1].start) {
        return Err("Dissolves cannot overlap on the same track".into());
    }
    Ok(())
}
