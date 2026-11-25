use super::*;
use crate::animation_key::Keyframe;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MotionChannel {
    #[default]
    Transform,
    Opacity,
    Crop,
}

impl MotionChannel {
    pub fn from_index(index: i32) -> Option<Self> {
        match index {
            0 => Some(Self::Transform),
            1 => Some(Self::Opacity),
            2 => Some(Self::Crop),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum MotionPoint {
    Transform(Keyframe<i64, Pose>),
    Opacity(Keyframe<u64, f32>),
    Crop(Keyframe<i64, Crop>),
}

impl MotionPoint {
    pub fn time(self) -> i128 {
        match self {
            Self::Transform(key) => key.time.into(),
            Self::Opacity(key) => key.time.into(),
            Self::Crop(key) => key.time.into(),
        }
    }

    pub fn set_time(&mut self, data: &ProjectData, clip: &Clip, frame: u64) -> Result<(), String> {
        let offset = frame.checked_sub(clip.timeline_start).filter(|offset| *offset < clip.duration)
            .ok_or("Keyframes must stay inside the selected clip")?;
        let frame_offset = || i64::try_from(offset).map_err(|_| "Keyframe position exceeds the clip range".to_owned());
        match self {
            Self::Transform(key) => key.time = frame_offset()?,
            Self::Opacity(key) => key.time = data.clip_source_time(clip, frame)?,
            Self::Crop(key) => key.time = frame_offset()?,
        }
        Ok(())
    }

    pub fn frame(self, data: &ProjectData, clip: &Clip) -> Option<u64> {
        if !matches!(self, Self::Opacity(_)) {
            return u64::try_from(self.time()).ok().filter(|offset| *offset < clip.duration)
                .and_then(|offset| clip.timeline_start.checked_add(offset));
        }
        let start = clip.timeline_start;
        let end = clip.timeline_end().ok()?.checked_sub(1)?;
        let stamp = |frame| data.clip_source_time(clip, frame).ok().map(i128::from);
        if self.time() < stamp(start)? || self.time() > stamp(end)? {
            return None;
        }
        let (mut left, mut right) = (start, end);
        while left < right {
            let middle = left + (right - left) / 2;
            if stamp(middle)? < self.time() {
                left = middle + 1;
            } else {
                right = middle;
            }
        }
        let previous = left.saturating_sub(1).max(start);
        if (stamp(previous)? - self.time()).abs() < (stamp(left)? - self.time()).abs() {
            Some(previous)
        } else {
            Some(left)
        }
    }
}

impl Clip {
    pub fn motion_keys(&self, channel: MotionChannel) -> Vec<MotionPoint> {
        match channel {
            MotionChannel::Transform => self.spatial_keys.iter().copied().map(MotionPoint::Transform).collect(),
            MotionChannel::Opacity => self.opacity_keys.iter().copied().map(MotionPoint::Opacity).collect(),
            MotionChannel::Crop => self.crop_keys.iter().copied().map(MotionPoint::Crop).collect(),
        }
    }

    fn replace_motion_keys(&mut self, channel: MotionChannel, points: Vec<MotionPoint>) {
        match channel {
            MotionChannel::Transform => self.spatial_keys = points.into_iter().map(|point| match point {
                MotionPoint::Transform(key) => key, _ => unreachable!("motion edit supplied another channel"),
            }).collect(),
            MotionChannel::Opacity => self.opacity_keys = points.into_iter().map(|point| match point {
                MotionPoint::Opacity(key) => key, _ => unreachable!("motion edit supplied another channel"),
            }).collect(),
            MotionChannel::Crop => self.crop_keys = points.into_iter().map(|point| match point {
                MotionPoint::Crop(key) => key, _ => unreachable!("motion edit supplied another channel"),
            }).collect(),
        }
    }
}

impl EditorDocument {
    pub fn edit_motion_keys(
        &mut self, id: &str, channel: MotionChannel, revision: u64,
        edit: impl FnOnce(&ProjectData, &Clip, &mut Vec<MotionPoint>) -> Result<(), String>,
    ) -> Result<(), String> {
        self.transact_at(revision, |data| {
            let (track, index) = data.writable_clip(id)?;
            let clip = &data.tracks[track].clips[index];
            let mut keys = clip.motion_keys(channel);
            edit(data, clip, &mut keys)?;
            keys.sort_by_key(|key| key.time());
            if keys.windows(2).any(|pair| pair[0].time() == pair[1].time()) {
                return Err("The edit would put two keyframes at the same position".into());
            }
            data.tracks[track].clips[index].replace_motion_keys(channel, keys);
            Ok(())
        })
    }
}
