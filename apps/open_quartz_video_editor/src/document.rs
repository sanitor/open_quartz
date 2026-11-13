use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const SCHEMA_VERSION: u32 = 1;
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
    pub width: u32,
    pub height: u32,
    pub duration_pts: Option<i64>,
    pub start_pts: Option<i64>,
    pub time_base: Rational,
    pub fps: Option<Rational>,
    pub has_audio: bool,
    pub online: bool,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ClipTransform {
    pub scale: f32,
    pub x: f32,
    pub y: f32,
    pub exposure: f32,
}
impl Default for ClipTransform {
    fn default() -> Self {
        Self {
            scale: 1.0,
            x: 0.0,
            y: 0.0,
            exposure: 0.0,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Clip {
    pub id: String,
    pub asset_id: String,
    pub timeline_start: u64,
    pub duration: u64,
    pub source_in_pts: i64,
    /// Exact fractional source tick, always in [0, 1); retained across splits and trims.
    #[serde(default)]
    pub source_subtick_numerator: u64,
    #[serde(default = "one")]
    pub source_subtick_denominator: u64,
    pub transform: ClipTransform,
}
impl Clip {
    pub fn timeline_end(&self) -> Result<u64, String> {
        self.timeline_start
            .checked_add(self.duration)
            .ok_or_else(|| "Clip end frame overflow".into())
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
    pub clips: Vec<Clip>,
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
            clips: Vec::new(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectFile {
    schema_version: u32,
    data: ProjectData,
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
        self.data.clips.iter().find(|clip| clip.id == id)
    }
    pub fn clip_at(&self, frame: u64) -> Option<&Clip> {
        self.data.clips.iter().find(|clip| {
            clip.timeline_start <= frame && clip.timeline_end().is_ok_and(|end| frame < end)
        })
    }
    pub fn duration_frames(&self) -> u64 {
        self.data
            .clips
            .iter()
            .filter_map(|clip| clip.timeline_end().ok())
            .max()
            .unwrap_or(0)
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
    pub fn add_clip(&mut self, asset_id: &str) -> Result<String, String> {
        let asset = self
            .asset(asset_id)
            .cloned()
            .ok_or("Media asset does not exist")?;
        let ticks = asset
            .duration_pts
            .ok_or("Exact placement requires a known video duration")?;
        let duration = duration_frames(ticks, asset.time_base, self.data.fps)?;
        if duration == 0 {
            return Err("Media is shorter than one sequence frame".into());
        }
        let start = first_gap(&self.data.clips, duration)?;
        let id = new_id("clip", self.revision);
        let source_in = asset
            .start_pts
            .ok_or("Precise placement requires a known source presentation timestamp")?;
        let clip = Clip {
            id: id.clone(),
            asset_id: asset.id,
            timeline_start: start,
            duration,
            source_in_pts: source_in,
            source_subtick_numerator: 0,
            source_subtick_denominator: 1,
            transform: ClipTransform::default(),
        };
        self.transact(|data| {
            data.clips.push(clip);
            data.clips.sort_by_key(|clip| clip.timeline_start);
            Ok(())
        })?;
        Ok(id)
    }
    pub fn split(&mut self, clip_id: &str, at: u64) -> Result<String, String> {
        let right_id = new_id("clip", self.revision);
        self.transact(|data| {
            let index = data
                .clips
                .iter()
                .position(|clip| clip.id == clip_id)
                .ok_or("Selected clip no longer exists")?;
            let mut right = data.clips[index].clone();
            let end = right.timeline_end()?;
            if at <= right.timeline_start || at >= end {
                return Err("Split point must be inside the selected clip".into());
            }
            let left_duration = at - right.timeline_start;
            let time_base = data
                .assets
                .iter()
                .find(|asset| asset.id == right.asset_id)
                .ok_or("Clip media is missing")?
                .time_base;
            advance_source(&mut right, i128::from(left_duration), data.fps, time_base)?;
            right.id = right_id.clone();
            right.timeline_start = at;
            right.duration = end - at;
            data.clips[index].duration = left_duration;
            data.clips.insert(index + 1, right);
            Ok(())
        })?;
        Ok(right_id)
    }
    pub fn lift(&mut self, clip_id: &str) -> Result<(), String> {
        self.transact(|data| {
            let before = data.clips.len();
            data.clips.retain(|clip| clip.id != clip_id);
            if data.clips.len() == before {
                return Err("Selected clip no longer exists".into());
            }
            Ok(())
        })
    }
    pub fn move_clip(&mut self, clip_id: &str, target: u64) -> Result<(), String> {
        self.transact(|data| {
            let index = data
                .clips
                .iter()
                .position(|clip| clip.id == clip_id)
                .ok_or("Selected clip no longer exists")?;
            let duration = data.clips[index].duration;
            let end = target
                .checked_add(duration)
                .ok_or("Clip end frame overflow")?;
            if data.clips.iter().enumerate().any(|(other_index, other)| {
                other_index != index
                    && overlaps(
                        target,
                        end,
                        other.timeline_start,
                        other.timeline_end().unwrap_or(u64::MAX),
                    )
            }) {
                return Err("Clip overlaps another clip on the video track".into());
            }
            data.clips[index].timeline_start = target;
            data.clips.sort_by_key(|clip| clip.timeline_start);
            Ok(())
        })
    }
    /// Keeps the right edge fixed; extension and contraction preserve exact source ticks.
    pub fn trim_left(&mut self, clip_id: &str, new_start: u64) -> Result<(), String> {
        self.transact(|data| {
            let index = data
                .clips
                .iter()
                .position(|clip| clip.id == clip_id)
                .ok_or("Selected clip no longer exists")?;
            let old = data.clips[index].clone();
            let end = old.timeline_end()?;
            if new_start >= end {
                return Err("Left trim must leave at least one frame".into());
            }
            let asset = data
                .assets
                .iter()
                .find(|asset| asset.id == old.asset_id)
                .ok_or("Clip media is missing")?;
            advance_source(
                &mut data.clips[index],
                i128::from(new_start) - i128::from(old.timeline_start),
                data.fps,
                asset.time_base,
            )?;
            data.clips[index].timeline_start = new_start;
            data.clips[index].duration = end - new_start;
            Ok(())
        })
    }
    /// Keeps the source in-point fixed; both directions must remain within the media range.
    pub fn trim_right(&mut self, clip_id: &str, new_end: u64) -> Result<(), String> {
        self.transact(|data| {
            let index = data
                .clips
                .iter()
                .position(|clip| clip.id == clip_id)
                .ok_or("Selected clip no longer exists")?;
            let start = data.clips[index].timeline_start;
            if new_end <= start {
                return Err("Right trim must leave at least one frame".into());
            }
            data.clips[index].duration = new_end - start;
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
            let clip = data
                .clips
                .iter_mut()
                .find(|clip| clip.id == clip_id)
                .ok_or("Selected clip no longer exists")?;
            if source_in_pts != clip.source_in_pts {
                clip.source_subtick_numerator = 0;
                clip.source_subtick_denominator = 1;
            }
            clip.timeline_start = timeline_start;
            clip.duration = duration;
            clip.source_in_pts = source_in_pts;
            data.clips.sort_by_key(|clip| clip.timeline_start);
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
        let temp = parent.join(format!(
            ".{}.{}.tmp",
            path.file_name().unwrap_or_default().to_string_lossy(),
            new_id("save", self.revision)
        ));
        {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
                .map_err(|error| format!("Cannot reserve project save temporary: {error}"))?;
            if let Err(error) = file.write_all(&serialized).and_then(|_| file.sync_all()) {
                let _ = fs::remove_file(&temp);
                return Err(format!("Cannot write project: {error}"));
            }
        }
        if let Err(error) = fs::rename(&temp, &path) {
            let _ = fs::remove_file(&temp);
            return Err(format!("Cannot commit project: {error}"));
        }
        if let Ok(dir) = File::open(&parent) {
            let _ = dir.sync_all();
        }
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
    pub fn open(path: PathBuf) -> Result<Self, String> {
        let path = path
            .canonicalize()
            .map_err(|error| format!("Cannot access project: {error}"))?;
        let bytes = fs::read(&path).map_err(|error| format!("Cannot read project: {error}"))?;
        let file: ProjectFile = serde_json::from_slice(&bytes)
            .map_err(|error| format!("Cannot parse project file: {error}"))?;
        if file.schema_version != SCHEMA_VERSION {
            return Err(format!(
                "Unsupported project schema {}",
                file.schema_version
            ));
        }
        if file.data.schema_version != SCHEMA_VERSION {
            return Err("Project schema header and data disagree".into());
        }
        let mut data = file.data;
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
        }
        let mut project = Self::new(data)?;
        project.path = Some(path);
        project.saved_revision = 0;
        Ok(project)
    }
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
        if asset.width == 0 || asset.height == 0 {
            return Err("Media dimensions must be positive".into());
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
    let asset_ids = data
        .assets
        .iter()
        .map(|asset| asset.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut clip_ids = std::collections::HashSet::new();
    let mut ordered = data.clips.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|clip| clip.timeline_start);
    let mut previous_end = 0;
    for clip in ordered {
        if clip.id.is_empty() || !clip_ids.insert(clip.id.as_str()) {
            return Err("Missing or duplicate clip ID".into());
        }
        if !asset_ids.contains(clip.asset_id.as_str()) {
            return Err("Clip references a missing media asset".into());
        }
        if clip.duration == 0 {
            return Err("Clip duration must be positive".into());
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
        let asset = data
            .assets
            .iter()
            .find(|asset| asset.id == clip.asset_id)
            .unwrap();
        let origin = asset
            .start_pts
            .ok_or("Precise placement requires a known source presentation timestamp")?;
        let duration = asset
            .duration_pts
            .ok_or("Exact placement requires a known video duration")?;
        let media_end = origin
            .checked_add(duration)
            .ok_or("Media timestamp range overflow")?;
        let (source_end, remainder, _) =
            shifted_source(clip, i128::from(clip.duration), data.fps, asset.time_base)?;
        if clip.source_in_pts < origin
            || source_end > media_end
            || (source_end == media_end && remainder != 0)
        {
            return Err("Clip source range extends outside media presentation duration".into());
        }
        let transform = clip.transform;
        if !transform.scale.is_finite()
            || !(0.1..=4.0).contains(&transform.scale)
            || !transform.x.is_finite()
            || !(-1.0..=1.0).contains(&transform.x)
            || !transform.y.is_finite()
            || !(-1.0..=1.0).contains(&transform.y)
            || !transform.exposure.is_finite()
            || !(-8.0..=8.0).contains(&transform.exposure)
        {
            return Err("Invalid clip transform".into());
        }
    }
    Ok(())
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
    Rational::new(fps.numerator, fps.denominator)?;
    Rational::new(time_base.numerator, time_base.denominator)?;
    if clip.source_subtick_denominator == 0
        || clip.source_subtick_numerator >= clip.source_subtick_denominator
    {
        return Err("Invalid fractional source in-point".into());
    }
    let frame_den = i128::from(fps.numerator) * i128::from(time_base.numerator);
    let old_den = i128::from(clip.source_subtick_denominator);
    let common = gcd128(frame_den, old_den);
    let denominator = old_den
        .checked_mul(frame_den / common)
        .ok_or("Source timestamp denominator overflow")?;
    let frame_num = frames
        .checked_mul(i128::from(fps.denominator))
        .and_then(|v| v.checked_mul(i128::from(time_base.denominator)))
        .ok_or("Source timestamp overflow")?;
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
    let offset = sequence_frame
        .checked_sub(clip.timeline_start)
        .ok_or("Frame precedes selected clip")?;
    let clip_end = clip.timeline_end()?;
    if sequence_frame >= clip_end {
        return Err("Frame follows selected clip".into());
    }
    let (ticks, numerator, denominator) =
        shifted_source(clip, i128::from(offset), sequence_fps, asset.time_base)?;
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
    let mut file = File::open(path).map_err(|error| format!("Cannot open media file: {error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 128 * 1024];
    loop {
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
    fn asset(path: &Path) -> MediaAsset {
        MediaAsset {
            id: "asset-1".into(),
            name: "clip.mp4".into(),
            path: path.to_string_lossy().into_owned(),
            fingerprint: "sum".into(),
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
            data.clips.push(second);
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
        assert_eq!(p.data.clips, before.clips);
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
        assert_eq!(opened.data.clips, p.data.clips);
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
