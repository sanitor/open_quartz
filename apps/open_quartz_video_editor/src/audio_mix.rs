use crate::document::ProjectData;
use oq_media::{AudioReader, MediaError, StereoSample, MAX_AUDIO_BLOCK};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

struct MixClip {
    start: u64,
    end: u64,
    source: i64,
    path: PathBuf,
    stream: usize,
    gain: f32,
}

pub struct AudioMix {
    clips: Vec<MixClip>,
    readers: HashMap<usize, AudioReader>,
    cancel: Arc<AtomicBool>,
    scratch: Vec<StereoSample>,
}

impl AudioMix {
    pub fn new(project: &ProjectData, cancel: Arc<AtomicBool>) -> Result<Option<Self>, String> {
        if !project
            .audio_tracks
            .iter()
            .any(|track| !track.clips.is_empty())
        {
            return Ok(None);
        }
        let mut clips = Vec::new();
        for track in &project.audio_tracks {
            if track.muted || track.gain == 0.0 {
                continue;
            }
            for clip in &track.clips {
                if clip.gain == 0.0 {
                    continue;
                }
                let asset = project
                    .assets
                    .iter()
                    .find(|asset| asset.id == clip.asset_id)
                    .ok_or("Audio asset is missing")?;
                if !asset.online {
                    return Err(format!("Audio media offline: {} — use Relink", asset.name));
                }
                let stream = asset
                    .audio
                    .as_ref()
                    .ok_or("Audio stream is missing")?
                    .stream;
                let interval = clip.interval(project)?;
                clips.push(MixClip {
                    start: interval.start,
                    end: interval.end,
                    source: interval.source,
                    path: PathBuf::from(&asset.path),
                    stream,
                    gain: track.gain * clip.gain,
                });
            }
        }
        Ok(Some(Self {
            clips,
            readers: HashMap::new(),
            cancel,
            scratch: vec![[0.0; 2]; MAX_AUDIO_BLOCK],
        }))
    }

    pub fn sources(&self) -> impl Iterator<Item = &PathBuf> {
        self.clips
            .iter()
            .map(|clip| &clip.path)
            .collect::<BTreeSet<_>>()
            .into_iter()
    }

    pub fn render(&mut self, start: u64, output: &mut [StereoSample]) -> Result<(), MediaError> {
        let end = start
            .checked_add(output.len() as u64)
            .ok_or("Audio render range overflow")?;
        if output.len() > MAX_AUDIO_BLOCK {
            return Err("Audio block is too large".into());
        }
        output.fill([0.0; 2]);
        self.readers.retain(|index, _| {
            let clip = &self.clips[*index];
            clip.start < end && clip.end > start
        });
        for (index, clip) in self.clips.iter().enumerate() {
            let from = clip.start.max(start);
            let to = clip.end.min(end);
            if from >= to {
                continue;
            }
            let reader = match self.readers.entry(index) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => entry.insert(
                    AudioReader::open(&clip.path, clip.stream, self.cancel.clone())?,
                ),
            };
            let source = clip
                .source
                .checked_add(
                    i64::try_from(from - clip.start).map_err(|_| "Audio source offset overflow")?,
                )
                .ok_or("Audio source offset overflow")?;
            let len = (to - from) as usize;
            let source_end = source
                .checked_add(len as i64)
                .ok_or("Audio source offset overflow")?;
            let scratch = &mut self.scratch[..len];
            reader.read(source..source_end, scratch)?;
            let destination = (from - start) as usize;
            for (sum, sample) in output[destination..destination + len]
                .iter_mut()
                .zip(scratch)
            {
                for channel in 0..2 {
                    if sample[channel].is_finite() {
                        sum[channel] += sample[channel] * clip.gain;
                    }
                }
            }
        }
        for sample in output.iter_mut().flatten() {
            *sample = sample.clamp(-1.0, 1.0);
        }
        Ok(())
    }
}
