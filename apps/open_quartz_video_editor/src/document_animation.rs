use super::{source_time_ns, Clip, EditorDocument, ProjectData};
use crate::animation_key::{self, Keyframe};

pub type OpacityKey = Keyframe<u64, f32>;

impl Clip {
    pub fn opacity_at(&self, source_ns: u64) -> f32 {
        animation_key::sample(&self.opacity_keys, i128::from(source_ns), self.transform.opacity, animation_key::mix_scalar)
    }

    pub(super) fn validate_opacity_keys(&self) -> Result<(), String> {
        if animation_key::validate(&self.opacity_keys, |opacity| (0.0..=1.0).contains(opacity)) {
            Ok(())
        } else {
            Err("Invalid clip opacity keyframes".into())
        }
    }
}

impl ProjectData {
    pub fn clip_source_time(&self, clip: &Clip, frame: u64) -> Result<u64, String> {
        let asset = self
            .assets
            .iter()
            .find(|asset| asset.id == clip.asset_id)
            .ok_or("Clip media is missing")?;
        if asset.still {
            return Err("Still images use sequence timing".into());
        }
        source_time_ns(asset, clip, frame, self.fps)
    }
}

impl EditorDocument {
    /// An explicit opacity value creates or updates a key at the playhead.
    /// With no value, capture the current interpolation.
    pub fn set_opacity_key(
        &mut self,
        id: &str,
        frame: u64,
        opacity: Option<f32>,
    ) -> Result<(), String> {
        self.transact(|data| {
            let (track, index) = data.writable_clip(id)?;
            let clip = &data.tracks[track].clips[index];
            let source_ns = data.clip_source_time(clip, frame)?;
            let opacity = opacity.unwrap_or_else(|| clip.opacity_at(source_ns));
            animation_key::set_key(&mut data.tracks[track].clips[index].opacity_keys, source_ns, opacity)?;
            Ok(())
        })
    }

    pub fn remove_opacity_key(&mut self, id: &str, frame: u64) -> Result<(), String> {
        self.transact(|data| {
            let (track, index) = data.writable_clip(id)?;
            let source_ns = data.clip_source_time(&data.tracks[track].clips[index], frame)?;
            let clip = &mut data.tracks[track].clips[index];
            let index = clip
                .opacity_keys
                .binary_search_by_key(&source_ns, |key| key.time)
                .map_err(|_| "The playhead is not on an opacity keyframe")?;
            let key = clip.opacity_keys.remove(index);
            if clip.opacity_keys.is_empty() {
                clip.transform.opacity = key.value;
            }
            Ok(())
        })
    }
}
