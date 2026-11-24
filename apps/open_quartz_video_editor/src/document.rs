use crate::clip_pipeline::ClipPipeline;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[path = "document_audio.rs"]
mod audio;
#[path = "document_timing.rs"]
mod timing;
#[path = "document_transition.rs"]
mod transition;
#[path = "document_title.rs"]
mod title;
#[path = "document_subtitle.rs"]
mod subtitle;
pub use title::Title;
pub use transition::DissolveSide;
#[path = "document_animation.rs"]
mod animation;
pub use animation::OpacityKey;
pub use timing::{normal_speed, valid_speed, Placement};
pub use audio::{AudioAsset, AudioInterval, AudioPlacement, SAMPLE_RATE};
const SCHEMA_VERSION: u32 = 13;
#[path = "document_spatial.rs"]
mod spatial;
pub use spatial::{Crop, SpatialKey};
#[path = "document_timeline_tools.rs"]
mod timeline_tools;
pub use timeline_tools::Marker;
pub const DEFAULT_TRACK: &str = "video-1";
const HISTORY_LIMIT: usize = 100;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Rational {
    pub numerator: i32,
    pub denominator: i32,
}
impl Rational {
    pub fn new(numerator: i32, denominator: i32) -> Result<Self, String> {
        if numerator <= 0 || denominator <= 0 {
            return Err("Frame rate must be a positive rational number".into());
        }
        let divisor = gcd(numerator as u32, denominator as u32) as i32;
        Ok(Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        })
    }
    pub fn frames_to_ns(self, frames: u64) -> Result<u64, String> {
        Self::new(self.numerator, self.denominator)?;
        let value = u128::from(frames)
            .checked_mul(u128::from(self.denominator as u32))
            .and_then(|v| v.checked_mul(1_000_000_000))
            .ok_or("Sequence time overflow")?
            / self.numerator as u128;
        u64::try_from(value).map_err(|_| "Sequence time exceeds nanosecond range".into())
    }
    pub fn ns_to_frames_floor(self, ns: u64) -> Result<u64, String> {
        Self::new(self.numerator, self.denominator)?;
        let value = u128::from(ns)
            .checked_mul(self.numerator as u128)
            .ok_or("Frame position overflow")?
            / u128::from(self.denominator as u32)
            / 1_000_000_000;
        u64::try_from(value).map_err(|_| "Frame position exceeds supported range".into())
    }
}
fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    a.max(1)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MediaAsset {
    pub id: String,
    pub name: String,
    /// Canonical absolute location while in memory; save writes a project-relative URI when possible.
    pub path: String,
    pub fingerprint: String,
    pub size_bytes: u64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub still: bool,
    pub width: u32,
    pub height: u32,
    pub duration_pts: Option<i64>,
    pub start_pts: Option<i64>,
    pub time_base: Rational,
    pub fps: Option<Rational>,
    pub has_audio: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioAsset>,
    pub online: bool,
}
impl MediaAsset {
    pub fn has_visual(&self) -> bool {
        self.width != 0 && self.height != 0
    }

