use super::*;
use serde::Deserialize;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Deserialize)]
struct Segment {
    offsets: Offsets,
    text: String,
}

#[derive(Clone, Deserialize)]
struct Offsets {
    from: i64,
    to: i64,
}

#[derive(Deserialize)]
struct Transcript {
    transcription: Vec<Segment>,
}

#[derive(Default)]
pub(super) struct TranscriptPanel {
    result: Option<(MediaAsset, Vec<Segment>)>,
    read: Option<mpsc::Receiver<Result<Vec<Segment>, String>>>,
    source: Option<MediaAsset>,
    cancel: Option<Arc<AtomicBool>>,
    worker: Option<std::thread::JoinHandle<()>>,
    search: String,
    selected: BTreeSet<usize>,
}

impl Drop for TranscriptPanel {
    fn drop(&mut self) {
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Release);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run(mut cmd: Command, path: &Path, cancel: &AtomicBool) -> Result<(), String> {
    let log = std::fs::File::create(path).map_err(|error| error.to_string())?;
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(log);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0800_0000);
    let mut child = cmd.spawn().map_err(|error| {
        format!(
            "Cannot start {}: {error}",
            cmd.get_program().to_string_lossy()
        )
    })?;
    loop {
        if cancel.load(Ordering::Acquire) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Transcription cancelled".into());
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                return Err(format!(
                    "{} exited with {status}",
                    cmd.get_program().to_string_lossy()
                ))
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(30)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.to_string());
            }
        }
    }
}

fn transcribe(asset: &MediaAsset, cancel: &AtomicBool) -> Result<Vec<Segment>, String> {
    let model = std::env::var_os("OPEN_QUARTZ_WHISPER_MODEL")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .ok_or("Set OPEN_QUARTZ_WHISPER_MODEL to a local Whisper model")?;
    let whisper =
        std::env::var_os("OPEN_QUARTZ_WHISPER_CLI").unwrap_or_else(|| "whisper-cli".into());
    let language = std::env::var("OPEN_QUARTZ_WHISPER_LANGUAGE").unwrap_or_else(|_| "auto".into());
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let audio = temp.path().join("audio.wav");
    let output = temp.path().join("transcript");
    let log = temp.path().join("task.log");
    let mut ffmpeg = Command::new(open_quartz::media::native::find_ffmpeg()?);
    ffmpeg
        .args(["-v", "error", "-i"])
        .arg(&asset.path)
        .args([
            "-map",
            "0:a:0",
            "-vn",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-c:a",
            "pcm_s16le",
        ])
        .arg(&audio);
    run(ffmpeg, &log, cancel)?;
    let mut recognizer = Command::new(whisper);
    recognizer
        .arg("-m")
        .arg(model)
        .arg("-f")
        .arg(audio)
        .args(["-l", &language, "-oj", "-np", "-of"])
        .arg(&output);
    run(recognizer, &log, cancel)?;
    let bytes = std::fs::read(output.with_extension("json"))
        .map_err(|error| format!("Cannot read Whisper result: {error}"))?;
    let mut transcript: Transcript = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Invalid Whisper result: {error}"))?;
    for segment in &mut transcript.transcription {
        segment.text = segment.text.trim().to_owned();
    }
    transcript
        .transcription
        .retain(|segment| segment.offsets.to > segment.offsets.from && !segment.text.is_empty());
    transcript
        .transcription
        .sort_by_key(|segment| segment.offsets.from);
    Ok(transcript.transcription)
}

impl Controller {
    pub(super) fn open_transcript(&mut self) {
        let Some(asset) = self
            .selected_asset
            .as_deref()
            .and_then(|id| self.document.asset(id))
            .cloned()
        else {
            self.status("Select a video with audio in the media library");
            return;
        };
        if !asset.has_video() || !asset.online || asset.audio.is_none() {
            self.status("Select an online video with audio");
            return;
        }
        if self.transcript.source.as_ref() != Some(&asset)
            && self.transcript.result.as_ref().map(|(source, _)| source) != Some(&asset)
        {
            let worker_asset = asset.clone();
            let cancel = Arc::new(AtomicBool::new(false));
            let worker_cancel = cancel.clone();
            let (send, read) = mpsc::channel();
            let worker = std::thread::Builder::new()
                .name("studio-transcription".into())
                .spawn(move || {
                    let _ = send.send(transcribe(&worker_asset, &worker_cancel));
                });
            let worker = match worker {
                Ok(worker) => worker,
                Err(error) => {
                    self.status(error.to_string());
                    return;
                }
            };
            self.transcript = TranscriptPanel {
                result: None,
                source: Some(asset),
                read: Some(read),
                cancel: Some(cancel),
                worker: Some(worker),
                search: String::new(),
                selected: BTreeSet::new(),
            };
        }
        if let Some(window) = self.window.upgrade() {
            window.set_transcript_open(true);
        }
        self.publish_transcript();
    }

