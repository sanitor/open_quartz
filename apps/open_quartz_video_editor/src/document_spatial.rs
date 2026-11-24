use super::{Clip, ClipTransform, EditorDocument};
use serde::{Deserialize, Serialize};

const MAX_KEYS: usize = 256;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Crop {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Crop {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub(super) fn validate(self) -> Result<(), String> {
        if [self.left, self.top, self.right, self.bottom]
            .into_iter()
            .all(|value| (0.0..1.0).contains(&value))
            && self.left + self.right < 1.0
            && self.top + self.bottom < 1.0
        {
            Ok(())
        } else {
            Err("Crop must leave a visible area inside the clip".into())
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Pose {
    pub x: f32,
    pub y: f32,
    pub scale: f32,
    pub rotation: f32,
}

impl Pose {
    fn from_transform(transform: ClipTransform) -> Self {
        Self {
            x: transform.x,
            y: transform.y,
            scale: transform.scale,
            rotation: transform.rotation,
        }
    }

    fn apply(self, transform: &mut ClipTransform) {
        transform.x = self.x;
        transform.y = self.y;
        transform.scale = self.scale;
        transform.rotation = self.rotation;
    }

    fn interpolate(self, other: Self, t: f64) -> Self {
        let mix = |a, b| (f64::from(a) + (f64::from(b) - f64::from(a)) * t) as f32;
        Self {
            x: mix(self.x, other.x),
            y: mix(self.y, other.y),
            scale: mix(self.scale, other.scale),
            rotation: mix(self.rotation, other.rotation),
        }
    }

    fn valid(self) -> bool {
        (-1.0..=1.0).contains(&self.x)
            && (-1.0..=1.0).contains(&self.y)
            && (0.1..=4.0).contains(&self.scale)
            && (-180.0..=180.0).contains(&self.rotation)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SpatialKey {
    pub offset: i64,
    pub pose: Pose,
    #[serde(default, skip_serializing_if = "super::is_false")]
    pub ease: bool,
}

impl Clip {
    fn pose_at(&self, offset: i128) -> Pose {
        let keys = &self.spatial_keys;
        let right = keys.partition_point(|key| i128::from(key.offset) <= offset);
        match (
            right.checked_sub(1).map(|index| &keys[index]),
            keys.get(right),
        ) {
            (Some(left), Some(right)) => {
                let delta = offset - i128::from(left.offset);
                let length = i128::from(right.offset) - i128::from(left.offset);
                let t = delta as f64 / length as f64;
                let t = if left.ease {
                    t * t * (3.0 - 2.0 * t)
                } else {
                    t
                };
                left.pose.interpolate(right.pose, t)
            }
            (Some(key), None) | (None, Some(key)) => key.pose,
            (None, None) => Pose::from_transform(self.transform),
        }
    }

    pub fn transform_at(&self, frame: u64) -> ClipTransform {
        let mut transform = self.transform;
        self.pose_at(i128::from(frame) - i128::from(self.timeline_start))
            .apply(&mut transform);
        transform
    }

    fn offset_at(&self, frame: u64) -> Result<i64, String> {
        if frame >= self.timeline_end()? {
            return Err("Move the playhead inside this clip to edit its transform keys".into());
        }
        frame
            .checked_sub(self.timeline_start)
            .and_then(|offset| i64::try_from(offset).ok())
            .ok_or_else(|| "Move the playhead inside this clip to edit its transform keys".into())
    }

    pub fn spatial_key_at(&self, frame: u64) -> Option<SpatialKey> {
        let offset = self.offset_at(frame).ok()?;
        let index = self
            .spatial_keys
            .binary_search_by_key(&offset, |key| key.offset)
            .ok()?;
        Some(self.spatial_keys[index])
    }

    pub(super) fn validate_spatial(&self) -> Result<(), String> {
        self.crop.validate()?;
        if self.spatial_keys.len() > MAX_KEYS
            || self.spatial_keys.iter().any(|key| !key.pose.valid())
            || self
                .spatial_keys
                .windows(2)
                .any(|pair| pair[0].offset >= pair[1].offset)
        {
            return Err("Invalid transform keyframes".into());
        }
        Ok(())
    }

    pub(super) fn preserve_spatial_keys_on_trim(&mut self, start: u64) -> Result<(), String> {
        let shift = i128::from(self.timeline_start) - i128::from(start);
        for key in &mut self.spatial_keys {
            key.offset = i64::try_from(i128::from(key.offset) + shift)
                .map_err(|_| "Transform keyframe position overflow")?;
        }
        Ok(())
    }

    fn write_spatial_key(&mut self, offset: i64, pose: Pose) -> Result<(), String> {
        match self
            .spatial_keys
            .binary_search_by_key(&offset, |key| key.offset)
        {
            Ok(index) => self.spatial_keys[index].pose = pose,
            Err(index) if self.spatial_keys.len() < MAX_KEYS => {
                self.spatial_keys.insert(
                    index,
                    SpatialKey {
                        offset,
                        pose,
                        ease: false,
                    },
                );
            }
            Err(_) => return Err("A clip supports up to 256 transform keyframes".into()),
        }
        Ok(())
    }
}

impl EditorDocument {
    pub fn set_transform_at(
        &mut self,
        id: &str,
        frame: u64,
        value: ClipTransform,
        revision: u64,
    ) -> Result<(), String> {
        self.transact_at(revision, |data| {
            let (track, index) = data.writable_clip(id)?;
            let clip = &mut data.tracks[track].clips[index];
            if clip.spatial_keys.is_empty() {
                clip.transform = value;
            } else {
                let offset = clip.offset_at(frame)?;
                clip.write_spatial_key(offset, Pose::from_transform(value))?;
                clip.transform.exposure = value.exposure;
                clip.transform.opacity = value.opacity;
            }
            Ok(())
        })
    }

    pub fn spatial_action(&mut self, id: &str, frame: u64, action: &str) -> Result<(), String> {
        self.transact(|data| {
            let (track, index) = data.writable_clip(id)?;
            let clip = &mut data.tracks[track].clips[index];
            let offset = clip.offset_at(frame)?;
            let key = clip
                .spatial_keys
                .binary_search_by_key(&offset, |key| key.offset);
            match (action, key) {
                ("toggle-key", Ok(index)) => {
                    let removed = clip.spatial_keys.remove(index);
                    if clip.spatial_keys.is_empty() {
                        removed.pose.apply(&mut clip.transform);
                    }
                }
                ("toggle-key", Err(_)) => {
                    let pose = clip.pose_at(i128::from(offset));
                    clip.write_spatial_key(offset, pose)?;
                }
                ("ease", Ok(index)) => {
                    clip.spatial_keys[index].ease = !clip.spatial_keys[index].ease
                }
                _ => return Err("Select a transform keyframe to change easing".into()),
            }
            Ok(())
        })
    }

    pub fn set_crop(&mut self, id: &str, edge: &str, value: f32) -> Result<(), String> {
        self.transact(|data| {
            let (track, index) = data.writable_clip(id)?;
            let crop = &mut data.tracks[track].clips[index].crop;
            match edge {
                "crop-left" => crop.left = value,
                "crop-top" => crop.top = value,
                "crop-right" => crop.right = value,
                "crop-bottom" => crop.bottom = value,
                _ => return Err("Unknown crop edge".into()),
            }
            Ok(())
        })
    }
}
