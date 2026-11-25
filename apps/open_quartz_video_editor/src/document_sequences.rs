use super::*;

pub(super) fn default_sequence_id() -> String {
    "sequence-1".into()
}

pub(super) fn default_sequence_name() -> String {
    "Sequence 1".into()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TimelineSequence {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: Rational,
    pub tracks: Vec<VideoTrack>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audio_tracks: Vec<audio::AudioTrack>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<Marker>,
}

impl ProjectData {
    pub fn sequence_list(&self) -> Vec<(&str, &str)> {
        std::iter::once((self.sequence_id.as_str(), self.sequence_name.as_str()))
            .chain(self.other_sequences.iter().map(|slot| (slot.id.as_str(), slot.name.as_str())))
            .collect()
    }

    pub fn sequence_options(&self) -> Vec<(&str, &str)> {
        let mut slots = self.sequence_list();
        slots.sort_by(|(a, _), (b, _)| a.cmp(b));
        slots
    }

    fn active_sequence(&self) -> TimelineSequence {
        TimelineSequence {
            id: self.sequence_id.clone(),
            name: self.sequence_name.clone(),
            width: self.width,
            height: self.height,
            fps: self.fps,
            tracks: self.tracks.clone(),
            audio_tracks: self.audio_tracks.clone(),
            markers: self.markers.clone(),
        }
    }

    fn activate(&mut self, sequence: TimelineSequence) {
        self.sequence_id = sequence.id;
        self.sequence_name = sequence.name;
        self.width = sequence.width;
        self.height = sequence.height;
        self.fps = sequence.fps;
        self.tracks = sequence.tracks;
        self.audio_tracks = sequence.audio_tracks;
        self.markers = sequence.markers;
    }

    pub(super) fn validate_sequence_slots(&self) -> Result<(), String> {
        let mut ids = std::collections::BTreeSet::new();
        let mut names = std::collections::BTreeSet::new();
        for (id, name) in self.sequence_list() {
            if id.is_empty() || !ids.insert(id) || !names.insert(name)
                || name.trim().is_empty() || name.chars().count() > 80
            {
                return Err("Sequence names and IDs must be distinct and nonempty".into());
            }
        }
        for slot in &self.other_sequences {
            let mut snapshot = ProjectData {
                schema_version: self.schema_version,
                id: self.id.clone(),
                name: self.name.clone(),
                assets: self.assets.clone(),
                ..ProjectData::default()
            };
            snapshot.activate(slot.clone());
            super::validate(&snapshot).map_err(|error| format!("{} · {error}", slot.name))?;
        }
        Ok(())
    }
}

impl EditorDocument {
    pub fn add_sequence(&mut self, duplicate: bool) -> Result<(), String> {
        let id = new_id("sequence", self.revision);
        self.transact(|data| {
            let previous = data.active_sequence();
            let mut next = previous.clone();
            next.id = id;
            let existing = data.sequence_list();
            next.name = if duplicate {
                let base: String = previous.name.chars().take(60).collect();
                (1..).map(|index| {
                    if index == 1 { format!("{base} copy") } else { format!("{base} copy {index}") }
                }).find(|name| !existing.iter().any(|(_, existing)| *existing == name)).unwrap()
            } else {
                (1..).map(|index| format!("Sequence {index}"))
                    .find(|name| !existing.iter().any(|(_, existing)| *existing == name)).unwrap()
            };
            if !duplicate {
                next.tracks = vec![VideoTrack::new(DEFAULT_TRACK.into(), "Video 1".into())];
                next.audio_tracks.clear();
                next.markers.clear();
            }
            data.other_sequences.push(previous);
            data.activate(next);
            Ok(())
        })
    }

    pub fn switch_sequence(&mut self, id: &str) -> Result<(), String> {
        if self.data.sequence_id == id { return Ok(()); }
        self.transact(|data| {
            let position = data.other_sequences.iter().position(|slot| slot.id == id)
                .ok_or("Sequence is missing")?;
            let previous = data.active_sequence();
            let next = std::mem::replace(&mut data.other_sequences[position], previous);
            data.activate(next);
            Ok(())
        })
    }

    pub fn rename_sequence(&mut self, name: &str) -> Result<(), String> {
        self.transact(|data| {
            data.sequence_name = name.trim().to_owned();
            Ok(())
        })
    }
}