    pub fn has_video(&self) -> bool {
        self.has_visual() && !self.still
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ClipTransform {
    pub scale: f32,
    pub x: f32,
    pub y: f32,
    pub exposure: f32,
    #[serde(default)]
    pub rotation: f32,
    #[serde(default = "opaque")]
    pub opacity: f32,
}
fn opaque() -> f32 { 1.0 }

impl Default for ClipTransform {
    fn default() -> Self {
        Self {
            scale: 1.0,
            x: 0.0,
            y: 0.0,
            exposure: 0.0,
            rotation: 0.0,
            opacity: 1.0,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Clip {
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub asset_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<Title>,
    pub timeline_start: u64,
    pub duration: u64,
    #[serde(default = "normal_speed", skip_serializing_if = "is_normal_speed")]
    pub speed_percent: u16,
    /// Retains the unconsumed source fraction when the result is rounded down to whole frames.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub source_span_remainder: u16,
    pub source_in_pts: i64,
    /// Exact fractional source tick, always in [0, 1); retained across splits and trims.
    #[serde(default)]
    pub source_subtick_numerator: u64,
    #[serde(default = "one")]
    pub source_subtick_denominator: u64,
    pub transform: ClipTransform,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub opacity_keys: Vec<OpacityKey>,
    #[serde(default, skip_serializing_if = "Crop::is_empty")]
    pub crop: Crop,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spatial_keys: Vec<SpatialKey>,
    pub pipeline: ClipPipeline,
}
pub(super) fn is_normal_speed(value: &u16) -> bool { *value == normal_speed() }
fn is_zero(value: &u16) -> bool { *value == 0 }
impl Clip {
    pub fn timeline_end(&self) -> Result<u64, String> {
        self.timeline_start
            .checked_add(self.duration)
            .ok_or_else(|| "Clip end frame overflow".into())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct VideoTrack {
    pub id: String,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub subtitles: bool,
    pub clips: Vec<Clip>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dissolves: Vec<transition::Dissolve>,
}
fn is_false(value: &bool) -> bool { !value }
impl VideoTrack {
    fn new(id: String, name: String) -> Self {
        Self { id, name, visible: true, locked: false, subtitles: false, clips: Vec::new(), dissolves: Vec::new() }
    }

    pub fn clip_at(&self, frame: u64) -> Option<&Clip> {
        let index = self.clips.partition_point(|clip| clip.timeline_start <= frame);
        self.clips.get(index.checked_sub(1)?)
            .filter(|clip| clip.timeline_end().is_ok_and(|end| frame < end))
    }

    fn require_unlocked(&self) -> Result<(), String> {
        if self.locked { return Err(format!("{} is locked", self.name)); }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProjectData {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: Rational,
    pub assets: Vec<MediaAsset>,
    pub tracks: Vec<VideoTrack>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audio_tracks: Vec<audio::AudioTrack>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<Marker>,
}
impl Default for ProjectData {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            id: format!("oqvideo-{}", std::process::id()),
            name: "Untitled Video".into(),
            width: 1920,
            height: 1080,
            fps: Rational {
                numerator: 30,
                denominator: 1,
            },
            assets: Vec::new(),
            tracks: vec![VideoTrack::new(DEFAULT_TRACK.into(), "Video 1".into())],
            audio_tracks: Vec::new(),
            markers: Vec::new(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectFile {
    schema_version: u32,
    data: ProjectData,
}

impl ProjectData {
    pub fn clips(&self) -> impl Iterator<Item = &Clip> {
        self.tracks.iter().flat_map(|track| &track.clips)
    }

    pub fn active_clips(&self, frame: u64) -> impl Iterator<Item = (&VideoTrack, &Clip)> {
        self.tracks.iter().filter(|track| track.visible)
            .filter_map(move |track| track.clip_at(frame).map(|clip| (track, clip)))
    }

    pub fn duration_frames(&self) -> u64 {
        self.clips().filter_map(|clip| clip.timeline_end().ok())
            .chain(self.audio_tracks.iter().flat_map(|track| &track.clips).filter_map(|clip| {
                match clip.placement {
                    AudioPlacement::Independent { start, duration, .. } =>
                        self.fps.sample_to_frame_ceil(start.checked_add(duration)?).ok(),
                    AudioPlacement::Linked { .. } => None,
                }
            })).max().unwrap_or(0)
    }

    fn clip_position(&self, id: &str) -> Result<(usize, usize), String> {
        self.tracks.iter().enumerate().find_map(|(track, lane)| {
            lane.clips.iter().position(|clip| clip.id == id).map(|clip| (track, clip))
        }).ok_or_else(|| "Selected clip no longer exists".into())
    }

    fn writable_clip(&self, id: &str) -> Result<(usize, usize), String> {
        let position = self.clip_position(id)?;
        self.tracks[position.0].require_unlocked()?;
        Ok(position)
    }

    fn track_index(&self, id: &str) -> Result<usize, String> {
        self.tracks.iter().position(|track| track.id == id)
            .ok_or_else(|| "Video track no longer exists".into())
    }
}

pub struct EditorDocument {
    pub data: ProjectData,
    path: Option<PathBuf>,
    undo: VecDeque<ProjectData>,
    redo: Vec<ProjectData>,
    pub revision: u64,
    pub saved_revision: u64,
    saved_data: ProjectData,
}
fn one() -> u64 {
    1
}
impl Default for EditorDocument {
    fn default() -> Self {
        Self::new(ProjectData::default()).expect("default project valid")
    }
}
impl EditorDocument {
    pub fn new(data: ProjectData) -> Result<Self, String> {
        validate(&data)?;
        Ok(Self {
            saved_data: data.clone(),
            data,
            path: None,
            undo: VecDeque::new(),
            redo: Vec::new(),
            revision: 0,
            saved_revision: 0,
        })
    }
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
    pub fn is_dirty(&self) -> bool {
        self.data != self.saved_data
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn asset(&self, id: &str) -> Option<&MediaAsset> {
        self.data.assets.iter().find(|asset| asset.id == id)
    }
    pub fn clip(&self, id: &str) -> Option<&Clip> {
        let (track, clip) = self.data.clip_position(id).ok()?;
        Some(&self.data.tracks[track].clips[clip])
    }
    pub fn clip_track(&self, id: &str) -> Option<&VideoTrack> {
        let (track, _) = self.data.clip_position(id).ok()?;
        self.data.tracks.get(track)
    }
    pub fn clip_at(&self, frame: u64) -> Option<&Clip> {
        self.data.active_clips(frame).next().map(|(_, clip)| clip)
    }
    pub fn duration_frames(&self) -> u64 {
        self.data.duration_frames()
    }
    pub fn add_asset(&mut self, asset: MediaAsset) -> Result<(), String> {
        self.transact(|data| {
            if data
                .assets
                .iter()
                .any(|item| item.id == asset.id || item.fingerprint == asset.fingerprint)
            {
                return Err("This media file is already in the project".into());
            }
            data.assets.push(asset);
            Ok(())
        })
    }
    pub fn set_clip_pipeline(
        &mut self,
        clip_id: &str,
        pipeline: ClipPipeline,
        expected_revision: u64,
    ) -> Result<(), String> {
        self.transact_at(expected_revision, |data| {
            let (track, index) = data.writable_clip(clip_id)?;
            let clip = &mut data.tracks[track].clips[index];
            clip.pipeline = pipeline;
            Ok(())
        })
    }
    #[cfg(test)]
    pub fn add_clip(&mut self, asset_id: &str) -> Result<String, String> {
        let track = self.data.tracks[0].id.clone();
        self.place_clip(asset_id, &track, None)
    }

    pub fn place_clip(&mut self, asset_id: &str, track: &str, start: Option<u64>) -> Result<String, String> {
        self.place_visual(asset_id, track, Placement::EmptySpace(start))
    }

    pub fn place_visual(&mut self, asset_id: &str, track: &str, placement: Placement) -> Result<String, String> {
        let track = self.data.track_index(track)?;
        self.data.tracks[track].require_unlocked()?;
        if self.data.tracks[track].subtitles {
            return Err("Select a video track to place media".into());
        }
        let asset = self
            .asset(asset_id)
            .cloned()
            .ok_or("Media asset does not exist")?;
        if !asset.has_visual() { return Err("Select an image or video asset".into()); }
        let duration = if asset.still {
            self.data.fps.ns_to_frames_floor(3_000_000_000)?.max(1)
        } else {
            let ticks = asset.duration_pts.ok_or("Exact placement requires a known video duration")?;
            duration_frames(ticks, asset.time_base, self.data.fps)?
        };
        if duration == 0 {
            return Err("Media is shorter than one sequence frame".into());
        }
        let start = match placement {
            Placement::EmptySpace(None) => first_gap(&self.data.tracks[track].clips, duration)?,
            Placement::EmptySpace(Some(frame)) | Placement::Insert(frame) | Placement::Overwrite(frame) => frame,
        };
        let id = new_id("clip", self.revision);
        let source_in = if asset.still {
            0
        } else {
            asset.start_pts.ok_or("Precise placement requires a known source presentation timestamp")?
        };
        let has_audio = asset.audio.is_some();
        let revision = self.revision;
        let clip = Clip {
            id: id.clone(),
            asset_id: asset.id,
            title: None,
            timeline_start: start,
            duration,
            speed_percent: normal_speed(),
            source_span_remainder: 0,
            source_in_pts: source_in,
            source_subtick_numerator: 0,
            source_subtick_denominator: 1,
            transform: ClipTransform::default(),
            opacity_keys: Vec::new(),
            crop: Crop::default(),
            spatial_keys: Vec::new(),
            pipeline: ClipPipeline::default(),
        };
        self.transact(|data| {
            match placement {
                Placement::EmptySpace(_) => {}
                Placement::Insert(frame) => data.insert_space(track, frame, duration, revision)?,
                Placement::Overwrite(frame) => data.clear_range(track, frame, clip.timeline_end()?, revision)?,
            }
            data.tracks[track].clips.push(clip);
            data.tracks[track].clips.sort_by_key(|clip| clip.timeline_start);
            if has_audio { data.link_audio(revision, asset_id, id.clone())?; }
            Ok(())
        })?;
        Ok(id)
    }
    pub fn split(&mut self, clip_id: &str, at: u64) -> Result<String, String> {
        let revision = self.revision;
        let mut result = None;
        self.transact(|data| {
            let (track, index) = data.writable_clip(clip_id)?;
            result = Some(data.split_video(track, index, at, revision)?);
            Ok(())
        })?;
        Ok(result.expect("successful split returns its right clip"))
    }
    pub fn lift(&mut self, clip_id: &str) -> Result<(), String> {
        self.transact(|data| {
            let (track, index) = data.writable_clip(clip_id)?;
            data.remove_video(track, index..index + 1);
            Ok(())
        })
    }
    pub fn move_clip(&mut self, clip_id: &str, target: u64) -> Result<(), String> {
        let track = self.clip_track(clip_id).ok_or("Selected clip no longer exists")?.id.clone();
        self.move_clip_to(clip_id, &track, target)
    }
    pub fn check_move(&self, clip_id: &str, track: &str, start: u64) -> Result<(), String> {
        let (from, index) = self.data.writable_clip(clip_id)?;
        let lane = &self.data.tracks[self.data.track_index(track)?];
        lane.require_unlocked()?;
        if lane.subtitles != self.data.tracks[from].subtitles {
            return Err("Subtitle cues belong to the subtitle track".into());
        }
        let end = start.checked_add(self.data.tracks[from].clips[index].duration)
            .ok_or("Clip end frame overflow")?;
        if lane.clips.iter().any(|clip| clip.id != clip_id
            && overlaps(start, end, clip.timeline_start, clip.timeline_end().unwrap_or(u64::MAX))) {
            return Err("Clip overlaps another clip on the video track".into());
        }
        Ok(())
    }
    pub fn move_clip_to(&mut self, clip_id: &str, track: &str, target: u64) -> Result<(), String> {
        self.check_move(clip_id, track, target)?;
        self.transact(|data| {
            let (from, index) = data.writable_clip(clip_id)?;
            let to = data.track_index(track)?;
            let mut clip = data.tracks[from].clips.remove(index);
            clip.timeline_start = target;
            data.tracks[to].clips.push(clip);
            data.tracks[to].clips.sort_by_key(|clip| clip.timeline_start);
            Ok(())
        })
    }
    pub fn add_track(&mut self, before: &str) -> Result<String, String> {
        let id = new_id("track", self.revision);
        self.transact(|data| {
            let index = data.track_index(before)?;
            let number = (1..).find(|n| !data.tracks.iter().any(|track| track.name == format!("Video {n}"))).unwrap();
            data.tracks.insert(index, VideoTrack::new(id.clone(), format!("Video {number}")));
            Ok(())
        })?;
        Ok(id)
    }
    pub fn track_action(&mut self, id: &str, action: &str, value: &str) -> Result<(), String> {
        self.transact(|data| {
            let index = data.track_index(id)?;
            let count = data.tracks.len();
            let track = &mut data.tracks[index];
            match action {
                "visible" => track.visible = !track.visible,
                "lock" => track.locked = !track.locked,
                "rename" => { track.require_unlocked()?; track.name = value.trim().into(); }
                "delete" => {
                    track.require_unlocked()?;
                    if count == 1 || !track.clips.is_empty() {
                        return Err("Keep at least one video track and remove clips before deleting a track".into());
                    }
                    data.tracks.remove(index);
                }
                _ => return Err("Unknown track action".into()),
            }
            Ok(())
        })
    }
    pub fn reorder_track(&mut self, id: &str, anchor: &str, after: bool) -> Result<(), String> {
        self.transact(|data| {
            let index = data.track_index(id)?;
            data.tracks[index].require_unlocked()?;
            if id != anchor {
                let track = data.tracks.remove(index);
                let position = data.track_index(anchor)? + usize::from(after);
                data.tracks.insert(position, track);
            }
            Ok(())
        })
    }
    pub fn set_transform(&mut self, id: &str, transform: ClipTransform, revision: u64) -> Result<(), String> {
        self.transact_at(revision, |data| {
            let (track, index) = data.writable_clip(id)?;
            data.tracks[track].clips[index].transform = transform;
            Ok(())
        })
    }

    /// Keeps the right edge fixed; extension and contraction preserve exact source ticks.
    pub fn trim_left(&mut self, clip_id: &str, new_start: u64) -> Result<(), String> {
        self.transact(|data| {
            let (track, index) = data.writable_clip(clip_id)?;
            data.trim_video_start(track, index, new_start)?;
            data.tracks[track].clips.sort_by_key(|clip| clip.timeline_start);
            Ok(())
        })
    }
    /// Keeps the source in-point fixed; both directions must remain within the media range.
    pub fn trim_right(&mut self, clip_id: &str, new_end: u64) -> Result<(), String> {
        self.transact(|data| {
            let (track, index) = data.writable_clip(clip_id)?;
            let start = data.tracks[track].clips[index].timeline_start;
            if new_end <= start {
                return Err("Right trim must leave at least one frame".into());
            }
            data.tracks[track].clips[index].resize_right(new_end - start);
            Ok(())
        })
    }
    /// Changes placement, duration and source in-point atomically as one undoable edit.
    /// An unchanged integer source in-point retains its exact fractional tick.
    pub fn set_timing(
        &mut self,
        clip_id: &str,
        timeline_start: u64,
        duration: u64,
        source_in_pts: i64,
    ) -> Result<(), String> {
        self.transact(|data| {
            let (track, index) = data.writable_clip(clip_id)?;
            let clip = &mut data.tracks[track].clips[index];
            if source_in_pts != clip.source_in_pts {
                clip.source_subtick_numerator = 0;
                clip.source_subtick_denominator = 1;
            }
            clip.timeline_start = timeline_start;
            clip.resize_right(duration);
            clip.source_in_pts = source_in_pts;
            data.tracks[track].clips.sort_by_key(|clip| clip.timeline_start);
            Ok(())
        })
    }
    pub fn rename(&mut self, name: &str) -> Result<(), String> {
        self.transact(|data| {
            data.name = name.trim().to_owned();
            Ok(())
        })
    }
    /// Frame positions stay fixed; source ranges are revalidated at the new rate.
    pub fn set_sequence(&mut self, width: u32, height: u32, fps: Rational) -> Result<(), String> {
        self.transact(|data| {
            data.width = width;
            data.height = height;
            data.fps = Rational::new(fps.numerator, fps.denominator)?;
            Ok(())
        })
    }
    pub fn relink(&mut self, asset_id: &str, path: PathBuf) -> Result<(), String> {
        let asset = self
            .asset(asset_id)
            .cloned()
            .ok_or("Media asset does not exist")?;
        let canonical = path
            .canonicalize()
            .map_err(|error| format!("Cannot access relinked file: {error}"))?;
        if !canonical.is_file() {
            return Err("Relink target is not a regular file".into());
        }
        let fingerprint = fingerprint(&canonical)?;
        if fingerprint != asset.fingerprint {
            return Err("Relinked file does not match the saved asset fingerprint".into());
        }
        self.transact(|data| {
            let item = data
                .assets
                .iter_mut()
                .find(|item| item.id == asset_id)
                .ok_or("Media asset does not exist")?;
            item.path = canonical.to_string_lossy().into_owned();
            item.online = true;
            Ok(())
        })
    }
    pub fn undo(&mut self) -> bool {
        let Some(next_revision) = self.revision.checked_add(1) else {
            return false;
        };
        let Some(previous) = self.undo.pop_back() else {
            return false;
        };
        self.redo.push(std::mem::replace(&mut self.data, previous));
        self.revision = next_revision;
        true
    }
    pub fn redo(&mut self) -> bool {
        let Some(next_revision) = self.revision.checked_add(1) else {
            return false;
        };
        let Some(next) = self.redo.pop() else {
            return false;
        };
        if self.undo.len() == HISTORY_LIMIT {
            self.undo.pop_front();
        }
        self.undo.push_back(std::mem::replace(&mut self.data, next));
        self.revision = next_revision;
        true
    }
    fn transact(
        &mut self,
        change: impl FnOnce(&mut ProjectData) -> Result<(), String>,
    ) -> Result<(), String> {
        self.transact_at(self.revision, change)
    }
    /// Applies a command only to the revision it was prepared against, as one undo step.
    /// Errors, stale revisions, invalid candidates and unchanged edits never alter history.
    pub fn transact_at(
        &mut self,
        expected_revision: u64,
        change: impl FnOnce(&mut ProjectData) -> Result<(), String>,
    ) -> Result<(), String> {
        if expected_revision != self.revision {
            return Err("Project changed before this command could be applied".into());
        }
        let mut candidate = self.data.clone();
        change(&mut candidate)?;
        audio::validate_locked_links(&self.data, &candidate)?;
        validate(&candidate)?;
        if candidate == self.data {
            return Ok(());
        }
        let next_revision = self
            .revision
            .checked_add(1)
            .ok_or("Project revision exhausted")?;
        if self.undo.len() == HISTORY_LIMIT {
            self.undo.pop_front();
        }
        self.undo
            .push_back(std::mem::replace(&mut self.data, candidate));
        self.redo.clear();
        self.revision = next_revision;
        Ok(())
    }
    /// Atomically replaces a `.oqvideo.json` file, storing media paths relative to it.
    /// A failed save leaves the current path, saved content and edit history unchanged.
    pub fn save_as(&mut self, path: PathBuf) -> Result<(), String> {
        if !path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().ends_with(".oqvideo.json"))
        {
            return Err("Project filename must use the .oqvideo.json extension".into());
        }
        validate(&self.data)?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let parent = parent
            .canonicalize()
            .map_err(|error| format!("Cannot access project directory: {error}"))?;
        if !parent.is_dir() {
            return Err("Project directory does not exist".into());
        }
        let path = parent.join(path.file_name().ok_or("Project filename is missing")?);
        let mut snapshot = self.data.clone();
        for asset in &mut snapshot.assets {
            asset.path = stored_asset_path(&parent, Path::new(&asset.path));
        }
        let serialized = serde_json::to_vec_pretty(&ProjectFile {
            schema_version: SCHEMA_VERSION,
            data: snapshot,
        })
        .map_err(|error| error.to_string())?;
        if let Ok(previous) = fs::read(&path) {
            if previous != serialized && decode_project_file(&previous).is_ok() {
                write_project_atomic(&previous_project_path(&path), &previous, self.revision, true)?;
            }
        }
        write_project_atomic(&path, &serialized, self.revision, true)?;
        self.path = Some(path);
        self.saved_revision = self.revision;
        self.saved_data = self.data.clone();
        Ok(())
    }
    pub fn save(&mut self) -> Result<(), String> {
        let path = self
            .path
            .clone()
            .ok_or("Project has not been saved; use Save As")?;
        self.save_as(path)
    }
    pub(crate) fn write_recovery(data: ProjectData, path: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec(&ProjectFile { schema_version: SCHEMA_VERSION, data })
            .map_err(|error| error.to_string())?;
        write_project_atomic(path, &bytes, 0, true)
    }

    pub fn open_as_recovery(path: PathBuf) -> Result<Self, String> {
        let mut document = Self::open(path)?;
        document.path = None;
        document.saved_data = ProjectData::default();
        Ok(document)
    }
    pub fn open(path: PathBuf) -> Result<Self, String> {
        let path = path
            .canonicalize()
            .map_err(|error| format!("Cannot access project: {error}"))?;
        let bytes = fs::read(&path).map_err(|error| format!("Cannot read project: {error}"))?;
        let mut data = decode_project_file(&bytes)?;
        let parent = path.parent().unwrap_or(Path::new("."));
        for asset in &mut data.assets {
            let asset_path = PathBuf::from(&asset.path);
            let resolved = if asset_path.is_absolute() {
                asset_path
            } else {
                parent.join(asset_path)
            };
            asset.path = resolved
                .canonicalize()
                .unwrap_or(resolved)
                .to_string_lossy()
                .into_owned();
            asset.online = Path::new(&asset.path).is_file()
                && fingerprint(Path::new(&asset.path)).is_ok_and(|sum| sum == asset.fingerprint);
            #[cfg(target_os = "macos")]
            if asset.online && asset.has_audio && asset.audio.is_none() {
                asset.audio = crate::media::audio_metadata(Path::new(&asset.path)).ok().flatten();
            }
        }
        let mut project = Self::new(data)?;
        project.path = Some(path);
        project.saved_revision = 0;
        Ok(project)
    }
}

pub fn previous_project_path(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let name = name.strip_suffix(".oqvideo.json").unwrap_or(&name);
    path.with_file_name(format!(".{name}.previous.oqvideo.json"))
}

pub(crate) fn write_new_artifact(path: &Path, bytes: &[u8]) -> Result<(), String> {
    write_project_atomic(path, bytes, 0, false)
}

fn write_project_atomic(path: &Path, serialized: &[u8], revision: u64, replace: bool) -> Result<(), String> {
    let parent = path.parent().ok_or("Project directory is missing")?;
    let temp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        new_id("save", revision)
    ));
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|error| format!("Cannot reserve project save temporary: {error}"))?;
        if let Err(error) = file.write_all(serialized).and_then(|_| file.sync_all()) {
            let _ = fs::remove_file(&temp);
            return Err(format!("Cannot write project: {error}"));
        }
    }
    let installed = if replace {
        fs::rename(&temp, path)
    } else {
        fs::hard_link(&temp, path)
    };
    if !replace || installed.is_err() {
        let _ = fs::remove_file(&temp);
    }
    installed.map_err(|error| format!("Cannot commit file: {error}"))?;
    if let Ok(dir) = File::open(&parent) {
        let _ = dir.sync_all();
    }
    Ok(())
}

fn decode_project_file(bytes: &[u8]) -> Result<ProjectData, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Envelope {
        schema_version: u32,
        data: serde_json::Value,
    }
    let mut file: Envelope = serde_json::from_slice(bytes)
        .map_err(|error| format!("Cannot parse project file: {error}"))?;
    if !matches!(file.schema_version, 1..=SCHEMA_VERSION) {
        return Err(format!("Unsupported project schema {}", file.schema_version));
    }
    if file.data.get("schema_version").and_then(serde_json::Value::as_u64)
        != Some(u64::from(file.schema_version))
    {
        return Err("Project schema header and data disagree".into());
    }
    if file.schema_version <= 2 {
        let project = file.data.as_object_mut().ok_or("Cannot parse project data")?;
        if project.contains_key("tracks") { return Err("Legacy project already contains tracks".into()); }
        let mut clips = project.remove("clips").ok_or("Legacy project is missing clips")?;
        let legacy_clips = clips.as_array_mut().ok_or("Cannot parse project clips")?;
        for clip in &mut *legacy_clips {
            let object = clip.as_object_mut().ok_or("Cannot parse project clip")?;
            match (file.schema_version, object.contains_key("pipeline")) {
                (1, true) => return Err("Schema 1 clip contains unknown field `pipeline`".into()),
                (1, false) => {
                    object.insert("pipeline".into(), serde_json::to_value(ClipPipeline::default())
                        .map_err(|error| error.to_string())?);
                }
                (2, false) => return Err("Schema 2 clip is missing required field `pipeline`".into()),
                _ => {}
            }
        }
        legacy_clips.sort_by_key(|clip| clip.get("timeline_start").and_then(serde_json::Value::as_u64));
        project.insert("tracks".into(), serde_json::json!([{
            "id": DEFAULT_TRACK, "name": "Video 1", "visible": true, "locked": false, "clips": clips
        }]));
    }
    file.data["schema_version"] = serde_json::json!(SCHEMA_VERSION);
    serde_json::from_value(file.data).map_err(|error| format!("Cannot parse project file: {error}"))
}

fn validate(data: &ProjectData) -> Result<(), String> {
    if data.schema_version != SCHEMA_VERSION {
        return Err("Unsupported in-memory project schema".into());
    }
    if data.name.trim().is_empty() {
        return Err("Project name cannot be empty".into());
    }
    if data.width == 0 || data.height == 0 || data.width % 2 != 0 || data.height % 2 != 0 {
        return Err("Sequence dimensions must be positive even values".into());
    }
    Rational::new(data.fps.numerator, data.fps.denominator)?;
    let mut ids = std::collections::HashSet::new();
    for asset in &data.assets {
        if asset.id.is_empty() || !ids.insert(asset.id.as_str()) {
            return Err("Missing or duplicate media asset ID".into());
        }
        Rational::new(asset.time_base.numerator, asset.time_base.denominator)?;
        if let Some(fps) = asset.fps {
            Rational::new(fps.numerator, fps.denominator)?;
        }
        if asset.still && (!asset.has_visual() || asset.start_pts.is_some() || asset.duration_pts.is_some()
            || asset.fps.is_some() || asset.has_audio || asset.audio.is_some()) {
            return Err("Invalid still image asset metadata".into());
        }
        if asset.duration_pts.is_some_and(|duration| duration <= 0) {
            return Err("Media duration must be positive".into());
        }
        if let (Some(origin), Some(duration)) = (asset.start_pts, asset.duration_pts) {
            origin
                .checked_add(duration)
                .ok_or("Media timestamp range overflow")?;
        }
    }
    let mut clip_ids = std::collections::HashSet::new();
    let mut tracks = std::collections::HashSet::new();
    let mut has_subtitle_track = false;
    if data.tracks.is_empty() { return Err("A sequence needs at least one video track".into()); }
    for track in &data.tracks {
        if track.id.is_empty() || track.name.trim().is_empty() || !tracks.insert(&track.id) {
            return Err("Missing or duplicate video track ID or empty track name".into());
        }
        if track.subtitles && std::mem::replace(&mut has_subtitle_track, true) {
            return Err("A sequence can have one subtitle track".into());
        }
        let mut previous_end = 0;
        for clip in &track.clips {
            if clip.id.is_empty() || !clip_ids.insert(clip.id.as_str()) {
                return Err("Missing or duplicate clip ID".into());
            }
            clip.pipeline.validate().map_err(|error| format!("Clip {} pipeline: {error}", clip.id))?;
            clip.validate_opacity_keys()?;
            clip.validate_spatial()?;
            if clip.duration == 0 {
                return Err("Clip duration must be positive".into());
            }
            if !valid_speed(clip.speed_percent) || clip.source_span_remainder >= clip.speed_percent {
                return Err("Invalid clip speed or source span".into());
            }
            if clip.source_subtick_denominator == 0
                || clip.source_subtick_numerator >= clip.source_subtick_denominator
            {
                return Err("Invalid fractional source in-point".into());
            }
            let end = clip.timeline_end()?;
            if clip.timeline_start < previous_end {
                return Err("Video track clips cannot overlap".into());
            }
            previous_end = end;
            if track.subtitles != clip.title.as_ref().is_some_and(|title| title.subtitle) {
                return Err("Subtitle cues belong to the subtitle track".into());
            }
            if let Some(title) = &clip.title {
                title.validate()?;
                clip.validate_sequence_timing()?;
                if !clip.asset_id.is_empty() || clip.pipeline != ClipPipeline::default() {
                    return Err("Title clips use a generated image".into());
                }
            } else {
                let asset = data
                    .assets
                    .iter()
                    .find(|asset| asset.id == clip.asset_id)
                    .ok_or("Clip references a missing media asset")?;
                if asset.still {
                    clip.validate_sequence_timing()?;
                } else {
                    if !asset.has_visual() { return Err("Video clip references an audio-only asset".into()); }
                    let origin = asset
                        .start_pts
                        .ok_or("Precise placement requires a known source presentation timestamp")?;
                    let duration = asset
                        .duration_pts
                        .ok_or("Exact placement requires a known video duration")?;
                    let media_end = origin
                        .checked_add(duration)
                        .ok_or("Media timestamp range overflow")?;
                    let (source_end, remainder, _) = clip.source_out(data.fps, asset.time_base)?;
                    if clip.source_in_pts < origin
                        || source_end > media_end
                        || (source_end == media_end && remainder != 0)
                    {
                        return Err("Clip source range extends outside media presentation duration".into());
                    }
                }
            }
            let transform = clip.transform;
            if !transform.scale.is_finite()
                || !(0.1..=4.0).contains(&transform.scale)
                || !transform.x.is_finite()
                || !(-1.0..=1.0).contains(&transform.x)
                || !transform.y.is_finite()
                || !(-1.0..=1.0).contains(&transform.y)
                || !transform.rotation.is_finite()
                || !(-180.0..=180.0).contains(&transform.rotation)
                || !(0.0..=1.0).contains(&transform.opacity)
                || !transform.exposure.is_finite()
                || !(-8.0..=8.0).contains(&transform.exposure)
            {
                return Err("Invalid clip transform".into());
            }
        }
        transition::validate_dissolves(data, track)?;
    }
    timeline_tools::validate_markers(&data.markers)?;
    audio::validate_audio(data)
}
fn first_gap(clips: &[Clip], duration: u64) -> Result<u64, String> {
    let mut ordered = clips.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|clip| clip.timeline_start);
    let mut cursor = 0u64;
    for clip in ordered {
        let end = clip.timeline_end()?;
        if cursor
            .checked_add(duration)
            .is_some_and(|candidate| candidate <= clip.timeline_start)
        {
            return Ok(cursor);
        }
        cursor = end;
    }
    Ok(cursor)
}
fn overlaps(a0: u64, a1: u64, b0: u64, b1: u64) -> bool {
    a0 < b1 && b0 < a1
}
fn gcd128(mut a: i128, mut b: i128) -> i128 {
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    a.max(1)
}
fn shifted_source(
    clip: &Clip,
    frames: i128,
    fps: Rational,
    time_base: Rational,
) -> Result<(i64, u64, u64), String> {
    let units = frames.checked_mul(i128::from(clip.speed_percent)).ok_or("Source timestamp overflow")?;
    shifted_source_units(clip, units, fps, time_base)
}
fn shifted_source_units(
    clip: &Clip,
    units: i128,
    fps: Rational,
    time_base: Rational,
) -> Result<(i64, u64, u64), String> {
    Rational::new(fps.numerator, fps.denominator)?;
    Rational::new(time_base.numerator, time_base.denominator)?;
    if clip.source_subtick_denominator == 0
        || clip.source_subtick_numerator >= clip.source_subtick_denominator
    {
        return Err("Invalid fractional source in-point".into());
    }
    let mut frame_den = i128::from(fps.numerator) * i128::from(time_base.numerator) * 100;
    let mut frame_num = units.checked_mul(i128::from(fps.denominator))
        .and_then(|n| n.checked_mul(i128::from(time_base.denominator)))
        .ok_or("Source timestamp overflow")?;
    let divisor = gcd128(frame_num.rem_euclid(frame_den), frame_den);
    frame_num /= divisor;
    frame_den /= divisor;
    let old_den = i128::from(clip.source_subtick_denominator);
    let common = gcd128(frame_den, old_den);
    let denominator = old_den
        .checked_mul(frame_den / common)
        .ok_or("Source timestamp denominator overflow")?;
    let numerator = frame_num
        .checked_mul(old_den / common)
        .and_then(|v| {
            i128::from(clip.source_subtick_numerator)
                .checked_mul(frame_den / common)
                .and_then(|old| v.checked_add(old))
        })
        .ok_or("Source timestamp overflow")?;
    let ticks = i128::from(clip.source_in_pts)
        .checked_add(numerator.div_euclid(denominator))
        .ok_or("Source timestamp overflow")?;
    let remainder = numerator.rem_euclid(denominator);
    let divisor = gcd128(remainder, denominator);
    Ok((
        i64::try_from(ticks).map_err(|_| "Source timestamp exceeds i64")?,
        u64::try_from(remainder / divisor).map_err(|_| "Source remainder overflow")?,
        u64::try_from(denominator / divisor).map_err(|_| "Source denominator overflow")?,
    ))
}
fn advance_source(
    clip: &mut Clip,
    frames: i128,
    fps: Rational,
    time_base: Rational,
) -> Result<(), String> {
    let (ticks, numerator, denominator) = shifted_source(clip, frames, fps, time_base)?;
    clip.source_in_pts = ticks;
    clip.source_subtick_numerator = numerator;
    clip.source_subtick_denominator = denominator;
    Ok(())
}
fn duration_frames(ticks: i64, time_base: Rational, fps: Rational) -> Result<u64, String> {
    if ticks <= 0 {
        return Err("Media duration must be positive".into());
    }
    Rational::new(time_base.numerator, time_base.denominator)?;
    Rational::new(fps.numerator, fps.denominator)?;
    let numerator = i128::from(ticks)
        .checked_mul(i128::from(time_base.numerator))
        .and_then(|v| v.checked_mul(i128::from(fps.numerator)))
        .ok_or("Media duration overflow")?;
    let denominator = i128::from(time_base.denominator) * i128::from(fps.denominator);
    u64::try_from(numerator / denominator)
        .map_err(|_| "Media duration exceeds supported range".into())
}
/// Maps a half-open sequence-frame position to source-relative nanoseconds without tick rounding.
pub fn source_time_ns(
    asset: &MediaAsset,
    clip: &Clip,
    sequence_frame: u64,
    sequence_fps: Rational,
) -> Result<u64, String> {
    if sequence_frame < clip.timeline_start {
        return Err("Frame precedes selected clip".into());
    }
    let clip_end = clip.timeline_end()?;
    if sequence_frame >= clip_end {
        return Err("Frame follows selected clip".into());
    }
    extended_source_time_ns(asset, clip, sequence_frame, sequence_fps)
}

/// Samples the validated source handles of an active dissolve.
pub(super) fn extended_source_time_ns(
    asset: &MediaAsset,
    clip: &Clip,
    sequence_frame: u64,
    sequence_fps: Rational,
) -> Result<u64, String> {
    let offset = i128::from(sequence_frame) - i128::from(clip.timeline_start);
    let (ticks, numerator, denominator) = shifted_source(clip, offset, sequence_fps, asset.time_base)?;
    let relative = i128::from(ticks)
        - i128::from(
            asset
                .start_pts
                .ok_or("Precise media source origin is unknown")?,
        );
    let tick_numerator = relative
        .checked_mul(i128::from(denominator))
        .and_then(|v| v.checked_add(i128::from(numerator)))
        .ok_or("Source timestamp overflow")?;
    if tick_numerator < 0 {
        return Err("Source-relative position precedes media origin".into());
    }
    let ns = tick_numerator
        .checked_mul(i128::from(asset.time_base.numerator))
        .and_then(|v| v.checked_mul(1_000_000_000))
        .ok_or("Source timestamp overflow")?
        / (i128::from(denominator) * i128::from(asset.time_base.denominator));
    u64::try_from(ns).map_err(|_| "Source-relative position is outside supported range".into())
}
fn stored_asset_path(project_dir: &Path, path: &Path) -> String {
    if !path.is_absolute() {
        return path.to_string_lossy().into_owned();
    }
    let base = project_dir.components().collect::<Vec<_>>();
    let target = path.components().collect::<Vec<_>>();
    let shared = base.iter().zip(&target).take_while(|(a, b)| a == b).count();
    if shared == 0 {
        return path.to_string_lossy().into_owned();
    }
    let mut relative = PathBuf::new();
    for _ in shared..base.len() {
        relative.push("..");
    }
    for component in &target[shared..] {
        relative.push(component.as_os_str());
    }
    relative.to_string_lossy().into_owned()
}
pub fn fingerprint(path: &Path) -> Result<String, String> {
    fingerprint_until(path, || false)
}
pub fn fingerprint_until(path: &Path, cancelled: impl Fn() -> bool) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("Cannot open media file: {error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 128 * 1024];
    loop {
        if cancelled() {
            return Err("Media fingerprint interrupted".into());
        }
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("Cannot fingerprint media: {error}"))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
pub fn new_id(kind: &str, revision: u64) -> String {
    format!(
        "{kind}-{}-{revision}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn still_image_edits_only_affect_sequence_time() {
        let mut doc = EditorDocument::default();
        let mut still = asset(Path::new("photo.png"));
        still.still = true;
        still.duration_pts = None;
        still.start_pts = None;
        still.fps = None;
        still.has_audio = false;
        still.audio = None;
        let asset_id = still.id.clone();
        doc.add_asset(still).unwrap();
        let clip_id = doc.add_clip(&asset_id).unwrap();
        assert_eq!(doc.clip(&clip_id).unwrap().duration, 90);
        doc.trim_left(&clip_id, 12).unwrap();
        doc.trim_right(&clip_id, 102).unwrap();
        let right = doc.split(&clip_id, 50).unwrap();
        for (id, start, end) in [(&clip_id, 12, 50), (&right, 50, 102)] {
            let clip = doc.clip(id).unwrap();
            assert_eq!(clip.timeline_start, start);
            assert_eq!(clip.timeline_end(), Ok(end));
            assert_eq!(clip.asset_id, asset_id);
            assert_eq!(clip.source_in_pts, 0);
        }
        assert!(doc.set_speed(&right, 200).is_err());
        assert_eq!(doc.clip(&right).unwrap().duration, 52);
    }

    fn asset(path: &Path) -> MediaAsset {
        MediaAsset {
            id: "asset-1".into(),
            name: "clip.mp4".into(),
            path: path.to_string_lossy().into_owned(),
            fingerprint: "sum".into(),
            still: false,
            size_bytes: 1,
            width: 64,
            height: 64,
            duration_pts: Some(90),
            start_pts: Some(9000),
            time_base: Rational {
                numerator: 1,
                denominator: 30,
            },
            fps: Some(Rational {
                numerator: 30,
                denominator: 1,
            }),
            has_audio: false,
            audio: None,
            online: true,
        }
    }
    fn project() -> EditorDocument {
        EditorDocument::new(ProjectData::default()).unwrap()
    }
    fn insert_asset(project: &mut EditorDocument, path: &Path) {
        project.add_asset(asset(path)).unwrap();
    }
    #[test]
    fn rational_fps_reduces_and_maps_boundaries_without_cumulative_rounding() {
        let fps = Rational::new(60000, 2002).unwrap();
        assert_eq!(
            fps,
            Rational {
                numerator: 30000,
                denominator: 1001
            }
        );
        assert_eq!(fps.frames_to_ns(30000).unwrap(), 1_001_000_000_000);
        assert_eq!(fps.ns_to_frames_floor(1_001_000_000_000).unwrap(), 30000);
        assert!(Rational::new(0, 1).is_err());
    }
    #[test]
    fn dissolve_uses_extended_handles_without_moving_the_cut_and_rolls_atomically() {
        let mut doc = project();
        insert_asset(&mut doc, Path::new("movie.mp4"));
        let outgoing = doc.add_clip("asset-1").unwrap();
        doc.trim_right(&outgoing, 12).unwrap();
        let incoming = doc.add_clip("asset-1").unwrap();
        doc.set_timing(&incoming, 12, 12, 9030).unwrap();
        doc.set_dissolve(&outgoing, 4).unwrap();
        assert_eq!(doc.data.tracks[0].next_video_boundary(9), Some(10));
        assert_eq!(doc.data.tracks[0].next_video_boundary(10), Some(14));
        assert_eq!(doc.data.tracks[0].next_video_boundary(12), Some(14));
        let active: Vec<_> = doc.data.video_inputs(11).collect();
        assert_eq!(active.len(), 2);
        assert_eq!(active[0].clip.id, outgoing);
        assert_eq!(active[1].clip.id, incoming);
        let asset = doc.asset("asset-1").unwrap();
        assert_eq!(extended_source_time_ns(asset, active[1].clip, 11, doc.data.fps), Ok(966_666_666));
        assert_eq!(doc.clip(&incoming).unwrap().timeline_start, 12);

        let old = doc.data.clone();
        let revision = doc.revision;
        assert!(doc.set_timing(&incoming, 12, 12, 9000).is_err());
        assert_eq!(doc.data, old);
        assert_eq!(doc.revision, revision);
        doc.roll_end(&outgoing, 13).unwrap();
        assert_eq!(doc.data.tracks[0].next_video_boundary(10), Some(11));
        assert_eq!(doc.data.video_inputs(14).count(), 2);
        assert_eq!(doc.data.video_inputs(15).count(), 1);
        doc.lift(&incoming).unwrap();
        assert!(doc.data.tracks[0].dissolves.is_empty());
        assert!(doc.undo());
        assert_eq!(doc.data.video_inputs(13).count(), 2);
    }
    #[test]
    fn same_track_dissolves_can_touch_but_cannot_overlap() {
        let mut doc = project();
        insert_asset(&mut doc, Path::new("movie.mp4"));
        let a = doc.add_clip("asset-1").unwrap();
        doc.trim_right(&a, 8).unwrap();
        let b = doc.add_clip("asset-1").unwrap();
        doc.set_timing(&b, 8, 4, 9030).unwrap();
        let c = doc.add_clip("asset-1").unwrap();
        doc.set_timing(&c, 12, 8, 9050).unwrap();
        doc.set_dissolve(&a, 6).unwrap();
        let revision = doc.revision;
        let snapshot = doc.data.clone();
        assert_eq!(doc.set_dissolve(&b, 6), Err("Dissolves cannot overlap on the same track".into()));
        assert_eq!(doc.revision, revision);
        assert_eq!(doc.data, snapshot);
        doc.set_dissolve(&b, 2).unwrap();
        assert_eq!(doc.data.tracks[0].next_video_boundary(10), Some(11));
        assert_eq!(doc.data.tracks[0].next_video_boundary(11), Some(13));
        let inputs: Vec<_> = doc.data.video_inputs(11).map(|input| input.clip.id.as_str()).collect();
        assert_eq!(inputs, [b.as_str(), c.as_str()]);
    }
    #[test]
    fn retime_preserves_source_span_and_split_samples_across_rate_rounding() {
        let mut doc = project();
        insert_asset(&mut doc, Path::new("movie.mp4"));
        let id = doc.add_clip("asset-1").unwrap();
        doc.set_speed(&id, 400).unwrap();
        let clip = doc.clip(&id).unwrap();
        let asset = doc.asset("asset-1").unwrap();
        assert_eq!((clip.duration, clip.source_span_remainder), (22, 200));
        assert_eq!(clip.source_out(doc.data.fps, asset.time_base), Ok((9090, 0, 1)));
        assert_eq!(doc.data.clip_source_time(clip, 1), Ok(133_333_333));
        doc.set_speed(&id, 100).unwrap();
        assert_eq!(doc.clip(&id).unwrap().duration, 90);
        doc.set_speed(&id, 125).unwrap();
        let tail = doc.split(&id, 1).unwrap();
        let clip = doc.clip(&tail).unwrap();
        assert_eq!((clip.source_in_pts, clip.source_subtick_numerator, clip.source_subtick_denominator), (9001, 1, 4));
        assert_eq!(clip.speed_percent, 125);
        assert_eq!(doc.data.clip_source_time(clip, 1), Ok(41_666_666));
        assert_eq!(clip.source_out(doc.data.fps, doc.asset("asset-1").unwrap().time_base), Ok((9090, 0, 1)));
    }
    #[test]
    fn overwrite_only_replaces_the_target_interval_across_multiple_clips() {
        let mut doc = project();
        insert_asset(&mut doc, Path::new("movie.mp4"));
        let overlay = doc.add_track(DEFAULT_TRACK).unwrap();
        let other = doc.place_clip("asset-1", &overlay, Some(0)).unwrap();
        let first = doc.place_clip("asset-1", DEFAULT_TRACK, Some(0)).unwrap();
        let second = doc.place_clip("asset-1", DEFAULT_TRACK, Some(90)).unwrap();
        let snapshot = doc.data.clone();

        let replacement = doc.place_visual("asset-1", DEFAULT_TRACK, Placement::Overwrite(45)).unwrap();
        let lane = &doc.data.tracks[doc.data.track_index(DEFAULT_TRACK).unwrap()];
        assert_eq!(lane.clips.len(), 3);
        assert_eq!(lane.clips[0].id, first);
        assert_eq!(lane.clips[0].timeline_end(), Ok(45));
        assert_eq!(lane.clips[1].id, replacement);
        assert_eq!(lane.clips[1].timeline_end(), Ok(135));
        assert_eq!(lane.clips[2].timeline_start, 135);
        assert_eq!(lane.clips[2].source_in_pts, 9045);
        assert_eq!(lane.clips[2].duration, 45);
        assert!(doc.clip(&second).is_none());
        assert_eq!(doc.clip(&other), snapshot.tracks[0].clips.first());
        assert!(doc.undo());
        assert_eq!(doc.data, snapshot);
    }
    #[test]
    fn split_preserves_total_duration_source_in_and_half_open_frame_ownership() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("movie");
        fs::write(&path, b"x").unwrap();
        let mut p = project();
        insert_asset(&mut p, &path);
        let id = p.add_clip("asset-1").unwrap();
        let right = p.split(&id, 45).unwrap();
        let left = p.clip(&id).unwrap();
        let right_clip = p.clip(&right).unwrap();
        assert_eq!(left.duration + right_clip.duration, 90);
        assert_eq!(right_clip.timeline_start, 45);
        assert_eq!(right_clip.source_in_pts, 9045);
        assert_eq!(p.clip_at(44).unwrap().id, id);
        assert_eq!(p.clip_at(45).unwrap().id, right);
    }
    #[test]
    fn overlap_move_and_invalid_trim_reject_without_revision_or_partial_mutation() {
        let mut p = project();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("movie");
        fs::write(&path, b"x").unwrap();
        insert_asset(&mut p, &path);
        let a = p.add_clip("asset-1").unwrap();
        let mut second = p.clip(&a).unwrap().clone();
        second.id = "clip-second".into();
        second.timeline_start = 100;
        second.duration = 20;
        p.transact(|data| {
            data.tracks[0].clips.push(second);
            Ok(())
        })
        .unwrap();
        let before = p.data.clone();
        let revision = p.revision;
        assert!(p.move_clip(&a, 110).is_err());
        assert!(p.trim_left(&a, 100).is_err());
        assert_eq!(p.data, before);
        assert_eq!(p.revision, revision);
    }
    #[test]
    fn lift_preserves_gap_and_undo_redo_restore_exact_state() {
        let mut p = project();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("movie");
        fs::write(&path, b"x").unwrap();
        insert_asset(&mut p, &path);
        let id = p.add_clip("asset-1").unwrap();
        p.lift(&id).unwrap();
        assert_eq!(p.duration_frames(), 0);
        assert!(p.undo());
        assert!(p.clip(&id).is_some());
        assert!(p.redo());
        assert!(p.clip(&id).is_none());
    }
    #[test]
    fn save_reopen_relinks_only_matching_content_and_preserves_project_on_failed_save() {
        let dir = tempfile::tempdir().unwrap();
        let media_dir = dir.path().join("media");
        fs::create_dir(&media_dir).unwrap();
        let media = media_dir.join("clip.mp4");
        fs::write(&media, b"immutable media").unwrap();
        let project_path = dir.path().join("edit.oqvideo.json");
        let mut p = project();
        let mut item = asset(&media);
        item.fingerprint = fingerprint(&media).unwrap();
        p.add_asset(item).unwrap();
        p.save_as(project_path.clone()).unwrap();
        let missing_dir = dir.path().join("moved");
        fs::create_dir(&missing_dir).unwrap();
        fs::rename(&media, missing_dir.join("clip.mp4")).unwrap();
        let mut opened = EditorDocument::open(project_path.clone()).unwrap();
        assert!(!opened.data.assets[0].online);
        assert!(opened
            .relink("asset-1", missing_dir.join("clip.mp4"))
            .is_ok());
        let changed = dir.path().join("different.mp4");
        fs::write(&changed, b"different bytes").unwrap();
        let before = opened.data.clone();
        let revision = opened.revision;
        assert!(opened.relink("asset-1", changed).is_err());
        assert_eq!(opened.data, before);
        assert_eq!(opened.revision, revision);
        let original = fs::read(&project_path).unwrap();
        let saved_revision = opened.saved_revision;
        let saved_path = opened.path.clone();
        assert!(opened
            .save_as(dir.path().join("missing/out.oqvideo.json"))
            .is_err());
        assert_eq!(fs::read(&project_path).unwrap(), original);
        assert_eq!(opened.path, saved_path);
        assert_eq!(opened.saved_revision, saved_revision);
        assert!(opened.is_dirty());
    }
    #[test]
    fn dirty_state_tracks_saved_content_through_undo_redo_and_rejects_unknown_schema() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("edit.oqvideo.json");
        let mut p = project();
        p.rename("Changed").unwrap();
        assert!(p.is_dirty());
        p.save_as(file.clone()).unwrap();
        assert!(!p.is_dirty());
        p.rename("After save").unwrap();
        assert!(p.is_dirty());
        assert!(p.undo());
        assert!(!p.is_dirty());
        assert!(p.redo());
        assert!(p.is_dirty());
        assert!(p.undo());
        assert!(!p.is_dirty());
        p.rename("Changed").unwrap();
        assert!(p.can_redo());
        let mut unsupported = ProjectFile {
            schema_version: 99,
            data: p.data.clone(),
        };
        fs::write(&file, serde_json::to_vec(&unsupported).unwrap()).unwrap();
        assert!(EditorDocument::open(file.clone())
            .err()
            .unwrap()
            .contains("Unsupported project schema"));
        unsupported.schema_version = SCHEMA_VERSION;
        unsupported.data.schema_version = 99;
        fs::write(&file, serde_json::to_vec(&unsupported).unwrap()).unwrap();
        assert!(EditorDocument::open(file.clone()).is_err());
        let mut unknown = serde_json::to_value(ProjectFile {
            schema_version: SCHEMA_VERSION,
            data: project().data,
        })
        .unwrap();
        unknown["data"]["tracks"] = serde_json::json!([{"kind":"audio"}]);
        fs::write(&file, serde_json::to_vec(&unknown).unwrap()).unwrap();
        assert!(EditorDocument::open(file)
            .err()
            .unwrap()
            .contains("unknown field"));
    }
    #[test]
    fn timing_is_one_validated_command_with_no_partial_commits() {
        let mut p = project();
        insert_asset(&mut p, Path::new("movie"));
        let id = p.add_clip("asset-1").unwrap();
        let original = p.data.clone();
        let revision = p.revision;
        p.set_timing(&id, 100, 10, 9080).unwrap();
        assert_eq!(p.revision, revision + 1);
        let clip = p.clip(&id).unwrap();
        assert_eq!(
            (clip.timeline_start, clip.duration, clip.source_in_pts),
            (100, 10, 9080)
        );
        assert!(p.undo());
        assert_eq!(p.data, original);
        assert!(p.can_redo());
        let revision = p.revision;
        let history = p.undo.len();
        assert!(p.set_timing(&id, 100, 11, 9080).is_err());
        assert!(p.set_timing(&id, u64::MAX, 1, 9000).is_err());
        assert!(p.set_timing(&id, 0, 0, 9000).is_err());
        assert!(p.split(&id, 0).is_err());
        assert!(p.split(&id, 90).is_err());
        assert!(p.trim_left(&id, 90).is_err());
        assert!(p.trim_right(&id, 0).is_err());
        assert!(p.trim_right(&id, 91).is_err());
        assert_eq!(p.data, original);
        assert_eq!(p.revision, revision);
        assert_eq!(p.undo.len(), history);
        assert!(p.can_redo());
        assert!(p.redo());
        assert_eq!(p.clip(&id).unwrap().source_in_pts, 9080);
    }
    #[test]
    fn command_failures_and_stale_revisions_leave_history_and_data_unchanged() {
        let mut p = project();
        p.rename("First").unwrap();
        p.rename("Second").unwrap();
        assert!(p.undo());
        let before = p.data.clone();
        let revision = p.revision;
        let history = p.undo.len();
        let mut called = false;
        assert!(p
            .transact_at(revision - 1, |data| {
                called = true;
                data.name = "Stale".into();
                Ok(())
            })
            .is_err());
        assert!(!called);
        assert!(p
            .transact_at(revision, |data| {
                data.name = "Partial".into();
                Err("Command failed".into())
            })
            .is_err());
        assert!(p
            .transact_at(revision, |data| {
                data.width = 0;
                data.name = "Invalid".into();
                Ok(())
            })
            .is_err());
        p.transact_at(revision, |_| Ok(())).unwrap();
        assert_eq!(p.data, before);
        assert_eq!(p.revision, revision);
        assert_eq!(p.undo.len(), history);
        assert!(p.can_redo());
        p.transact_at(revision, |data| {
            data.name = "Combined".into();
            data.width = 1280;
            data.height = 720;
            Ok(())
        })
        .unwrap();
        assert_eq!(p.revision, revision + 1);
        assert!(!p.can_redo());
        assert!(p.undo());
        assert_eq!(p.data, before);
    }
    #[test]
    fn fractional_splits_preserve_exact_ticks_and_preview_time() {
        let mut p = project();
        p.set_sequence(1920, 1080, Rational::new(30000, 1001).unwrap())
            .unwrap();
        insert_asset(&mut p, Path::new("movie"));
        let id = p.add_clip("asset-1").unwrap();
        let original = p.clip(&id).unwrap().clone();
        assert_eq!(original.duration, 89);
        let right = p.split(&id, 1).unwrap();
        let next = p.split(&right, 2).unwrap();
        let middle = p.clip(&right).unwrap();
        assert_eq!(middle.source_in_pts, 9001);
        assert_eq!(
            (
                middle.source_subtick_numerator,
                middle.source_subtick_denominator
            ),
            (1, 1000)
        );
        let tail = p.clip(&next).unwrap();
        assert_eq!(tail.source_in_pts, 9002);
        assert_eq!(
            (
                tail.source_subtick_numerator,
                tail.source_subtick_denominator
            ),
            (1, 500)
        );
        let media = p.asset("asset-1").unwrap();
        let fps = p.data.fps;
        assert_eq!(source_time_ns(media, tail, 2, fps).unwrap(), 66_733_333);
        assert_eq!(
            source_time_ns(media, tail, 2, fps).unwrap(),
            source_time_ns(media, &original, 2, fps).unwrap()
        );
        assert!(source_time_ns(media, tail, 1, fps).is_err());
        assert!(source_time_ns(media, tail, 89, fps).is_err());
        let split_state = p.data.clone();
        assert!(p.undo());
        assert!(p.redo());
        assert_eq!(p.data, split_state);
        p.set_timing(&next, 20, 10, 9002).unwrap();
        let tail = p.clip(&next).unwrap();
        assert_eq!(
            (
                tail.source_subtick_numerator,
                tail.source_subtick_denominator
            ),
            (1, 500)
        );
        p.set_timing(&next, 20, 10, 9003).unwrap();
        let tail = p.clip(&next).unwrap();
        assert_eq!(
            (
                tail.source_subtick_numerator,
                tail.source_subtick_denominator
            ),
            (0, 1)
        );
    }
    #[test]
    fn trims_extend_and_contract_without_fractional_drift_or_source_overrun() {
        let mut p = project();
        p.set_sequence(1920, 1080, Rational::new(30000, 1001).unwrap())
            .unwrap();
        insert_asset(&mut p, Path::new("movie"));
        let id = p.add_clip("asset-1").unwrap();
        p.move_clip(&id, 10).unwrap();
        let original = p.clip(&id).unwrap().clone();
        for _ in 0..5 {
            p.trim_left(&id, 11).unwrap();
            let clip = p.clip(&id).unwrap();
            assert_eq!(
                (
                    clip.source_in_pts,
                    clip.source_subtick_numerator,
                    clip.source_subtick_denominator
                ),
                (9001, 1, 1000)
            );
            p.trim_left(&id, 10).unwrap();
            assert_eq!(p.clip(&id).unwrap(), &original);
        }
        let before = p.data.clone();
        let revision = p.revision;
        assert!(p.trim_left(&id, 9).is_err());
        assert!(p.trim_right(&id, 100).is_err());
        assert_eq!(p.data, before);
        assert_eq!(p.revision, revision);
        p.trim_right(&id, 98).unwrap();
        p.trim_right(&id, 99).unwrap();
        assert_eq!(p.clip(&id).unwrap(), &original);
    }
    #[test]
    fn sequence_and_name_edits_validate_the_entire_candidate() {
        let mut p = project();
        insert_asset(&mut p, Path::new("movie"));
        p.add_clip("asset-1").unwrap();
        let before = p.data.clone();
        let revision = p.revision;
        assert!(p.rename("   ").is_err());
        assert!(p
            .set_sequence(1279, 720, Rational::new(30, 1).unwrap())
            .is_err());
        assert!(p
            .set_sequence(1280, 720, Rational::new(15, 1).unwrap())
            .is_err());
        assert_eq!(p.data, before);
        assert_eq!(p.revision, revision);
        p.set_sequence(1280, 720, Rational::new(60, 1).unwrap())
            .unwrap();
        assert_eq!(p.data.width, 1280);
        assert_eq!(p.data.tracks[0].clips, before.tracks[0].clips);
        assert!(p.undo());
        assert_eq!(p.data, before);
        p.rename("  Renamed  ").unwrap();
        assert_eq!(p.data.name, "Renamed");
    }
    #[test]
    fn bounded_history_and_revision_exhaustion_never_partially_mutate() {
        let mut p = project();
        for index in 0..HISTORY_LIMIT + 5 {
            p.rename(&format!("Edit {index}")).unwrap();
        }
        let mut count = 0;
        while p.undo() {
            count += 1;
        }
        assert_eq!(count, HISTORY_LIMIT);
        assert!(p.can_redo());
        let before = p.data.clone();
        p.revision = u64::MAX;
        assert!(!p.undo());
        assert!(!p.redo());
        assert!(p.rename("Overflow").is_err());
        assert_eq!(p.data, before);
        assert!(p.can_redo());
    }
    #[test]
    fn saved_relative_media_and_fractions_survive_bundle_relocation() {
        let root = tempfile::tempdir().unwrap();
        let bundle = root.path().join("bundle");
        fs::create_dir_all(bundle.join("media")).unwrap();
        let media = bundle.join("media/clip.mp4");
        fs::write(&media, b"original media").unwrap();
        let mut item = asset(&media.canonicalize().unwrap());
        item.fingerprint = fingerprint(&media).unwrap();
        let mut p = project();
        p.set_sequence(1920, 1080, Rational::new(30000, 1001).unwrap())
            .unwrap();
        p.add_asset(item).unwrap();
        let id = p.add_clip("asset-1").unwrap();
        p.split(&id, 1).unwrap();
        p.save_as(bundle.join("edit.oqvideo.json")).unwrap();
        let saved: ProjectFile =
            serde_json::from_slice(&fs::read(bundle.join("edit.oqvideo.json")).unwrap()).unwrap();
        assert_eq!(saved.data.assets[0].path, "media/clip.mp4");
        let relocated = root.path().join("relocated");
        fs::rename(&bundle, &relocated).unwrap();
        let opened = EditorDocument::open(relocated.join("edit.oqvideo.json")).unwrap();
        assert!(opened.data.assets[0].online);
        assert!(!opened.is_dirty());
        assert_eq!(opened.data.tracks[0].clips, p.data.tracks[0].clips);
        assert_eq!(
            Path::new(&opened.data.assets[0].path),
            relocated.join("media/clip.mp4").canonicalize().unwrap()
        );
        fs::write(relocated.join("media/clip.mp4"), b"changed content").unwrap();
        let opened = EditorDocument::open(relocated.join("edit.oqvideo.json")).unwrap();
        assert!(!opened.data.assets[0].online);
    }
    #[test]
    fn sibling_media_paths_remain_relative_after_save_as() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("projects")).unwrap();
        fs::create_dir(root.path().join("media")).unwrap();
        let media = root.path().join("media/clip.mp4");
        fs::write(&media, b"media").unwrap();
        let mut item = asset(&media.canonicalize().unwrap());
        item.fingerprint = fingerprint(&media).unwrap();
        let mut p = project();
        p.add_asset(item).unwrap();
        let path = root.path().join("projects/edit.oqvideo.json");
        p.save_as(path.clone()).unwrap();
        let file: ProjectFile = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(file.data.assets[0].path, "../media/clip.mp4");
        assert!(EditorDocument::open(path).unwrap().data.assets[0].online);
    }
    #[test]
    fn failed_save_commit_removes_temporary_and_preserves_saved_state() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("edit.oqvideo.json");
        let mut p = project();
        assert!(p.save().is_err());
        assert!(p.save_as(root.path().join("wrong.oqvideo")).is_err());
        p.save_as(path.clone()).unwrap();
        let original = fs::read(&path).unwrap();
        let saved_path = p.path.clone();
        let saved_revision = p.saved_revision;
        let saved_data = p.saved_data.clone();
        p.rename("Unsaved").unwrap();
        let revision = p.revision;
        let history = p.undo.len();
        let blocked = root.path().join("blocked.oqvideo.json");
        fs::create_dir(&blocked).unwrap();
        assert!(p.save_as(blocked).is_err());
        assert_eq!(p.path, saved_path);
        assert_eq!(p.saved_revision, saved_revision);
        assert_eq!(p.saved_data, saved_data);
        assert_eq!(p.revision, revision);
        assert_eq!(p.undo.len(), history);
        assert!(p.is_dirty());
        assert_eq!(fs::read(path).unwrap(), original);
        assert!(fs::read_dir(root.path()).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));
    }
}

#[cfg(test)]
#[path = "document_pipeline_tests.rs"]
mod pipeline_tests;
