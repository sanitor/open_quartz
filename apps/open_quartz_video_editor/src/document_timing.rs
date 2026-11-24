use super::{
    advance_source, new_id, shifted_source_units, AudioPlacement, Clip, EditorDocument,
    ProjectData, Rational,
};
use std::{collections::HashSet, ops::Range};

#[derive(Clone, Copy)]
pub enum Placement {
    EmptySpace(Option<u64>),
    Insert(u64),
    Overwrite(u64),
}

pub const fn normal_speed() -> u16 {
    100
}

pub fn valid_speed(percent: u16) -> bool {
    (25..=400).contains(&percent)
}

impl Clip {
    // A unit is one hundredth of a source frame at the sequence frame rate.
    fn source_span_units(&self) -> i128 {
        i128::from(self.duration) * i128::from(self.speed_percent)
            + i128::from(self.source_span_remainder)
    }

    pub(super) fn resize_right(&mut self, duration: u64) {
        if self.duration != duration {
            self.duration = duration;
            self.source_span_remainder = 0;
        }
    }

    pub fn source_out(
        &self,
        fps: Rational,
        time_base: Rational,
    ) -> Result<(i64, u64, u64), String> {
        shifted_source_units(self, self.source_span_units(), fps, time_base)
    }

    pub fn frames_until_source_out(
        &self,
        out: i64,
        fps: Rational,
        time_base: Rational,
    ) -> Result<u64, String> {
        if out == self.source_out(fps, time_base)?.0 {
            return Ok(self.duration);
        }
        let fraction = i128::from(self.source_subtick_denominator);
        let ticks = (i128::from(out) - i128::from(self.source_in_pts))
            .checked_mul(fraction)
            .and_then(|ticks| ticks.checked_sub(i128::from(self.source_subtick_numerator)))
            .ok_or("Source out overflow")?;
        if ticks <= 0 {
            return Err("Source out must follow source in".into());
        }
        let numerator = ticks
            .checked_mul(i128::from(time_base.numerator))
            .and_then(|value| value.checked_mul(i128::from(fps.numerator) * 100))
            .ok_or("Source out overflow")?;
        let denominator = fraction
            .checked_mul(
                i128::from(time_base.denominator)
                    * i128::from(fps.denominator)
                    * i128::from(self.speed_percent),
            )
            .ok_or("Source out overflow")?;
        u64::try_from(numerator / denominator).map_err(|_| "Source out overflow".into())
    }
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
        if clip.title.is_none() {
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
        }
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
        left.resize_right(at - left.timeline_start);
        self.split_link(revision, &original_id, right_id.clone());
        if let Some(item) = self.tracks[track].dissolves.iter_mut().find(|item| item.outgoing == original_id) {
            item.outgoing = right_id.clone();
        }
        Ok(right_id)
    }

    pub(super) fn remove_video(&mut self, track: usize, indices: Range<usize>) {
        let removed: HashSet<_> = self.tracks[track]
            .clips
            .drain(indices)
            .map(|clip| clip.id)
            .collect();
        self.tracks[track].dissolves.retain(|item| !removed.contains(&item.outgoing) && !removed.contains(&item.incoming));
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
    pub fn set_speed(&mut self, clip_id: &str, percent: u16) -> Result<(), String> {
        if !valid_speed(percent) {
            return Err("Speed must be a whole percentage from 25 to 400".into());
        }
        self.transact(|data| {
            let (track, index) = data.writable_clip(clip_id)?;
            let clip = &mut data.tracks[track].clips[index];
            let span = clip.source_span_units();
            let rate = i128::from(percent);
            let duration =
                u64::try_from(span / rate).map_err(|_| "Retimed clip exceeds timeline range")?;
            if duration == 0 {
                return Err("This speed would leave less than one sequence frame".into());
            }
            clip.duration = duration;
            clip.speed_percent = percent;
            clip.source_span_remainder = (span % rate) as u16;
            Ok(())
        })
    }

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
            data.tracks[track].clips[index].resize_right(frame - start);
            Ok(())
        })
    }
}
