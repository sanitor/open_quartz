use std::collections::VecDeque;
use std::time::Instant;

use super::{FrameCompletion, FrameDemand};

/// Limits include plans, decoding, submitted GPU work and unconsumed terminals.
/// The graph evaluator remains ordered; only independent stages overlap.
#[derive(Clone, Copy, Debug)]
pub struct FrameWindowConfig {
    pub low_water: usize,
    pub high_water: usize,
    pub prefill: usize,
    pub max_span_ns: u64,
    pub max_bytes: u64,
    pub max_surfaces: usize,
}

impl Default for FrameWindowConfig {
    fn default() -> Self {
        Self {
            low_water: 1,
            high_water: 3,
            prefill: 2,
            max_span_ns: 100_000_000,
            max_bytes: 256 * 1024 * 1024,
            max_surfaces: 6,
        }
    }
}

impl FrameWindowConfig {
    pub fn validate(self) -> Result<Self, String> {
        if self.low_water >= self.high_water
            || self.prefill == 0
            || self.prefill > self.high_water
            || self.high_water < 2
            || self.max_span_ns == 0
            || self.max_bytes == 0
            || self.max_surfaces < self.high_water
        {
            return Err("frame window requires low < high, 1 <= prefill <= high, and a bounded minimum working set".into());
        }
        Ok(self)
    }
}

#[derive(Clone, Debug)]
pub struct FrameStageEvent {
    pub request_id: u64,
    pub epoch: u64,
    pub timeline_ns: u64,
    pub elapsed_ns: u64,
    pub at: Instant,
    pub stage: &'static str,
    pub queued: usize,
    pub surfaces: usize,
    pub bytes: u64,
    pub source_requests: usize,
    pub gpu_surfaces: usize,
    pub gpu_bytes: u64,
}

pub(super) struct QueuedFrame {
    pub demand: FrameDemand,
    pub prefetched: bool,
    pub reserved_bytes: u64,
    pub targets: Option<crate::rendering::PreparedFrameTargets>,
}

pub(super) struct FrameWindow {
    pub config: FrameWindowConfig,
    pub queued: VecDeque<QueuedFrame>,
    pub completed: VecDeque<FrameCompletion>,
    pub enabled: bool,
    pub failed: bool,
    pub refill: bool,
    pub last_key: Option<(u64, u64)>,
    pub origin: Instant,
    pub execution_deadline: Option<u64>,
    pub presentation_shift: Option<(u64, u64, u64)>,
    pub events: VecDeque<FrameStageEvent>,
    pub trace_enabled: bool,
}

impl Default for FrameWindow {
    fn default() -> Self {
        Self {
            config: FrameWindowConfig::default(),
            queued: VecDeque::new(),
            completed: VecDeque::new(),
            enabled: false,
            failed: false,
            refill: true,
            last_key: None,
            origin: Instant::now(),
            execution_deadline: None,
            presentation_shift: None,
            events: VecDeque::new(),
            trace_enabled: false,
        }
    }
}

impl super::FramePipeline {
    pub fn configure_window(&mut self, config: FrameWindowConfig) -> Result<(), String> {
        let config = config.validate()?;
        if self.has_in_flight() {
            return Err("cannot reconfigure a nonempty frame window".into());
        }
        if config.prefill >= self.stable_previews.budget().max_frames {
            return Err("frame prefill must leave one stable slot for the retained display".into());
        }
        #[cfg(feature = "native-video")]
        self.videos.set_prefetch_limits(
            config.high_water,
            config.max_surfaces,
            config.max_bytes / 2,
        )?;
        #[cfg(all(target_os = "macos", feature = "native-video"))]
        self.render_device
            .configure_videotoolbox_imports(config.max_surfaces, (config.max_bytes / 2) as usize)?;
        self.window.config = config;
        self.window.enabled = true;
        Ok(())
    }

    pub fn window_config(&self) -> FrameWindowConfig {
        self.window.config
    }

    pub fn window_len(&self) -> usize {
        self.window.queued.len()
            + self.window.completed.len()
            + usize::from(self.pending.is_some())
            + usize::from(self.completion.is_some())
    }

