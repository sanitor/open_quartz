use super::title::generated_clip;
use super::{new_id, EditorDocument, ProjectData, Rational, Title, VideoTrack};
use std::fs::File;
use std::io::Read;
use std::path::Path;

const MAX_SRT_BYTES: u64 = 1024 * 1024;

struct Cue {
    start_ns: u64,
    end_ns: u64,
    text: String,
}

impl EditorDocument {
    pub fn append_transcript_cut(
        &mut self,
        asset: &str,
        selections: Vec<(std::ops::Range<u64>, String)>,
    ) -> Result<(), String> {
        if selections.is_empty() {
            return Err("Select at least one transcript segment".into());
        }
        let mut draft = EditorDocument::new(self.data.clone())?;
        draft.add_sequence(false)?;
        let video_track = draft.data.tracks[0].id.clone();
        let subtitle_id = new_id("transcript", self.revision);
        let mut end = 0u64;
        for (index, (range, text)) in selections.into_iter().enumerate() {
            let length = range.end.checked_sub(range.start).ok_or("Invalid transcript range")?;
            draft.place_visual_range(asset, &video_track, super::Placement::EmptySpace(Some(end)),
                Some(range), &super::AudioDestination::Automatic)?;
            subtitle_track(&mut draft.data, &subtitle_id)?.clips.push(generated_clip(
                format!("{subtitle_id}-{index}"), end, length, subtitle(text),
            ));
            end = end.checked_add(length).ok_or("Transcript cut duration overflow")?;
        }
        self.transact(|data| { *data = draft.data; Ok(()) })
    }

    pub fn add_subtitle(&mut self, start: u64) -> Result<(String, String), String> {
        let id = new_id("subtitle", self.revision);
        let mut track_id = new_id("subtitle-track", self.revision);
        self.transact(|data| {
            let default_duration = data.fps.ns_to_frames_floor(2_000_000_000)?.max(1);
            let track = subtitle_track(data, &track_id)?;
            track_id.clone_from(&track.id);
            if track.clip_at(start).is_some() {
                return Err("A subtitle already covers the playhead".into());
            }
            let duration = track
                .clips
                .iter()
                .find(|clip| clip.timeline_start > start)
                .map_or(default_duration, |next| {
                    default_duration.min(next.timeline_start - start)
                });
            track.clips.push(generated_clip(
                id.clone(),
                start,
                duration,
                subtitle("Subtitle".into()),
            ));
            track.clips.sort_by_key(|clip| clip.timeline_start);
            Ok(())
        })?;
        Ok((track_id, id))
    }

    pub fn import_subtitles(&mut self, path: &Path) -> Result<usize, String> {
        let mut source = Vec::new();
        File::open(path)
            .and_then(|file| file.take(MAX_SRT_BYTES + 1).read_to_end(&mut source))
            .map_err(|error| format!("Cannot read subtitles: {error}"))?;
        if source.len() as u64 > MAX_SRT_BYTES {
            return Err("SRT file exceeds 1 MiB".into());
        }
        let source = std::str::from_utf8(&source).map_err(|_| "SRT must use UTF-8 text")?;
        let cues = parse_srt(source)?;
        let count = cues.len();
        let import_id = new_id("subtitle-import", self.revision);
        self.transact(|data| {
            let fps = data.fps;
            let track = subtitle_track(data, &import_id)?;
            for (index, cue) in cues.into_iter().enumerate() {
                let start = boundary_frame(fps, cue.start_ns)?;
                let end = boundary_frame(fps, cue.end_ns)?;
                if start >= end {
                    return Err(format!("SRT cue {} spans no sequence frames", index + 1));
                }
                track.clips.push(generated_clip(
                    format!("{import_id}-{index}"),
                    start,
                    end - start,
                    subtitle(cue.text),
                ));
            }
            track.clips.sort_by_key(|clip| clip.timeline_start);
            for pair in track.clips.windows(2) {
                if pair[0].timeline_end()? > pair[1].timeline_start {
                    return Err("Imported subtitles overlap on the sequence".into());
                }
            }
            Ok(())
        })?;
        Ok(count)
    }
}

