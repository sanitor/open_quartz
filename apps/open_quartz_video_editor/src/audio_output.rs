use crate::audio_mix::AudioMix;
use crate::document::{ProjectData, SAMPLE_RATE};
use oq_media::StereoSample;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

#[cfg(target_os = "macos")]
#[path = "audio_output_macos.rs"]
mod platform;
#[cfg(windows)]
#[path = "audio_output_windows.rs"]
mod platform;

const BLOCK: usize = 1024;
const BYTES: usize = std::mem::size_of::<StereoSample>();

#[derive(Debug, Default)]
pub struct AudioClock {
    video_ready: AtomicBool,
    finished: AtomicBool,
    played: AtomicU64,
    peak: AtomicU32,
    clipping: AtomicBool,
    track_peaks: Vec<AtomicU32>,
}
impl AudioClock {
    pub fn start(&self) {
        self.video_ready.store(true, Ordering::Release);
    }
    pub fn elapsed_ns(&self) -> u64 {
        let samples = self.played.load(Ordering::Acquire);
        u64::try_from(u128::from(samples) * 1_000_000_000 / u128::from(SAMPLE_RATE))
            .unwrap_or(u64::MAX)
    }
    pub fn peak(&self) -> f32 {
        f32::from_bits(self.peak.load(Ordering::Relaxed))
    }
    pub fn clipping(&self) -> bool {
        self.clipping.load(Ordering::Relaxed)
    }
    pub fn track_peaks(&self) -> impl Iterator<Item = f32> + '_ {
        self.track_peaks
            .iter()
            .map(|level| f32::from_bits(level.load(Ordering::Relaxed)))
    }
    pub fn finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }
    fn update_meters(&self, mix: &AudioMix) {
        self.peak
            .store(mix.output_peak.min(1.0).to_bits(), Ordering::Relaxed);
        if mix.output_peak > 1.0 {
            self.clipping.store(true, Ordering::Relaxed);
        }
        for (meter, peak) in self.track_peaks.iter().zip(&mix.track_peaks) {
            meter.store(peak.to_bits(), Ordering::Relaxed);
        }
    }
}

pub struct AudioPlayback {
    pub clock: Arc<AudioClock>,
    cancel: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    worker: Option<JoinHandle<()>>,
}

impl AudioPlayback {
    pub fn start(project: &ProjectData, start_frame: u64) -> Result<Option<Self>, String> {
        let cancel = Arc::new(AtomicBool::new(false));
        let Some(mix) = AudioMix::new(project, cancel.clone(), None)? else {
            return Ok(None);
        };
        if mix.sources().next().is_none() {
            return Ok(None);
        }
        let start = project.fps.frame_to_sample(start_frame)?;
        let end = project.fps.frame_to_sample(project.duration_frames())?;
        let clock = Arc::new(AudioClock {
            track_peaks: (0..project.audio_tracks.len())
                .map(|_| AtomicU32::default())
                .collect(),
            ..AudioClock::default()
        });
        let error = Arc::new(Mutex::new(None));
        let worker_clock = clock.clone();
        let worker_cancel = cancel.clone();
        let worker_error = error.clone();
        let worker = thread::Builder::new()
            .name("oq-editor-audio".into())
            .spawn(move || {
                if let Err(error) = platform::play(mix, start, end, &worker_clock, &worker_cancel) {
                    if !worker_cancel.load(Ordering::Acquire) {
                        *worker_error.lock().unwrap() = Some(error);
                    }
                }
            })
            .map_err(|error| format!("Cannot start audio playback: {error}"))?;
        Ok(Some(Self {
            clock,
            cancel,
            error,
            worker: Some(worker),
        }))
    }

    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap().take()
    }
}

impl Drop for AudioPlayback {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            let _ = worker.join();
        }
    }
}
