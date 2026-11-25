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

const NESTED_ASSET_PREFIX: &str = "sequence:";

impl MediaAsset {
    pub fn sequence_source(&self) -> Option<&str> {
        self.id.strip_prefix(NESTED_ASSET_PREFIX)
    }
}

fn nested_asset_id(sequence: &str) -> String {
    format!("{NESTED_ASSET_PREFIX}{sequence}")
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

    pub fn sequence_project(&self, id: &str) -> Result<Self, String> {
        let mut child = self.clone();
        let position = child.other_sequences.iter().position(|slot| slot.id == id)
            .ok_or("Nested sequence is missing")?;
        let current = child.active_sequence();
        let sequence = std::mem::replace(&mut child.other_sequences[position], current);
        child.activate(sequence);
        Ok(child)
    }

    fn validate_sequence_references(&self) -> Result<(), String> {
        fn visit<'a>(
            sequence: &'a str, edges: &std::collections::BTreeMap<&'a str, Vec<&'a str>>,
            opened: &mut std::collections::BTreeSet<&'a str>,
            finished: &mut std::collections::BTreeMap<&'a str, usize>,
        ) -> Result<usize, String> {
            if let Some(&depth) = finished.get(sequence) { return Ok(depth); }
            if !opened.insert(sequence) { return Err("A sequence cannot contain itself through a nest".into()); }
            if opened.len() > 12 { return Err("Sequences can be nested up to 12 levels".into()); }
            let mut depth = 1;
            for child in edges.get(sequence).ok_or("Nested sequence is missing")? {
                depth = depth.max(1 + visit(child, edges, opened, finished)?);
            }
            if depth > 12 { return Err("Sequences can be nested up to 12 levels".into()); }
            opened.remove(sequence);
            finished.insert(sequence, depth);
            Ok(depth)
        }
        let tracks = std::iter::once((self.sequence_id.as_str(), &self.tracks))
            .chain(self.other_sequences.iter().map(|slot| (slot.id.as_str(), &slot.tracks)));
        let edges = tracks.map(|(id, tracks)| {
            (id, tracks.iter().flat_map(|track| &track.clips)
                .filter_map(|clip| clip.asset_id.strip_prefix(NESTED_ASSET_PREFIX)).collect())
        }).collect::<std::collections::BTreeMap<_, _>>();
        let mut finished = std::collections::BTreeMap::new();
        for id in edges.keys() {
            visit(id, &edges, &mut Default::default(), &mut finished)?;
        }
        Ok(())
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
            super::validate_timeline(&snapshot).map_err(|error| format!("{} · {error}", slot.name))?;
        }
        self.validate_sequence_references()
    }
}

impl EditorDocument {
    pub fn place_sequence(&mut self, id: &str, track: &str, start: u64) -> Result<String, String> {
        if self.data.sequence_id == id {
            return Err("Select a different sequence to nest".into());
        }
        let child = self.data.sequence_project(id)?;
        let asset_id = nested_asset_id(id);
        let duration = i64::try_from(child.duration_frames()).map_err(|_| "Sequence duration is too long")?;
        if duration == 0 { return Err("Add clips to the source sequence before nesting it".into()); }
        let track_index = self.data.track_index(track)?;
        self.data.tracks[track_index].require_unlocked()?;
        if self.data.tracks[track_index].subtitles { return Err("Select a video track to nest a sequence".into()); }
        let revision = self.revision;
        let clip_id = new_id("clip", revision);
        self.transact(|data| {
            // Keep the original source clock for already-authored trims. A child edit can
            // change its program content without moving the instance on the parent timeline.
            let asset = data.assets.iter().find(|asset| asset.id == asset_id);
            let child_clock = Rational::new(child.fps.denominator, child.fps.numerator)?;
            if let Some(asset) = asset {
                if asset.time_base != child_clock {
                    return Err("Restore the source sequence frame rate used by its existing nests".into());
                }
            }
            let length = super::duration_frames(duration, child_clock, data.fps)?;
            if length == 0 { return Err("Source sequence is shorter than one destination frame".into()); }
            let clip = Clip {
                id: clip_id.clone(), asset_id: asset_id.clone(), title: None,
                timeline_start: start, duration: length, speed_percent: normal_speed(),
                source_span_remainder: 0, source_in_pts: 0, source_subtick_numerator: 0,
                source_subtick_denominator: 1, transform: ClipTransform::default(),
                opacity_keys: Vec::new(), crop: Crop::default(), crop_keys: Vec::new(),
                spatial_keys: Vec::new(), pipeline: ClipPipeline::default(), effect_keys: Vec::new(),
            };
            if let Some(asset) = data.assets.iter_mut().find(|asset| asset.id == asset_id) {
                asset.duration_pts = Some(asset.duration_pts.unwrap_or(0).max(duration));
            } else {
                data.assets.push(MediaAsset {
                    id: asset_id.clone(), name: child.sequence_name, path: String::new(),
                    fingerprint: asset_id, size_bytes: 0, still: false,
                    width: child.width, height: child.height, duration_pts: Some(duration),
                    start_pts: Some(0), time_base: child_clock, fps: Some(child.fps),
                    has_audio: false, audio: None, online: true,
                });
            }
            data.tracks[track_index].clips.push(clip);
            data.tracks[track_index].clips.sort_by_key(|clip| clip.timeline_start);
            Ok(())
        })?;
        Ok(clip_id)
    }

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
            let asset_id = nested_asset_id(&data.sequence_id);
            if let Some(asset) = data.assets.iter_mut().find(|asset| asset.id == asset_id) {
                asset.name = data.sequence_name.clone();
            }
            Ok(())
        })
    }
}