    pub fn window_needs_refill(&mut self) -> bool {
        if self.window_len() <= self.window.config.low_water {
            self.window.refill = true;
        }
        if self.window_len() >= self.window.config.high_water {
            self.window.refill = false;
        }
        self.window.refill
    }

    /// Admission owns bounded plans, not graph evaluator state. It never seeks,
    /// decodes, waits for GPU completion or mutates graph textures.
    pub fn submit_window(&mut self, demand: FrameDemand) -> super::FrameAdmission {
        if !self.window.enabled {
            if let Err(error) = self.configure_window(self.window.config) {
                return super::FrameAdmission::Rejected { demand, error };
            }
        }
        if self.window.failed {
            return super::FrameAdmission::Rejected {
                demand,
                error: "frame window failed; reset its graph or source before admission".into(),
            };
        }
        if let Err(error) = self.validate_demand(&demand) {
            return super::FrameAdmission::Rejected { demand, error };
        }
        let key = (demand.input.stamp.epoch, demand.request_id);
        if self.window.last_key.is_some_and(|previous| key <= previous) {
            return super::FrameAdmission::Rejected {
                demand,
                error: "window epoch/request is stale".into(),
            };
        }
        let (width, height, format) = self.window_output_extent(&demand.output_node_id, 0);
        let pixel_bytes = if format == crate::rendering::TextureFormat::Rgba32Float {
            16
        } else {
            4
        };
        let bytes = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|pixels| pixels.checked_mul(pixel_bytes));
        let Some(bytes) = bytes.filter(|bytes| *bytes <= self.window.config.max_bytes / 2) else {
            return super::FrameAdmission::Rejected {
                demand,
                error: "frame window minimum output working set exceeds byte capacity".into(),
            };
        };
        let first = self
            .window
            .completed
            .front()
            .map(|value| value.stamp.timeline_ns)
            .or_else(|| {
                self.pending
                    .as_ref()
                    .map(|value| value.demand.input.stamp.timeline_ns)
            })
            .or_else(|| {
                self.window
                    .queued
                    .front()
                    .map(|value| value.demand.input.stamp.timeline_ns)
            });
        let reserved: u64 = self
            .window
            .queued
            .iter()
            .map(|value| value.reserved_bytes)
            .sum();
        if self.window_len() >= self.window.config.high_water
            || reserved.saturating_add(bytes) > self.window.config.max_bytes / 2
            || first.is_some_and(|first| {
                demand.input.stamp.timeline_ns.abs_diff(first) > self.window.config.max_span_ns
            })
        {
            return super::FrameAdmission::WouldBlock(demand);
        }
        let budget = self.frame_budget.clone();
        let _admission = match budget.try_admission() {
            Ok(Some(guard)) => guard,
            Ok(None) => return super::FrameAdmission::WouldBlock(demand),
            Err(error) => return super::FrameAdmission::Rejected { demand, error },
        };
        let targets = match self.prepare_demand_targets(&demand) {
            Ok(Some(targets)) => targets,
            Ok(None) => {
                self.record_stage(&demand, "BudgetBlocked");
                return super::FrameAdmission::WouldBlock(demand);
            }
            Err(error) => return super::FrameAdmission::Rejected { demand, error },
        };
        match self
            .stable_previews
            .prepare_slots(width, height, format, self.window.config.prefill)
        {
            Ok(()) => {}
            Err(crate::rendering::FrameCaptureError::WouldBlock) => {
                return super::FrameAdmission::WouldBlock(demand)
            }
            Err(error) => {
                return super::FrameAdmission::Rejected {
                    demand,
                    error: error.to_string(),
                }
            }
        }
        #[cfg(all(target_os = "macos", feature = "native-video"))]
        for node_id in self.videos.node_ids() {
            if let Some((width, height)) = self.videos.dimensions(&node_id) {
                match self
                    .render_device
                    .prepare_video_texture(&node_id, width, height)
                {
                    Ok(true) => {}
                    Ok(false) => return super::FrameAdmission::WouldBlock(demand),
                    Err(error) => {
                        return super::FrameAdmission::Rejected {
                            demand,
                            error: error.to_string(),
                        }
                    }
                }
            }
        }
        #[cfg(feature = "native-video")]
        {
            let excluded = demand
                .sources
                .iter()
                .map(|source| source.node_id.clone())
                .collect::<Vec<_>>();
            match self.videos.prefetch_at(
                demand.request_id,
                demand.input.stamp.timeline_ns,
                demand.input.stamp.epoch,
                &excluded,
            ) {
                Ok(true) => {}
                Ok(false) => return super::FrameAdmission::WouldBlock(demand),
                Err(error) => return super::FrameAdmission::Rejected { demand, error },
            }
        }
        self.record_stage(&demand, "Planned");
        self.window.last_key = Some(key);
        self.window.queued.push_back(super::QueuedFrame {
            demand,
            prefetched: true,
            reserved_bytes: bytes,
            targets: Some(targets),
        });
        super::FrameAdmission::Accepted
    }

    pub fn drain_stage_events(&mut self) -> Vec<FrameStageEvent> {
        self.window.events.drain(..).collect()
    }

    pub fn enable_stage_trace(&mut self, enabled: bool) {
        self.window.trace_enabled = enabled;
        self.stable_previews.set_trace_enabled(enabled);
        #[cfg(feature = "native-video")]
        self.videos.set_trace_enabled(enabled);
        if !enabled {
            self.window.events.clear();
        }
    }

    pub(crate) fn record_presented(&mut self, stamp: &crate::outputs::FrameStamp, late: bool) {
        if !self.window.trace_enabled {
            return;
        }
        let (surfaces, bytes) = self.frame_budget.usage();
        let (source_requests, gpu_surfaces, gpu_bytes) = self.stage_occupancy();
        if self.window.events.len() == 512 {
            self.window.events.pop_front();
        }
        self.window.events.push_back(FrameStageEvent {
            request_id: stamp.frame,
            epoch: stamp.epoch,
            timeline_ns: stamp.timeline_ns,
            elapsed_ns: self
                .window
                .origin
                .elapsed()
                .as_nanos()
                .min(u128::from(u64::MAX)) as u64,
            at: Instant::now(),
            stage: if late { "PresentedLate" } else { "Presented" },
            queued: self.window_len(),
            surfaces,
            bytes,
            source_requests,
            gpu_surfaces,
            gpu_bytes,
        });
    }

    pub(super) fn record_stage(&mut self, demand: &FrameDemand, stage: &'static str) {
        if !self.window.trace_enabled {
            return;
        }
        let (surfaces, bytes) = self.frame_budget.usage();
        let (source_requests, gpu_surfaces, gpu_bytes) = self.stage_occupancy();
        if self.window.events.len() == 512 {
            self.window.events.pop_front();
        }
        self.window.events.push_back(FrameStageEvent {
            request_id: demand.request_id,
            epoch: demand.input.stamp.epoch,
            timeline_ns: demand.input.stamp.timeline_ns,
            elapsed_ns: self
                .window
                .origin
                .elapsed()
                .as_nanos()
                .min(u128::from(u64::MAX)) as u64,
            at: Instant::now(),
            stage,
            queued: self.window_len(),
            surfaces,
            bytes,
            source_requests,
            gpu_surfaces,
            gpu_bytes,
        });
    }

    fn stage_occupancy(&self) -> (usize, usize, u64) {
        #[cfg(feature = "native-video")]
        let sources = self.videos.prefetch_pending_requests();
        #[cfg(not(feature = "native-video"))]
        let sources = 0;
        #[cfg(all(target_os = "macos", feature = "native-video"))]
        let (gpu_surfaces, gpu_bytes) = self.render_device.videotoolbox_pending();
        #[cfg(not(all(target_os = "macos", feature = "native-video")))]
        let (gpu_surfaces, gpu_bytes) = (0, 0);
        (sources, gpu_surfaces, gpu_bytes as u64)
    }

    pub(super) fn collect_window_completion(&mut self) {
        if self.window.enabled {
            if let Some(completion) = self.completion.take() {
                self.window.completed.push_back(completion);
            }
        }
    }

    pub(super) fn progress_window(&mut self) {
        if !self.window.enabled {
            return;
        }
        self.collect_window_completion();
        if self.window.failed {
            while let Some(queued) = self.window.queued.pop_front() {
                self.window.completed.push_back(super::FrameCompletion {
                    request_id: queued.demand.request_id,
                    stamp: queued.demand.input.stamp,
                    graph_revision: queued.demand.graph_revision,
                    outcome: super::FrameOutcome::Cancelled,
                });
            }
            return;
        }
        #[cfg(feature = "native-video")]
        for queued in &mut self.window.queued {
            if queued.prefetched {
                continue;
            }
            let excluded = queued
                .demand
                .sources
                .iter()
                .map(|value| value.node_id.clone())
                .collect::<Vec<_>>();
            match self.videos.prefetch_at(
                queued.demand.request_id,
                queued.demand.input.stamp.timeline_ns,
                queued.demand.input.stamp.epoch,
                &excluded,
            ) {
                Ok(true) => queued.prefetched = true,
                Ok(false) | Err(_) => break, // active admission reports source failure with its terminal
            }
        }
        if self.pending.is_some() {
            return;
        }
        if self.window.execution_deadline.is_some_and(|deadline| {
            self.window.queued.front().is_some_and(|queued| {
                self.presentation_deadline(&queued.demand.input.stamp) > deadline
            })
        }) {
            return;
        }
        let Some(mut queued) = self.window.queued.pop_front() else {
            return;
        };
        let targets = match queued.targets.take() {
            Some(targets) => targets,
            None => match self.prepare_demand_targets(&queued.demand) {
                Ok(Some(targets)) => targets,
                Ok(None) => {
                    self.window.queued.push_front(queued);
                    return;
                }
                Err(error) => {
                    self.finish(queued.demand, super::FrameOutcome::Failed(error));
                    self.collect_window_completion();
                    return;
                }
            },
        };
        match self.submit_prepared(queued.demand, targets) {
            super::FrameAdmission::Accepted => {}
            super::FrameAdmission::Rejected { demand, error } => {
                self.finish(demand, super::FrameOutcome::Failed(error))
            }
            super::FrameAdmission::WouldBlock(demand) => self
                .window
                .queued
                .push_front(super::QueuedFrame { demand, ..queued }),
        }
        self.collect_window_completion();
    }

    /// Clock-free peeking lets the host present only frames whose deadline is due.
    pub fn next_completion_stamp(&mut self) -> Option<&crate::outputs::FrameStamp> {
        self.collect_window_completion();
        self.window.completed.front().map(|value| &value.stamp)
    }

    pub(crate) fn discard_cancelled_completions(&mut self) {
        self.window
            .completed
            .retain(|completion| !matches!(completion.outcome, super::FrameOutcome::Cancelled));
    }

    pub fn window_prefilled(&self) -> bool {
        let mut ready = 0;
        for completion in &self.window.completed {
            match &completion.outcome {
                super::FrameOutcome::Submitted(frame) if frame.ready.is_ready() => ready += 1,
                super::FrameOutcome::Failed(_) => return true,
                _ => {}
            }
        }
        ready >= self.window.config.prefill
    }

    pub(crate) fn delay_window_deadlines(&mut self, delay_ns: u64) {
        let last = self
            .window
            .queued
            .back()
            .map(|queued| &queued.demand.input.stamp)
            .or_else(|| {
                self.pending
                    .as_ref()
                    .map(|pending| &pending.demand.input.stamp)
            })
            .or_else(|| {
                self.window
                    .completed
                    .back()
                    .map(|completion| &completion.stamp)
            });
        if let Some(stamp) = last {
            self.window.presentation_shift = Some((stamp.epoch, stamp.frame, delay_ns));
        }
    }

    pub(crate) fn presentation_deadline(&self, stamp: &crate::outputs::FrameStamp) -> u64 {
        let delay = self
            .window
            .presentation_shift
            .filter(|(epoch, through, _)| stamp.epoch == *epoch && stamp.frame <= *through)
            .map_or(0, |(_, _, delay)| delay);
        stamp.deadline_ns.saturating_add(delay)
    }

    pub(super) fn window_output_extent(
        &self,
        node_id: &str,
        depth: usize,
    ) -> (u32, u32, crate::rendering::TextureFormat) {
        use crate::rendering::TextureFormat;
        let Some(plan) = self.session.execution_plan() else {
            return (1, 1, TextureFormat::Rgba8Unorm);
        };
        if depth > plan.nodes.len() {
            return (
                plan.default_width.max(1),
                plan.default_height.max(1),
                TextureFormat::Rgba8Unorm,
            );
        }
        #[cfg(feature = "native-video")]
        if let Some((width, height)) = self.videos.dimensions(node_id) {
            return (width, height, TextureFormat::Rgba8Unorm);
        }
        let node = plan.nodes.iter().find(|node| node.id == node_id);
        if let Some(target) = node.and_then(|node| node.target.as_ref()) {
            let format = if target.float {
                TextureFormat::Rgba32Float
            } else {
                TextureFormat::Rgba8Unorm
            };
            if let Some(source) = &target.size_source {
                let (width, height, _) = self.window_output_extent(source, depth + 1);
                return (width, height, format);
            }
            return (target.width.max(1), target.height.max(1), format);
        }
        if let Some(node) =
            node.filter(|node| node.node_type == crate::project::NodeType::Inference)
        {
            if let Some(source) = node.upstream.values().next() {
                let (width, height, _) = self.window_output_extent(source, depth + 1);
                if let Some((width, height)) =
                    self.inference.output_dimensions(node_id, width, height)
                {
                    return (width, height, TextureFormat::Rgba8Unorm);
                }
            }
        }
        if let Some(source) = node.and_then(|node| node.upstream.values().next()) {
            return self.window_output_extent(source, depth + 1);
        }
        self.render_device.output_texture(node_id).map_or(
            (
                plan.default_width.max(1),
                plan.default_height.max(1),
                TextureFormat::Rgba8Unorm,
            ),
            |texture| (texture.width, texture.height, texture.format),
        )
    }

    pub(super) fn prepare_demand_targets(
        &mut self,
        demand: &FrameDemand,
    ) -> Result<Option<crate::rendering::PreparedFrameTargets>, String> {
        let plan = self
            .session
            .execution_plan()
            .ok_or("Frame pipeline has no graph")?;
        let mut sizes = std::collections::HashMap::new();
        for node in &plan.nodes {
            #[cfg(feature = "native-video")]
            if let Some(size) = self.videos.dimensions(&node.id) {
                sizes.insert(node.id.clone(), size);
            }
            if let Some(source) = demand
                .sources
                .iter()
                .find(|source| source.node_id == node.id)
            {
                sizes.insert(
                    node.id.clone(),
                    (source.frame.image.width, source.frame.image.height),
                );
            }
            if !sizes.contains_key(&node.id) {
                if let Some(texture) = self.render_device.output_texture(&node.id) {
                    sizes.insert(node.id.clone(), (texture.width, texture.height));
                }
            }
            let upstream = node
                .upstream
                .values()
                .next()
                .and_then(|source| sizes.get(source))
                .copied();
            if node.node_type == crate::project::NodeType::Inference {
                if let Some((width, height)) = upstream {
                    if self.inference.is_loaded(&node.id)
                        && !self
                            .inference
                            .prepare_frame_budget(&node.id, width, height)?
                    {
                        return Ok(None);
                    }
                    if let Some(size) = self.inference.output_dimensions(&node.id, width, height) {
                        sizes.insert(node.id.clone(), size);
                    }
                }
            } else if let Some(target) = &node.target {
                let size = target
                    .size_source
                    .as_ref()
                    .and_then(|source| sizes.get(source))
                    .copied()
                    .unwrap_or((target.width, target.height));
                sizes.insert(node.id.clone(), size);
            } else if let Some(size) = upstream {
                sizes.insert(node.id.clone(), size);
            }
        }
        self.render_device
            .try_prepare_frame_target_plan(plan, &sizes)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn progress_due(&mut self, deadline_ns: u64) {
        self.window.execution_deadline = Some(deadline_ns);
        self.progress();
        self.window.execution_deadline = None;
    }

    pub(crate) fn first_completion_ready(&self) -> bool {
        self.window
            .completed
            .front()
            .is_some_and(|completion| match &completion.outcome {
                super::FrameOutcome::Submitted(frame) => frame.ready.is_ready(),
                super::FrameOutcome::Failed(_) => true,
                super::FrameOutcome::Cancelled => false,
            })
    }
}
