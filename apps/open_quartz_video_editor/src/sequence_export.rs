use super::{
    configure, playback, sequence_region_end, sequence_source_times,
    FramePipeline, PlaybackCapabilities, PreviewConfiguration, PreviewRequest, ProjectData,
    RenderDevice, OUTPUT_ID,
};
use crate::delivery::{Cadence, ExportOptions};
use crate::output_queue::Phase;
use open_quartz::execution::{ExportConfig, ExportJob, ExportPhase, ExportSequence};
use open_quartz::rendering::InputInterpretation;
use oq_media::{EncoderConfig, RationalTimeBase};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub struct ExportTask {
    cancel: Arc<AtomicBool>,
    progress: Arc<Mutex<(String, Option<Phase>)>>,
    worker: Option<JoinHandle<()>>,
}

impl ExportTask {
    pub fn start(
        render: RenderDevice,
        project: ProjectData,
        revision: u64,
        path: PathBuf,
        frames: Range<u64>,
        options: ExportOptions,
    ) -> Result<Self, String> {
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Mutex::new(("Preparing MP4 export…".into(), None)));
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
                    frames,
                    options,
                    &worker_cancel,
                    &worker_progress,
                );
                let (text, phase) = match result {
                    Ok(true) => (format!("Export complete · {}", path.display()), Phase::Complete),
                    Ok(false) => ("Export cancelled".into(), Phase::Cancelled),
                    Err(error) => (format!("Export failed · {error}"), Phase::Failed),
                };
                *worker_progress.lock().unwrap() = (text, Some(phase));
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            cancel,
            progress,
            worker: Some(worker),
        })
    }

    pub fn progress(&self) -> (String, Option<Phase>) {
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
    end: u64,
    cadence: Cadence,
    hdr10: bool,
    configured: Option<PreviewConfiguration>,
    request: Option<PreviewRequest>,
}

impl ExportSequence for Sequence {
    fn prepare_region(&mut self, pipeline: &mut FramePipeline, frame: u64) -> Result<u64, String> {
        let frame = self.cadence.sample(frame)?;
        let mut request = super::build_sequence_request(&self.project, 0, self.revision, frame, &mut |_| None, None,
            false, self.hdr10 || self.project.managed_color)?;
        request.managed_view = !self.hdr10 && self.project.managed_color;
        let end = sequence_region_end(&self.project, frame, self.end);
        playback::configure_limits(pipeline, &request, self.project.fps)?;
        configure(pipeline, &mut self.configured, &request)?;
        self.request = Some(request);
        self.cadence.boundary(end)
    }

    fn source_times(&self, frame: u64) -> Result<(u64, BTreeMap<String, u64>), String> {
        let request = self
            .request
            .as_ref()
            .ok_or("Export region has not been prepared")?;
        sequence_source_times(&self.project, request, self.cadence.sample(frame)?)
    }
}

fn video_source_ids(
    project: &ProjectData,
    end: u64,
    cadence: &Cadence,
    total: u64,
    cancel: &AtomicBool,
) -> Result<BTreeSet<String>, String> {
    let mut ids = BTreeSet::new();
    let mut output_frame = 0;
    while output_frame < total && !cancel.load(Ordering::Acquire) {
        let frame = cadence.sample(output_frame)?;
        let request = super::sequence_request(project, 0, 0, frame, |_| None, None)?;
        ids.extend(request.sources().map(|(_, asset, _)| asset.id.clone()));
        collect_stills(&request, &mut ids);
        output_frame = cadence.boundary(sequence_region_end(project, frame, end))?;
    }
    Ok(ids)
}

fn collect_stills(request: &super::PreviewRequest, assets: &mut BTreeSet<String>) {
    for layer in &request.layers {
        match &layer.source {
            super::LayerSource::Still(asset) => { assets.insert(asset.id.clone()); }
            super::LayerSource::Nested { request, .. } => collect_stills(request, assets),
            _ => {}
        }
    }
}

fn run(
    render: RenderDevice,
    mut project: ProjectData,
    revision: u64,
    path: &Path,
    frames: Range<u64>,
    options: ExportOptions,
    cancel: &Arc<AtomicBool>,
    progress: &Mutex<(String, Option<Phase>)>,
) -> Result<bool, String> {
    if options.hdr10 {
        if project.tracks.iter().filter(|track| track.visible && !track.clips.is_empty()).count() > 1 {
            return Err("HDR10 currently requires a single visible video track".into());
        }
        for input in project.tracks.iter().filter(|track| track.visible).flat_map(|track| &track.clips) {
            if input.timeline_start >= frames.end || input.timeline_end()? <= frames.start { continue; }
            if project.assets.iter().find(|asset| asset.id == input.asset_id).is_some_and(|asset| asset.sequence_source().is_some()) {
                return Err("Nested sequences are not available in HDR10 export".into());
            }
            if input.pipeline.graph().nodes.iter().any(|node| node.id != super::SOURCE_ID && node.id != OUTPUT_ID
                && node.node_type != open_quartz::project::NodeType::ColorCorrector
                && crate::clip_pipeline::BranchComposite::from_node(node).is_none()
                && !crate::clip_pipeline::ClipPipeline::is_lut(node))
            {
                return Err("HDR10 supports Color Corrector and clip composition. Remove legacy shader effects from the export range.".into());
            }
        }
    }
    let output_fps = options.fps(project.fps);
    let cadence = Cadence::new(frames.start, project.fps, output_fps);
    let total = cadence.boundary(frames.end)?;
    (project.width, project.height) = options.extent(project.width, project.height);
    let source_ids = video_source_ids(&project, frames.end, &cadence, total, cancel)?;
    if cancel.load(Ordering::Acquire) {
        return Ok(false);
    }
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
    let samples =
        project.fps.frame_to_sample(frames.start)?..project.fps.frame_to_sample(frames.end)?;
    let audio = crate::audio_mix::AudioMix::new(&project, cancel.clone(), Some(&samples))?;
    if let Some(audio) = &audio {
        sources.extend(audio.sources().cloned());
    }
    let audio = audio.map(|mut audio| -> oq_media::AudioRender {
        Box::new(move |start, block| {
            let start = samples
                .start
                .checked_add(start)
                .ok_or("Export audio offset overflow")?;
            // The encoder rounds the output duration independently. If that yields
            // an extra sample, pad it instead of reading past the selected end.
            let count = samples.end.saturating_sub(start).min(block.len() as u64) as usize;
            let (selected, padding) = block.split_at_mut(count);
            padding.fill([0.0; 2]);
            audio.render(start, selected)
        })
    });
    let sources: Vec<_> = sources.into_iter().collect();
    if cancel.load(Ordering::Acquire) {
        return Ok(false);
    }
    let end = frames.end;
    let config = ExportConfig {
        output: path.into(),
        output_node_id: OUTPUT_ID.into(),
        frames: 0..total,
        encoder: EncoderConfig {
            width: project.width,
            height: project.height,
            time_base: RationalTimeBase {
                numerator: output_fps.denominator,
                denominator: output_fps.numerator,
            },
            bitrate: options.bitrate(project.width, project.height),
            queue_capacity: 3,
            max_b_frames: 0,
            overwrite: false,
        },
        input_interpretation: if options.hdr10 { InputInterpretation::Hdr10AcesCgRgba16Float }
            else { InputInterpretation::SdrSrgbRgba8Black },
        date: [0.0; 4],
        mouse: [0.0; 4],
    };
    let sequence = Sequence {
        project,
        revision,
        end,
        cadence,
        hdr10: options.hdr10,
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
