//! Clock-free, bounded hardware MP4 file consumer of the shared FramePipeline.
use super::{FrameAdmission, FrameDemand, FrameEvaluationInput, FrameOutcome, FramePipeline};
use crate::outputs::FrameStamp;
use crate::rendering::{
    ConversionAdmission, ConvertedFrame, InputInterpretation, Nv12Converter, StableFrame,
    TextureFormat,
};
use crate::resources::source_identity::SourceFileIdentity;
use oq_media::{
    EncodeAdmission, EncodeFrame, EncoderConfig, EncoderReport, EncoderStatus, Mp4Encoder,
};
use std::collections::VecDeque;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct ExportConfig {
    pub output: PathBuf,
    pub output_node_id: String,
    /// Half-open sequence-frame range; source samples retain this original origin.
    pub frames: Range<u64>,
    pub encoder: EncoderConfig,
    pub input_interpretation: InputInterpretation,
    /// Frozen controls, never sampled from a playback or wall clock during export.
    pub date: [f32; 4],
    pub mouse: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportPhase {
    Preparing,
    Rendering,
    Draining,
    ReadyToCommit,
    Committing,
    Completed,
    Cancelling,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug)]
pub struct ExportStatus {
    pub phase: ExportPhase,
    pub total_frames: u64,
    pub rendered_frames: u64,
    pub accepted_frames: u64,
    pub report: Option<Arc<EncoderReport>>,
    pub error: Option<Arc<str>>,
}
impl ExportStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.phase,
            ExportPhase::Completed | ExportPhase::Cancelled | ExportPhase::Failed
        )
    }
}

#[derive(Clone, Debug)]
pub struct ExportStageEvent {
    pub output_frame: u64,
    pub actual_source_pts_ns: Option<i64>,
    pub stage: &'static str,
    pub at: Instant,
    pub surfaces: usize,
    pub bytes: u64,
}

/// Owns one frozen pipeline/session and a separate codec/file worker. `poll`
/// progresses bounded work without waiting for codec, disk, or GPU completion.
/// Drive it on the job's render owner, not an application UI callback.
/// Completed means drain + trailer + fsync + atomic commit, not accepted input.
pub struct ExportJob {
    pipeline: FramePipeline,
    converter: Nv12Converter,
    encoder: Mp4Encoder,
    config: ExportConfig,
    identities: Vec<SourceFileIdentity>,
    frames: VecDeque<(u64, StableFrame)>,
    converted: VecDeque<(u64, ConvertedFrame)>,
    next_request: u64,
    epoch: u64,
    drain_requested: bool,
    commit_requested: bool,
    pending_error: Option<Arc<str>>,
    status: ExportStatus,
    trace_enabled: bool,
    events: VecDeque<ExportStageEvent>,
}

