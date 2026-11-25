use super::{
    is_false, new_id, normal_speed, Clip, ClipPipeline, ClipTransform, EditorDocument, VideoTrack,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Title {
    pub text: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub subtitle: bool,
    /// Font size as a percentage of the sequence canvas height.
    pub size_percent: u16,
}

impl Default for Title {
    fn default() -> Self {
        Self {
            text: "Title".into(),
            subtitle: false,
            size_percent: 10,
        }
    }
}

impl Title {
    pub fn validate(&self) -> Result<(), String> {
        if self.text.trim().is_empty() || self.text.chars().count() > 256 {
            return Err(if self.subtitle {
                "Subtitle text must contain 1 to 256 characters"
            } else {
                "Title text must contain 1 to 256 characters"
            }
            .into());
        }
        if !(1..=30).contains(&self.size_percent) {
            return Err("Title size must be a percentage from 1 to 30".into());
        }
        Ok(())
    }
}

impl EditorDocument {
    /// Place a title over the existing edit as one undoable command.
    pub fn add_title(&mut self, start: u64) -> Result<(String, String), String> {
        let track_id = new_id("title-track", self.revision);
        let clip_id = new_id("title", self.revision);
        self.transact(|data| {
            let duration = data.fps.ns_to_frames_floor(3_000_000_000)?.max(1);
            let mut track = VideoTrack::new(track_id.clone(), "Titles".into());
            track.clips.push(generated_clip(
                clip_id.clone(),
                start,
                duration,
                Title::default(),
            ));
            data.tracks.insert(0, track);
            Ok(())
        })?;
        Ok((track_id, clip_id))
    }

    pub fn edit_title(&mut self, clip_id: &str, field: &str, value: &str) -> Result<(), String> {
        self.transact(|data| {
            let (lane, index) = data.writable_clip(clip_id)?;
            let title = data.tracks[lane].clips[index]
                .title
                .as_mut()
                .ok_or("Select a title clip")?;
            match field {
                "title-text" | "subtitle-text" => title.text = value.to_owned(),
                "title-size" => {
                    title.size_percent = value
                        .trim()
                        .parse()
                        .map_err(|_| "Title size must be a percentage from 1 to 30")?
                }
                _ => return Err("Unknown title property".into()),
            }
            Ok(())
        })
    }
}

pub(super) fn generated_clip(id: String, start: u64, duration: u64, text: Title) -> Clip {
    Clip {
        id,
        asset_id: String::new(),
        title: Some(text),
        timeline_start: start,
        duration,
        speed_percent: normal_speed(),
        source_span_remainder: 0,
        source_in_pts: 0,
        source_subtick_numerator: 0,
        source_subtick_denominator: 1,
        transform: ClipTransform::default(),
        opacity_keys: Vec::new(), mask: None,
        crop: Default::default(),
        crop_keys: Vec::new(),
        spatial_keys: Vec::new(),
        pipeline: ClipPipeline::default(),
        effect_keys: Vec::new(), grade_versions: None,
    }
}
