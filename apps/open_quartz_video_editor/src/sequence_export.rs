use super::{
    configure, playback, sequence_region, sequence_source_times, FramePipeline,
    PlaybackCapabilities, PreviewConfiguration, PreviewRequest, ProjectData, RenderDevice,
    OUTPUT_ID,
};
use open_quartz::execution::{ExportConfig, ExportJob, ExportPhase, ExportSequence};
use open_quartz::rendering::InputInterpretation;
use oq_media::{EncoderConfig, RationalTimeBase};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub struct ExportTask {
    cancel: Arc<AtomicBool>,
    progress: Arc<Mutex<(String, bool)>>,
    worker: Option<JoinHandle<()>>,
}

impl ExportTask {
    pub fn start(
        render: RenderDevice,
        project: ProjectData,
        revision: u64,
        path: PathBuf,
    ) -> Result<Self, String> {
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Mutex::new(("Preparing MP4 export…".into(), false)));
        let worker_cancel = cancel.clone();
        let worker_progress = progress.clone();
        let worker = thread::Builder::new()
            .name("oq-editor-export".into())
            .spawn(move || {
                let result = run(
                    render,
                    project,
                    revision,
                    &path,
                    &worker_cancel,
                    &worker_progress,
                );
                let text = match result {
                    Ok(true) => format!("Export complete · {}", path.display()),
                    Ok(false) => "Export cancelled".into(),
                    Err(error) => format!("Export failed · {error}"),
                };
                *worker_progress.lock().unwrap() = (text, true);
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            cancel,
            progress,
            worker: Some(worker),
        })
    }

    pub fn progress(&self) -> (String, bool) {
        self.progress.lock().unwrap().clone()
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(worker) = &self.worker {
            worker.thread().unpark();
        }
    }
}

impl Drop for ExportTask {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            self.cancel.store(true, Ordering::Release);
            worker.thread().unpark();
            let _ = worker.join();
        }
    }
}

struct Sequence {
    project: ProjectData,
    revision: u64,
    duration: u64,
    configured: Option<PreviewConfiguration>,
    request: Option<PreviewRequest>,
}

impl ExportSequence for Sequence {
    fn prepare_region(&mut self, pipeline: &mut FramePipeline, frame: u64) -> Result<u64, String> {
        let (request, end) =
            sequence_region(&self.project, 0, self.revision, frame, self.duration)?;
        playback::configure_limits(pipeline, &request, self.project.fps)?;
        configure(pipeline, &mut self.configured, &request)?;
        self.request = Some(request);
        Ok(end)
    }

    fn source_times(&self, frame: u64) -> Result<(u64, BTreeMap<String, u64>), String> {
        let request = self
            .request
            .as_ref()
            .ok_or("Export region has not been prepared")?;
        sequence_source_times(&self.project, request, frame)
    }
}

fn run(
    render: RenderDevice,
    project: ProjectData,
    revision: u64,
    path: &Path,
    cancel: &Arc<AtomicBool>,
    progress: &Mutex<(String, bool)>,
) -> Result<bool, String> {
    let duration = project.duration_frames();
    let source_ids: BTreeSet<_> = project
        .tracks
        .iter()
        .filter(|track| track.visible)
        .flat_map(|track| &track.clips)
        .filter(|clip| clip.title.is_none())
        .map(|clip| clip.asset_id.as_str())
        .collect();
    let mut sources = BTreeSet::new();
    for asset in project
        .assets
        .iter()
        .filter(|asset| source_ids.contains(asset.id.as_str()))
    {
        if !asset.online {
            return Err(format!("Media offline: {} — use Relink", asset.name));
        }
        sources.insert(PathBuf::from(&asset.path));
    }
    let audio = crate::audio_mix::AudioMix::new(&project, cancel.clone())?;
    if let Some(audio) = &audio {
        sources.extend(audio.sources().cloned());
    }
    let audio = audio.map(|mut audio| -> oq_media::AudioRender {
        Box::new(move |start, block| audio.render(start, block))
    });
    let sources: Vec<_> = sources.into_iter().collect();
    if cancel.load(Ordering::Acquire) {
        return Ok(false);
    }
    let config = ExportConfig {
        output: path.into(),
        output_node_id: OUTPUT_ID.into(),
        frames: 0..duration,
        encoder: EncoderConfig {
            width: project.width,
            height: project.height,
            time_base: RationalTimeBase {
                numerator: project.fps.denominator,
                denominator: project.fps.numerator,
            },
            bitrate: (u64::from(project.width) * u64::from(project.height) * 4)
                .clamp(2_000_000, 80_000_000),
            queue_capacity: 3,
            max_b_frames: 0,
            overwrite: false,
        },
        input_interpretation: InputInterpretation::SdrSrgbRgba8Black,
        date: [0.0; 4],
        mouse: [0.0; 4],
    };
    let sequence = Sequence {
        project,
        revision,
        duration,
        configured: None,
        request: None,
    };
    let pipeline =
        FramePipeline::from_render_device(render, PlaybackCapabilities { data_paths: vec![] });
    let mut job = ExportJob::new_sequence_with_audio(pipeline, config, sequence, &sources, audio)?;
    let owner = thread::current();
    job.set_waker(Arc::new(move || owner.unpark()))?;
    let mut cancelling = false;
    loop {
        if !cancelling && cancel.load(Ordering::Acquire) {
            job.cancel();
            cancelling = true;
        }
        let status = job.poll();
        let text = match status.phase {
            ExportPhase::Completed => return Ok(true),
            ExportPhase::Cancelled => return Ok(false),
            ExportPhase::Failed => {
                return Err(status.error.as_deref().unwrap_or("Encoder failed").into())
            }
            ExportPhase::Cancelling => "Cancelling export…".into(),
            ExportPhase::ReadyToCommit | ExportPhase::Committing => "Finalizing MP4…".into(),
            _ => format!(
                "Exporting MP4 · {} / {} frames",
                status.accepted_frames, status.total_frames
            ),
        };
        progress.lock().unwrap().0 = text;
        thread::park_timeout(Duration::from_millis(2));
    }
}