impl ExportJob {
    pub fn new(mut pipeline: FramePipeline, config: ExportConfig) -> Result<Self, String> {
        if pipeline.has_in_flight() || !pipeline.has_graph() {
            return Err("export requires a prepared, idle graph session".into());
        }
        if config.frames.start >= config.frames.end {
            return Err("export frame range must be nonempty and half-open".into());
        }
        let total = config.frames.end - config.frames.start;
        if total > i64::MAX as u64 {
            return Err("export range exceeds encoder timestamp capacity".into());
        }
        sample_ns(config.frames.end - 1, config.encoder.time_base)?;
        if config
            .date
            .iter()
            .chain(&config.mouse)
            .any(|value| !value.is_finite())
        {
            return Err("export controls must be finite".into());
        }
        let plan = pipeline
            .session
            .execution_plan()
            .ok_or("export has no graph")?;
        if !plan
            .nodes
            .iter()
            .any(|node| node.id == config.output_node_id)
        {
            return Err("export output node does not exist".into());
        }
        for node in &plan.nodes {
            if node.node_type == crate::project::NodeType::Inference
                && !pipeline.inference.is_loaded(&node.id)
            {
                return Err(format!("export inference {} is not loaded", node.id));
            }
        }
        let (width, height, format) = pipeline.window_output_extent(&config.output_node_id, 0);
        if format != TextureFormat::Rgba8Unorm
            || (width, height) != (config.encoder.width, config.encoder.height)
        {
            return Err(format!("export output {width}x{height} {format:?} disagrees with encoder {}x{} RGBA8 input", config.encoder.width, config.encoder.height));
        }
        let identities = pipeline.videos.file_identities();
        if identities.len() != pipeline.videos.node_ids().len() {
            return Err(
                "offline export requires precise local file sources with frozen identities".into(),
            );
        }
        for identity in &identities {
            identity.validate()?;
            if identity.is_target(&config.output)? {
                return Err("export target must not replace its source material".into());
            }
        }
        let epoch = pipeline.last_key.map_or(Ok(1), |(epoch, _)| {
            epoch.checked_add(1).ok_or("export epoch overflow")
        })?;
        pipeline.pause_execution()?;
        pipeline.discard_cancelled_completions();
        pipeline.resume_execution()?;
        let mut window = pipeline.window_config();
        let period = sample_ns(1, config.encoder.time_base)?;
        window.max_span_ns = period
            .checked_mul(window.high_water as u64)
            .ok_or("export window span overflow")?
            .max(1);
        pipeline.configure_window(window)?;
        let mut converter = Nv12Converter::new(
            pipeline.backend(),
            config.encoder.queue_capacity,
            window.max_bytes / 2,
        )?;
        converter.set_input_interpretation(config.input_interpretation);
        // Reserve the downstream native working set before starting source work.
        if !converter.prepare_surfaces(width, height, config.encoder.queue_capacity)? {
            return Err(
                "shared frame budget cannot currently reserve the export encoder working set"
                    .into(),
            );
        }
        let encoder =
            Mp4Encoder::open(&config.output, config.encoder.clone()).map_err(|e| e.to_string())?;
        Ok(Self {
            pipeline,
            converter,
            encoder,
            config,
            identities,
            frames: VecDeque::with_capacity(window.high_water),
            converted: VecDeque::with_capacity(window.high_water),
            next_request: 0,
            epoch,
            drain_requested: false,
            commit_requested: false,
            pending_error: None,
            status: ExportStatus {
                phase: ExportPhase::Preparing,
                total_frames: total,
                rendered_frames: 0,
                accepted_frames: 0,
                report: None,
                error: None,
            },
            trace_enabled: false,
            events: VecDeque::new(),
        })
    }

    pub fn set_waker(&mut self, wake: Arc<dyn Fn() + Send + Sync>) -> Result<(), String> {
        self.pipeline.set_waker(wake.clone())?;
        self.converter.set_waker(wake.clone());
        self.encoder.set_waker(wake);
        Ok(())
    }

    pub fn enable_stage_trace(&mut self, enabled: bool) {
        self.trace_enabled = enabled;
        self.pipeline.enable_stage_trace(enabled);
        self.converter.set_trace_enabled(enabled);
        if !enabled {
            self.events.clear();
        }
    }
    pub fn drain_stage_events(&mut self) -> Vec<ExportStageEvent> {
        self.events.drain(..).collect()
    }
    pub fn status(&self) -> ExportStatus {
        self.status.clone()
    }
    pub fn frame_budget_usage(&self) -> (usize, u64) {
        self.pipeline.frame_budget_usage()
    }

    pub fn cancel(&mut self) {
        if self.status.is_terminal() {
            return;
        }
        self.pipeline.cancel_pending();
        self.frames.clear();
        self.converted.clear();
        self.encoder.cancel();
        self.status.phase = ExportPhase::Cancelling;
    }

    pub fn poll(&mut self) -> ExportStatus {
        if self.status.is_terminal() {
            return self.status();
        }
        if let Err(error) = self.advance() {
            self.pipeline.cancel_pending();
            self.frames.clear();
            self.converted.clear();
            self.encoder.cancel();
            self.status.phase = ExportPhase::Cancelling;
            self.pending_error = Some(error.into());
        }
        self.status()
    }

