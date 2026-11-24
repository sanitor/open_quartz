use super::{advance_source, new_id, AudioPlacement, EditorDocument, ProjectData};
use std::{collections::HashSet, ops::Range};

#[derive(Clone, Copy)]
pub enum Placement {
    EmptySpace(Option<u64>),
    Insert(u64),
    Overwrite(u64),
}

impl ProjectData {
    pub(super) fn trim_video_start(
        &mut self,
        track: usize,
        index: usize,
        start: u64,
    ) -> Result<(), String> {
        let clip = &mut self.tracks[track].clips[index];
        let end = clip.timeline_end()?;
        if start >= end {
            return Err("Left trim must leave at least one frame".into());
        }
        let time_base = self
            .assets
            .iter()
            .find(|asset| asset.id == clip.asset_id)
            .ok_or("Clip media is missing")?
            .time_base;
        advance_source(
            clip,
            i128::from(start) - i128::from(clip.timeline_start),
            self.fps,
            time_base,
        )?;
        clip.timeline_start = start;
        clip.duration = end - start;
        Ok(())
    }

    pub(super) fn split_video(
        &mut self,
        track: usize,
        index: usize,
        at: u64,
        revision: u64,
    ) -> Result<String, String> {
        let clips = &mut self.tracks[track].clips;
        let mut right = clips[index].clone();
        if at <= right.timeline_start || at >= right.timeline_end()? {
            return Err("Split point must be inside the selected clip".into());
        }
        let original_id = right.id.clone();
        right.id = new_id("clip", revision);
        let right_id = right.id.clone();
        clips.insert(index + 1, right);
        self.trim_video_start(track, index + 1, at)?;
        let left = &mut self.tracks[track].clips[index];
        left.duration = at - left.timeline_start;
        self.split_link(revision, &original_id, right_id.clone());
        Ok(right_id)
    }

    pub(super) fn remove_video(&mut self, track: usize, indices: Range<usize>) {
        let removed: HashSet<_> = self.tracks[track]
            .clips
            .drain(indices)
            .map(|clip| clip.id)
            .collect();
        for lane in &mut self.audio_tracks {
            lane.clips.retain(|clip| {
                !matches!(&clip.placement,
                AudioPlacement::Linked { video_id } if removed.contains(video_id))
            });
        }
    }

    fn cut_video_at(&mut self, track: usize, frame: u64, revision: u64) -> Result<(), String> {
        let clips = &self.tracks[track].clips;
        let right = clips.partition_point(|clip| clip.timeline_start < frame);
        if let Some(index) = right.checked_sub(1) {
            if clips[index].timeline_end()? > frame {
                self.split_video(track, index, frame, revision)?;
            }
        }
        Ok(())
    }

    pub(super) fn insert_space(
        &mut self,
        track: usize,
        frame: u64,
        duration: u64,
        revision: u64,
    ) -> Result<(), String> {
        self.cut_video_at(track, frame, revision)?;
        for clip in self.tracks[track]
            .clips
            .iter_mut()
            .filter(|clip| clip.timeline_start >= frame)
        {
            clip.timeline_start = clip
                .timeline_start
                .checked_add(duration)
                .ok_or("Insert exceeds timeline range")?;
        }
        Ok(())
    }

    pub(super) fn clear_range(
        &mut self,
        track: usize,
        start: u64,
        end: u64,
        revision: u64,
    ) -> Result<(), String> {
        self.cut_video_at(track, start, revision)?;
        self.cut_video_at(track, end, revision)?;
        let clips = &self.tracks[track].clips;
        let first = clips.partition_point(|clip| clip.timeline_start < start);
        let last = clips.partition_point(|clip| clip.timeline_start < end);
        if first < last {
            self.remove_video(track, first..last);
        }
        Ok(())
    }
}

impl EditorDocument {
    pub fn ripple_delete(&mut self, clip_id: &str) -> Result<(), String> {
        self.transact(|data| {
            let (track, index) = data.writable_clip(clip_id)?;
            let duration = data.tracks[track].clips[index].duration;
            data.remove_video(track, index..index + 1);
            // The lane is ordered and non-overlapping, so all following starts
            // are at least one removed duration from the timeline origin.
            for clip in &mut data.tracks[track].clips[index..] {
                clip.timeline_start -= duration;
            }
            Ok(())
        })
    }

    pub fn can_roll_end(&self, clip_id: &str) -> bool {
        let Ok((track, index)) = self.data.writable_clip(clip_id) else {
            return false;
        };
        let clips = &self.data.tracks[track].clips;
        clips
            .get(index + 1)
            .is_some_and(|next| clips[index].timeline_end() == Ok(next.timeline_start))
    }

    pub fn roll_end(&mut self, clip_id: &str, frame: u64) -> Result<(), String> {
        if !self.can_roll_end(clip_id) {
            return Err("Roll requires two adjacent clips on an unlocked video track".into());
        }
        self.transact(|data| {
            let (track, index) = data.writable_clip(clip_id)?;
            let start = data.tracks[track].clips[index].timeline_start;
            let next = &data.tracks[track].clips[index + 1];
            if frame == next.timeline_start {
                return Ok(());
            }
            if frame <= start || frame >= next.timeline_end()? {
                return Err("Roll must leave at least one frame on each side".into());
            }
            data.trim_video_start(track, index + 1, frame)?;
            data.tracks[track].clips[index].duration = frame - start;
            Ok(())
        })
    }
}
