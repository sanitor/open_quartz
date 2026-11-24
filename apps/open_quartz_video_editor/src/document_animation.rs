use super::{source_time_ns, Clip, EditorDocument, ProjectData};
use serde::{Deserialize, Serialize};

const MAX_OPACITY_KEYS: usize = 256;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpacityKey {
    pub source_ns: u64,
    pub opacity: f32,
}

impl Clip {
    pub fn opacity_at(&self, source_ns: u64) -> f32 {
        let keys = &self.opacity_keys;
        let right = keys.partition_point(|key| key.source_ns <= source_ns);
        match (
            right.checked_sub(1).and_then(|left| keys.get(left)),
            keys.get(right),
        ) {
            (Some(left), Some(right)) => {
                let progress =
                    (source_ns - left.source_ns) as f64 / (right.source_ns - left.source_ns) as f64;
                (f64::from(left.opacity)
                    + (f64::from(right.opacity) - f64::from(left.opacity)) * progress)
                    as f32
            }
            (Some(key), None) | (None, Some(key)) => key.opacity,
            (None, None) => self.transform.opacity,
        }
    }

    pub(super) fn validate_opacity_keys(&self) -> Result<(), String> {
        if self.opacity_keys.len() > MAX_OPACITY_KEYS
            || self
                .opacity_keys
                .iter()
                .any(|key| !(0.0..=1.0).contains(&key.opacity))
            || self
                .opacity_keys
                .windows(2)
                .any(|pair| pair[0].source_ns >= pair[1].source_ns)
        {
            return Err("Invalid clip opacity keyframes".into());
        }
        Ok(())
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
            let key = OpacityKey { source_ns, opacity };
            let keys = &mut data.tracks[track].clips[index].opacity_keys;
            match keys.binary_search_by_key(&source_ns, |key| key.source_ns) {
                Ok(index) => keys[index] = key,
                Err(index) if keys.len() < MAX_OPACITY_KEYS => keys.insert(index, key),
                Err(_) => {
                    return Err(format!(
                        "A clip supports up to {MAX_OPACITY_KEYS} opacity keyframes"
                    ))
                }
            }
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
                .binary_search_by_key(&source_ns, |key| key.source_ns)
                .map_err(|_| "The playhead is not on an opacity keyframe")?;
            let key = clip.opacity_keys.remove(index);
            if clip.opacity_keys.is_empty() {
                clip.transform.opacity = key.opacity;
            }
            Ok(())
        })
    }
}
