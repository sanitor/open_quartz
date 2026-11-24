use crate::document::{AudioInterval, MediaAsset, ProjectData};
use oq_media::{audio_sample, AudioReader, RationalTimeBase, MAX_AUDIO_BLOCK};
use std::collections::HashMap;
use std::fmt::Write;
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};

const BINS: usize = 4096;
const COLUMNS: usize = 128;

#[derive(Default)]
pub struct Waveforms {
    cache: HashMap<String, Option<Peaks>>,
    pending: Option<Pending>,
    scan: bool,
}

struct Pending {
    id: String,
    result: mpsc::Receiver<Result<Peaks, String>>,
    cancel: Arc<AtomicBool>,
}

struct Peaks {
    start: i64,
    length: usize,
    bins: [f32; BINS],
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

impl Waveforms {
    pub fn request_scan(&mut self) {
        self.scan = true;
    }

    pub fn invalidate(&mut self, id: &str) {
        self.cache.remove(id);
        if self.pending.as_ref().is_some_and(|job| job.id == id) {
            self.pending = None;
        }
        self.scan = true;
    }

    pub fn poll(&mut self, project: &ProjectData) -> bool {
        let mut changed = false;
        if let Some(job) = &self.pending {
            let peaks = match job.result.try_recv() {
                Ok(result) => result.ok(),
                Err(mpsc::TryRecvError::Empty) => return false,
                Err(mpsc::TryRecvError::Disconnected) => None,
            };
            self.cache.insert(job.id.clone(), peaks);
            self.pending = None;
            changed = true;
            self.scan = true;
        }
        if !std::mem::take(&mut self.scan) {
            return changed;
        }
        let asset = project
            .audio_tracks
            .iter()
            .flat_map(|track| &track.clips)
            .filter(|clip| !self.cache.contains_key(&clip.asset_id))
            .find_map(|clip| {
                project
                    .assets
                    .iter()
                    .find(|asset| asset.id == clip.asset_id && asset.online)
            });
        if let Some(asset) = asset {
            let asset = asset.clone();
            let id = asset.id.clone();
            let (send, result) = mpsc::channel();
            let cancel = Arc::new(AtomicBool::new(false));
            let worker_cancel = cancel.clone();
            if std::thread::Builder::new()
                .name("oq-audio-waveform".into())
                .spawn(move || {
                    let _ = send.send(Peaks::read(&asset, worker_cancel));
                })
                .is_ok()
            {
                self.pending = Some(Pending { id, result, cancel });
            } else {
                self.cache.insert(id, None);
            }
        }
        changed
    }

    pub fn path(&self, asset: &str, interval: AudioInterval) -> Option<String> {
        self.cache
            .get(asset)?
            .as_ref()
            .map(|peaks| peaks.path(interval))
    }
}

impl Peaks {
    fn read(asset: &MediaAsset, cancel: Arc<AtomicBool>) -> Result<Self, String> {
        let audio = asset.audio.as_ref().ok_or("Missing waveform audio")?;
        let base = RationalTimeBase {
            numerator: audio.time_base.numerator,
            denominator: audio.time_base.denominator,
        };
        let end = audio
            .start_pts
            .checked_add(audio.duration_pts.ok_or("Unknown waveform duration")?)
            .ok_or("Waveform duration overflow")?;
        let start = audio_sample(audio.start_pts, base).map_err(|error| error.to_string())?;
        let end = audio_sample(end, base).map_err(|error| error.to_string())?;
        let length = end
            .checked_sub(start)
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value > 0)
            .ok_or("Invalid waveform duration")?;
        let mut reader = AudioReader::open(Path::new(&asset.path), audio.stream, cancel)
            .map_err(|error| error.to_string())?;
        let mut bins = [0.0_f32; BINS];
        let mut block = vec![[0.0; 2]; MAX_AUDIO_BLOCK];
        for offset in (0..length).step_by(MAX_AUDIO_BLOCK) {
            let count = (length - offset).min(MAX_AUDIO_BLOCK);
            let at = start + offset as i64;
            reader
                .read(at..at + count as i64, &mut block[..count])
                .map_err(|error| error.to_string())?;
            for (index, sample) in block[..count].iter().enumerate() {
                let bin = ((offset + index) as u128 * BINS as u128 / length as u128) as usize;
                bins[bin] = bins[bin].max(sample[0].abs().max(sample[1].abs()).min(1.0));
            }
        }
        Ok(Self {
            start,
            length,
            bins,
        })
    }

    fn path(&self, interval: AudioInterval) -> String {
        let mut path = String::new();
        let offset = i128::from(interval.source) - i128::from(self.start);
        let span = i128::from(interval.end - interval.start) * i128::from(interval.speed_percent);
        let index = |column: usize, ceil: bool| {
            let sample = offset * (100 * COLUMNS as i128) + span * column as i128;
            let denominator = 100 * COLUMNS as i128 * self.length as i128;
            (sample * BINS as i128 + if ceil { denominator - 1 } else { 0 })
                .div_euclid(denominator)
                .clamp(0, BINS as i128) as usize
        };
        for column in 0..COLUMNS {
            let peak = self.bins[index(column, false)..index(column + 1, true)]
                .iter()
                .copied()
                .fold(0.0_f32, f32::max);
            let y = (peak.sqrt() * 18.0).round() as u32;
            let _ = write!(path, "M{column} {}V{} ", 20 - y, 20 + y);
        }
        path
    }
}