fn subtitle_track<'a>(data: &'a mut ProjectData, id: &str) -> Result<&'a mut VideoTrack, String> {
    let index = if let Some(index) = data.tracks.iter().position(|track| track.subtitles) {
        index
    } else {
        let mut track = VideoTrack::new(id.into(), "Subtitles".into());
        track.subtitles = true;
        data.tracks.insert(0, track);
        0
    };
    let track = &mut data.tracks[index];
    track.require_unlocked()?;
    Ok(track)
}

fn subtitle(text: String) -> Title {
    Title {
        text,
        subtitle: true,
        size_percent: 5,
    }
}

// A cue includes samples at or after its start and strictly before its end.
pub(crate) fn boundary_frame(fps: Rational, ns: u64) -> Result<u64, String> {
    let scaled = u128::from(ns) * fps.numerator as u128;
    let frame = scaled.div_ceil(fps.denominator as u128 * 1_000_000_000);
    u64::try_from(frame).map_err(|_| "Subtitle position exceeds supported range".into())
}

fn parse_srt(source: &str) -> Result<Vec<Cue>, String> {
    let normalized = source
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut lines = normalized.lines().peekable();
    let mut cues = Vec::new();
    while lines.peek().is_some() {
        let Some(index) = lines.by_ref().find(|line| !line.trim().is_empty()) else {
            break;
        };
        let number = cues.len() + 1;
        let invalid = || format!("Invalid timestamp or index in SRT cue {number}");
        if index.trim().parse::<u64>().is_err() {
            return Err(invalid());
        }
        let (start, end) = lines
            .next()
            .and_then(|line| line.split_once("-->"))
            .ok_or_else(invalid)?;
        let start_ns = timestamp(start.trim()).ok_or_else(invalid)?;
        let end_ns = timestamp(end.trim())
            .filter(|end| *end > start_ns)
            .ok_or_else(invalid)?;
        let text = lines
            .by_ref()
            .take_while(|line| !line.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        subtitle(text.clone())
            .validate()
            .map_err(|error| format!("SRT cue {number}. {error}"))?;
        cues.push(Cue {
            start_ns,
            end_ns,
            text,
        });
    }
    if cues.is_empty() {
        return Err("SRT contains no cues".into());
    }
    Ok(cues)
}

fn timestamp(value: &str) -> Option<u64> {
    let (clock, ms) = value.split_once(',')?;
    let (hours, rest) = clock.split_once(':')?;
    let (minutes, seconds) = rest.split_once(':')?;
    if hours.len() < 2
        || minutes.len() != 2
        || seconds.len() != 2
        || ms.len() != 3
        || ![hours, minutes, seconds, ms]
            .iter()
            .all(|part| part.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let hours = hours.parse::<u64>().ok()?;
    let minutes = minutes.parse::<u64>().ok().filter(|value| *value < 60)?;
    let seconds = seconds.parse::<u64>().ok().filter(|value| *value < 60)?;
    let ms = ms.parse::<u64>().ok()?;
    hours
        .checked_mul(3_600_000_000_000)?
        .checked_add(minutes * 60_000_000_000 + seconds * 1_000_000_000 + ms * 1_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtitle_boundaries_use_the_first_sample_at_or_after_each_timestamp() {
        let fps = Rational::new(30_000, 1_001).unwrap();
        assert_eq!(boundary_frame(fps, 0), Ok(0));
        assert_eq!(boundary_frame(fps, 1_000_000_000), Ok(30));
        assert_eq!(boundary_frame(fps, 1_001_000_000), Ok(30));
        assert_eq!(boundary_frame(fps, 1_001_000_001), Ok(31));
    }

    #[test]
    fn a_cue_with_no_samples_leaves_the_project_and_history_intact() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("short.srt");
        std::fs::write(
            &path,
            "1\n00:00:00,000 --> 00:00:01,000\nFirst cue\n\n2\n00:00:01,001 --> 00:00:01,002\nNo sample\n",
        )
        .unwrap();

        let mut doc = EditorDocument::default();
        assert_eq!(
            doc.import_subtitles(&path),
            Err("SRT cue 2 spans no sequence frames".into())
        );
        assert_eq!(doc.data, ProjectData::default());
        assert!(!doc.can_undo());
    }
}