    fn advance(&mut self) -> Result<(), String> {
        let encoder = self.encoder.poll().map_err(|e| e.to_string())?;
        match encoder {
            EncoderStatus::Failed(error) => {
                self.status.phase = ExportPhase::Failed;
                self.status.error = self
                    .pending_error
                    .take()
                    .or_else(|| Some(error.to_string().into()));
                return Ok(());
            }
            EncoderStatus::Cancelled => {
                self.status.error = self.pending_error.take();
                self.status.phase = if self.status.error.is_some() {
                    ExportPhase::Failed
                } else {
                    ExportPhase::Cancelled
                };
                return Ok(());
            }
            EncoderStatus::Completed(report) => {
                if report.input_frames != self.status.total_frames
                    || self.status.accepted_frames != self.status.total_frames
                {
                    return Err("encoder committed an incomplete frame range".into());
                }
                self.status.report = Some(Arc::new(report));
                self.status.phase = ExportPhase::Completed;
                return Ok(());
            }
            EncoderStatus::ReadyToCommit(_) => {
                if self.status.phase == ExportPhase::Cancelling {
                    return Ok(());
                }
                self.validate_sources()?;
                self.status.phase = ExportPhase::ReadyToCommit;
                if self.encoder.commit().map_err(|e| e.to_string())? {
                    self.commit_requested = true;
                    self.status.phase = ExportPhase::Committing;
                }
                return Ok(());
            }
            EncoderStatus::Draining => self.status.phase = ExportPhase::Draining,
            EncoderStatus::Running => {}
        }
        if self.status.phase == ExportPhase::Cancelling || self.commit_requested {
            return Ok(());
        }
        self.pipeline
            .render_device
            .device()
            .poll(wgpu::PollType::Poll)
            .map_err(|e| e.to_string())?;
        self.validate_sources()?;
        if self.drain_requested {
            return Ok(());
        }
        self.status.phase = ExportPhase::Rendering;

        // Preserve output order across GPU completion and codec backpressure.
        if self
            .converted
            .front()
            .is_some_and(|(_, frame)| frame.ready.is_ready())
        {
            let (index, frame) = self.converted.pop_front().expect("ready front");
            let input = EncodeFrame {
                surface: frame.surface,
                pts: i64::try_from(index).map_err(|_| "encoder PTS overflow")?,
                duration: 1,
            };
            match self.encoder.submit(input).map_err(|e| e.to_string())? {
                EncodeAdmission::Accepted => {
                    self.status.accepted_frames += 1;
                    self.record(index, None, "EncoderAccepted");
                }
                EncodeAdmission::WouldBlock(input) => {
                    self.converted.push_front((
                        index,
                        ConvertedFrame {
                            surface: input.surface,
                            ready: frame.ready,
                        },
                    ));
                }
            }
        }
        if self.converted.len() < self.config.encoder.queue_capacity {
            if let Some((index, frame)) = self.frames.pop_front() {
                let pts = frame
                    .identity
                    .content_stamp
                    .as_ref()
                    .and_then(|stamp| stamp.media_pts_ns);
                match self.converter.try_convert(frame)? {
                    ConversionAdmission::Accepted(frame) => {
                        self.record(index, pts, "Nv12Submitted");
                        self.converted.push_back((index, frame));
                    }
                    ConversionAdmission::WouldBlock(frame) => {
                        self.frames.push_front((index, frame))
                    }
                }
            }
        }
        if self.frames.len() + self.converted.len() < self.pipeline.window_config().high_water {
            if let Some(completion) = self.pipeline.poll_completion() {
                let index = completion
                    .request_id
                    .checked_sub(1)
                    .ok_or("invalid export request identity")?;
                match completion.outcome {
                    FrameOutcome::Submitted(frame) => {
                        if (frame.image.width, frame.image.height)
                            != (self.config.encoder.width, self.config.encoder.height)
                        {
                            return Err("export output dimensions changed after preparation".into());
                        }
                        let pts = frame
                            .identity
                            .content_stamp
                            .as_ref()
                            .and_then(|stamp| stamp.media_pts_ns);
                        self.record(index, pts, "GraphSubmitted");
                        self.status.rendered_frames += 1;
                        self.frames.push_back((index, frame));
                    }
                    FrameOutcome::Cancelled => {
                        return Err("export frame cancelled before completion".into())
                    }
                    FrameOutcome::Failed(error) => return Err(error),
                }
            }
        }
        if self.next_request < self.status.total_frames && self.pipeline.window_needs_refill() {
            for _ in 0..self.pipeline.window_config().high_water {
                if self.next_request == self.status.total_frames {
                    break;
                }
                let sequence_frame = self
                    .config
                    .frames
                    .start
                    .checked_add(self.next_request)
                    .ok_or("export sequence-frame overflow")?;
                let timeline_ns = sample_ns(sequence_frame, self.config.encoder.time_base)?;
                let previous_timeline_ns = if self.next_request == 0 {
                    timeline_ns
                } else {
                    sample_ns(sequence_frame - 1, self.config.encoder.time_base)?
                };
                let demand = FrameDemand {
                    request_id: self.next_request + 1,
                    graph_revision: self.pipeline.revision(),
                    output_node_id: self.config.output_node_id.clone(),
                    sources: Vec::new(),
                    input: FrameEvaluationInput {
                        stamp: FrameStamp {
                            epoch: self.epoch,
                            frame: self.next_request + 1,
                            timeline_ns,
                            deadline_ns: 0,
                        },
                        previous_timeline_ns,
                        date: self.config.date,
                        mouse: self.config.mouse,
                        resolution: [
                            self.config.encoder.width as f32,
                            self.config.encoder.height as f32,
                            1.0,
                        ],
                    },
                };
                match self.pipeline.submit_window(demand) {
                    FrameAdmission::Accepted => self.next_request += 1,
                    FrameAdmission::WouldBlock(_) => break,
                    FrameAdmission::Rejected { error, .. } => return Err(error),
                }
            }
        }
        self.pipeline.progress();
        if self.status.accepted_frames == self.status.total_frames
            && self.frames.is_empty()
            && self.converted.is_empty()
        {
            self.validate_sources()?;
            if self.encoder.finish().map_err(|e| e.to_string())? {
                self.drain_requested = true;
                self.status.phase = ExportPhase::Draining;
            }
        }
        Ok(())
    }

    fn validate_sources(&self) -> Result<(), String> {
        for source in &self.identities {
            source.validate()?;
        }
        Ok(())
    }
    fn record(&mut self, frame: u64, pts: Option<i64>, stage: &'static str) {
        if !self.trace_enabled {
            return;
        }
        if self.events.len() == 512 {
            self.events.pop_front();
        }
        let (surfaces, bytes) = self.pipeline.frame_budget_usage();
        self.events.push_back(ExportStageEvent {
            output_frame: frame,
            actual_source_pts_ns: pts,
            stage,
            at: Instant::now(),
            surfaces,
            bytes,
        });
    }
}

fn sample_ns(frame: u64, time_base: oq_media::RationalTimeBase) -> Result<u64, String> {
    if time_base.numerator <= 0 || time_base.denominator <= 0 {
        return Err("export time base must be positive".into());
    }
    let value = u128::from(frame)
        .checked_mul(time_base.numerator as u128)
        .and_then(|value| value.checked_mul(1_000_000_000))
        .and_then(|value| value.checked_add(time_base.denominator as u128 / 2))
        .ok_or("export frame-to-time overflow")?
        / time_base.denominator as u128;
    u64::try_from(value).map_err(|_| "export source time exceeds nanosecond capacity".into())
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;