    pub(super) fn poll_transcript(&mut self) {
        let Some(read) = &self.transcript.read else {
            return;
        };
        let result = match read.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("Transcription worker stopped".into()),
        };
        self.transcript.read = None;
        self.transcript.cancel = None;
        if let Some(worker) = self.transcript.worker.take() {
            let _ = worker.join();
        }
        let source = self.transcript.source.take();
        match result {
            Ok(segments) => self.transcript.result = source.map(|asset| (asset, segments)),
            Err(error) => self.status(error),
        }
        self.publish_transcript();
    }

    pub(super) fn publish_transcript(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        window.set_transcript_busy(self.transcript.read.is_some());
        window.set_transcript_query(self.transcript.search.clone().into());
        let query = self.transcript.search.to_lowercase();
        let rows = self
            .transcript
            .result
            .iter()
            .flat_map(|(_, segments)| segments.iter().enumerate())
            .filter(|(_, segment)| segment.text.to_lowercase().contains(&query))
            .map(|(index, segment)| crate::TranscriptRow {
                index: index as i32,
                label: format!(
                    "{:.2}s · {}",
                    segment.offsets.from as f64 / 1_000.0,
                    segment.text
                )
                .into(),
                checked: self.transcript.selected.contains(&index),
            })
            .collect::<Vec<_>>();
        window.set_transcript_rows(ModelRc::new(VecModel::from(rows)));
        window.set_transcript_can_build(!self.transcript.selected.is_empty());
    }

    pub(super) fn transcript_action(&mut self, action: &str, index: i32, query: &str) {
        match action {
            "search" => self.transcript.search = query.to_owned(),
            "toggle" => {
                let Ok(index) = usize::try_from(index) else {
                    return;
                };
                if !self.transcript.selected.remove(&index) {
                    self.transcript.selected.insert(index);
                }
            }
            "close" => {
                if self.transcript.read.is_some() {
                    self.transcript = Default::default();
                }
                if let Some(window) = self.window.upgrade() {
                    window.set_transcript_open(false);
                }
            }
            "seek" | "build" => {
                let Some((asset, segments)) = &self.transcript.result else {
                    return;
                };
                if self.document.asset(&asset.id) != Some(asset) {
                    self.status(
                        "The transcript source has changed. Transcribe the current media again.",
                    );
                    return;
                }
                let fps = self.document.data.fps;
                let ranges = |segment: &Segment| -> Option<(Range<u64>, String)> {
                    let from = document::subtitle::boundary_frame(
                        fps,
                        source_ns(asset, segment.offsets.from)?,
                    )
                    .ok()?;
                    let to = document::subtitle::boundary_frame(
                        fps,
                        source_ns(asset, segment.offsets.to)?,
                    )
                    .ok()?
                    .min(asset.source_duration_frames(fps).ok()?);
                    (from < to).then(|| (from..to, segment.text.clone()))
                };
                if action == "seek" {
                    let Some((range, _)) = segments.get(index as usize).and_then(ranges) else {
                        return;
                    };
                    let id = asset.id.clone();
                    self.pause();
                    self.selected_asset = Some(id);
                    self.asset_preview = true;
                    self.source_action("seek", &range.start.to_string());
                    if let Some(window) = self.window.upgrade() {
                        window.set_transcript_open(false);
                    }
                } else {
                    let selected: Vec<_> = self
                        .transcript
                        .selected
                        .iter()
                        .filter_map(|&i| segments.get(i).and_then(ranges))
                        .collect();
                    let asset = asset.id.clone();
                    let previous = self.document.data.sequence_id.clone();
                    let result = self.document.append_transcript_cut(&asset, selected);
                    if result.is_ok() {
                        self.refresh_sequence_view(&previous);
                        if let Some(window) = self.window.upgrade() {
                            window.set_transcript_open(false);
                        }
                    }
                    self.finish_edit(result);
                }
            }
            _ => return,
        }
        self.publish_transcript();
    }
}

fn source_ns(asset: &MediaAsset, timestamp_ms: i64) -> Option<u64> {
    let audio = asset.audio.as_ref()?;
    let to_ns = |pts: i64, base: Rational| {
        i128::from(pts) * i128::from(base.numerator) * 1_000_000_000 / i128::from(base.denominator)
    };
    let value = i128::from(timestamp_ms) * 1_000_000 + to_ns(audio.start_pts, audio.time_base)
        - to_ns(asset.start_pts?, asset.time_base);
    u64::try_from(value.max(0)).ok()
}
