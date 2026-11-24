use super::*;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Marker {
    pub id: String,
    pub frame: u64,
    pub name: String,
}

pub(super) fn validate_markers(markers: &[Marker]) -> Result<(), String> {
    let mut ids = BTreeSet::new();
    let mut frames = BTreeSet::new();
    for marker in markers {
        if marker.id.is_empty()
            || marker.frame > i32::MAX as u64
            || marker.name.trim().is_empty()
            || marker.name.chars().count() > 64
            || !ids.insert(&marker.id)
            || !frames.insert(marker.frame)
        {
            return Err("Invalid or duplicate sequence marker".into());
        }
    }
    Ok(())
}

impl EditorDocument {
    pub fn move_selected(&mut self, ids: &BTreeSet<String>, delta: i32) -> Result<(), String> {
        self.transact(|data| {
            let mut earliest = u64::MAX;
            for id in ids {
                let (lane, index) = data.writable_clip(id)?;
                earliest = earliest.min(data.tracks[lane].clips[index].timeline_start);
            }
            let delta = if delta < 0 {
                i64::from(delta).max(-(earliest.min(i64::MAX as u64) as i64))
            } else {
                i64::from(delta)
            };
            for track in &mut data.tracks {
                for clip in &mut track.clips {
                    if ids.contains(&clip.id) {
                        clip.timeline_start = clip
                            .timeline_start
                            .checked_add_signed(delta)
                            .ok_or("Selection exceeds the sequence range")?;
                    }
                }
                track.clips.sort_by_key(|clip| clip.timeline_start);
            }
            Ok(())
        })
    }

    pub fn lift_selected(&mut self, ids: &BTreeSet<String>) -> Result<(), String> {
        self.transact(|data| {
            let mut positions = ids
                .iter()
                .map(|id| data.writable_clip(id))
                .collect::<Result<Vec<_>, _>>()?;
            positions.sort_unstable_by(|a, b| b.cmp(a));
            for (lane, index) in positions {
                data.remove_video(lane, index..index + 1);
            }
            Ok(())
        })
    }

    pub fn slip_clip(&mut self, id: &str, delta: i32) -> Result<(), String> {
        self.transact(|data| {
            let (lane, index) = data.writable_clip(id)?;
            let clip = &mut data.tracks[lane].clips[index];
            let asset = data
                .assets
                .iter()
                .find(|asset| asset.id == clip.asset_id && asset.has_video())
                .ok_or("Slip requires a video source")?;
            advance_source(clip, i128::from(delta), data.fps, asset.time_base)
        })
    }

    pub fn slide_clip(&mut self, id: &str, delta: i32) -> Result<(), String> {
        self.transact(|data| {
            let (lane, index) = data.writable_clip(id)?;
            let previous = index
                .checked_sub(1)
                .ok_or("Slide requires two adjacent clips")?;
            let clips = &data.tracks[lane].clips;
            let (left, middle, right) = (
                &clips[previous],
                &clips[index],
                clips
                    .get(index + 1)
                    .ok_or("Slide requires two adjacent clips")?,
            );
            let middle_end = middle.timeline_end()?;
            if left.timeline_end()? != middle.timeline_start || right.timeline_start != middle_end {
                return Err("Slide requires two adjacent clips".into());
            }
            let shift = |frame: u64| {
                frame
                    .checked_add_signed(i64::from(delta))
                    .ok_or("Slide exceeds the sequence range")
            };
            let start = shift(middle.timeline_start)?;
            let end = shift(middle_end)?;
            let left_start = left.timeline_start;
            if start <= left_start || end >= right.timeline_end()? {
                return Err("Slide must leave at least one frame in both neighbors".into());
            }
            data.trim_video_start(lane, index + 1, end)?;
            let clips = &mut data.tracks[lane].clips;
            clips[previous].resize_right(start - left_start);
            clips[index].timeline_start = start;
            Ok(())
        })
    }

    pub fn set_marker(&mut self, frame: u64, action: &str, value: &str) -> Result<(), String> {
        let id = new_id("marker", self.revision);
        self.transact(|data| {
            let existing = data.markers.iter().position(|marker| marker.frame == frame);
            match (action, existing) {
                ("toggle", Some(index)) => {
                    data.markers.remove(index);
                }
                ("toggle", None) if frame < data.duration_frames() => {
                    data.markers.push(Marker {
                        id,
                        frame,
                        name: format!("Marker {}", frame),
                    });
                    data.markers.sort_by_key(|marker| marker.frame);
                }
                ("name", Some(index)) => data.markers[index].name = value.trim().to_owned(),
                _ => return Err("Select an existing location on the sequence".into()),
            }
            Ok(())
        })
    }
}
