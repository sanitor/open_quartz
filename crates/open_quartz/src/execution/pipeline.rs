use super::{FrameEvaluationInput, FrameExecutionSession};
use crate::evaluation::{EngineState, ExecutionCommand};
use crate::inference::tasks::{InferenceMetrics, InferenceOutputEvent, InferenceTasks};
use crate::inference::InferenceConfig;
#[cfg(feature = "native-video")]
use crate::media::{
    native::{NativeVideoConfig, NativeVideoFrame, NativeVideoInfo, NativeVideoMetrics},
    sources::VideoSources,
};
use crate::outputs::{
    ContentStamp, FrameAlphaMode, FrameColorSpace, FrameSnapshotIdentity, FrameStamp,
    PlaybackCapabilities,
};
use crate::project::{Graph, NodeType, SdkError};
#[cfg(windows)]
use crate::rendering::SharedTextureFrame;
use crate::rendering::{
    FrameCaptureError, GpuBackend, GpuOutputHandle, RenderDevice, StableFrame, StableFrameBudget,
    StableFramePool,
};
use std::collections::HashMap;
use std::sync::Arc;

#[path = "window.rs"]
mod window;
pub use window::{FrameStageEvent, FrameWindowConfig};
use window::{FrameWindow, QueuedFrame};

#[cfg(all(target_os = "macos", feature = "native-video"))]
#[path = "export.rs"]
mod export;
#[cfg(all(target_os = "macos", feature = "native-video"))]
pub use export::{ExportConfig, ExportJob, ExportPhase, ExportStageEvent, ExportStatus};

fn runtime_error(error: SdkError) -> String {
    error.to_json()
}

/// A validated graph and its GPU resources, ready to commit on the preparing device.
pub struct PreparedNativeGraph {
    pub(crate) graph: Graph,
    plan: crate::compiler::ExecutionPlan,
    pub(crate) rendering: crate::rendering::PreparedRenderPlan,
    pub(crate) backend: Arc<GpuBackend>,
    pub(crate) dirty_images: Vec<String>,
    #[cfg(feature = "native-video")]
    pub(crate) videos: Vec<(String, crate::media::sources::PreparedVideoSource)>,
    #[cfg(feature = "native-video")]
    pub(crate) detached_videos: Vec<String>,
}

impl PreparedNativeGraph {
    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    pub fn execution_plan(&self) -> &crate::compiler::ExecutionPlan {
        &self.plan
    }

    pub fn stage_image(
        &mut self,
        node_id: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        if !self.graph.nodes.iter().any(|node| node.id == node_id) {
            return Err(format!(
                "Image node {node_id} does not exist in the prepared graph"
            ));
        }
        self.rendering
            .stage_image(node_id, rgba, width, height)
            .map_err(|error| error.to_string())?;
        self.dirty_images.push(node_id.to_owned());
        Ok(())
    }

    pub fn stage_remove_texture(&mut self, node_id: &str) {
        self.rendering.stage_remove_texture(node_id);
        if self.graph.nodes.iter().any(|node| node.id == node_id) {
            self.dirty_images.push(node_id.to_owned());
        }
    }

    #[cfg(feature = "native-video")]
    pub fn stage_video(
        &mut self,
        node_id: &str,
        config: NativeVideoConfig,
        preview_only: bool,
    ) -> Result<NativeVideoInfo, String> {
        if !self.graph.nodes.iter().any(|node| {
            node.id == node_id
                && node.node_type == NodeType::Input
                && node.data.input_mode == Some(crate::project::model::InputMode::Video)
        }) {
            return Err(format!(
                "Video node {node_id} does not exist in the prepared graph"
            ));
        }
        let mut source =
            crate::media::sources::PreparedVideoSource::open_precise(config, preview_only)?;
        source.set_frame_budget(self.backend.frame_budget())?;
        let info = source.info().clone();
        self.stage_remove_texture(node_id);
        self.videos.retain(|(id, _)| id != node_id);
        self.videos.push((node_id.to_owned(), source));
        Ok(info)
    }

    #[cfg(feature = "native-video")]
    pub fn prewarm_videos(&mut self, timeline_ns: u64, epoch: u64) -> Result<bool, String> {
        let mut ready = true;
        for (_, source) in &mut self.videos {
            ready &= source.prefetch_at(timeline_ns, epoch)?;
        }
        Ok(ready)
    }

    #[cfg(feature = "native-video")]
    pub fn stage_detach_video(&mut self, node_id: &str) {
        self.stage_remove_texture(node_id);
        self.videos.retain(|(id, _)| id != node_id);
        self.detached_videos.push(node_id.to_owned());
    }
}

/// A source snapshot is immutable and belongs to the same shared GPU owner.
/// A rejected demand returns all of these leases unchanged to the caller.
pub struct ExternalFrameInput {
    pub node_id: String,
    pub version: u64,
    pub frame: StableFrame,
}

pub struct FrameDemand {
    pub request_id: u64,
    pub graph_revision: u32,
    pub input: FrameEvaluationInput,
    pub output_node_id: String,
    pub sources: Vec<ExternalFrameInput>,
}

pub enum FrameAdmission {
    Accepted,
    WouldBlock(FrameDemand),
    Rejected { demand: FrameDemand, error: String },
}

/// Submitted means all graph dependencies and the immutable output copy have
/// been enqueued. Physical GPU completion is carried separately by frame.ready.
pub enum FrameOutcome {
    Submitted(StableFrame),
    Cancelled,
    Failed(String),
}

pub struct FrameCompletion {
    pub request_id: u64,
    pub stamp: FrameStamp,
    pub graph_revision: u32,
    pub outcome: FrameOutcome,
}

struct PendingFrame {
    demand: FrameDemand,
    commands: Vec<ExecutionCommand>,
    next: usize,
    started: bool,
    waiting_inference: Option<String>,
    sources_uploaded: bool,
    media_pts_ns: Option<i64>,
}

/// Clock-free bounded source/GPU/output pipeline. Graph/feedback evaluation is
/// ordered; decoding later demands overlaps submitted GPU and retained outputs.
pub struct FramePipeline {
    pub(crate) render_device: RenderDevice,
    pub(crate) session: FrameExecutionSession,
    #[cfg(feature = "native-video")]
    pub(crate) videos: VideoSources,
    pub(crate) inference: InferenceTasks,
    inference_sampler: Arc<wgpu::Sampler>,
    inference_available: bool,
    pub(crate) stable_previews: StableFramePool,
    pending: Option<PendingFrame>,
    completion: Option<FrameCompletion>,
    latest: Option<StableFrame>,
    last_key: Option<(u64, u64)>,
    inference_ready: HashMap<String, FrameStamp>,
    input_versions: HashMap<String, u64>,
    window: FrameWindow,
    frame_budget: Arc<crate::resources::FrameBudget>,
    budget_wake: Option<crate::resources::FrameWakeRegistration>,
}

impl FramePipeline {
    pub fn from_render_device(
        render_device: RenderDevice,
        capabilities: PlaybackCapabilities,
    ) -> Self {
        let inference_available =
            cfg!(target_os = "macos") && render_device.info().backend == "Metal";
        let inference_sampler =
            Arc::new(render_device.device().create_sampler(&Default::default()));
        let frame_budget = render_device.backend().frame_budget();
        let stable_previews = StableFramePool::with_frame_budget(
            render_device.backend(), StableFrameBudget::default(), frame_budget.clone())
            .expect("default stable preview budget is nonzero");
        #[cfg(feature = "native-video")]
        let mut videos = VideoSources::new();
        #[cfg(feature = "native-video")]
        videos.set_frame_budget(frame_budget.clone()).expect("new source set is empty");
        #[cfg(all(target_os = "macos", feature = "native-video"))]
        render_device.set_videotoolbox_budget(frame_budget.clone()).expect("backend coordinator is shared");
        Self {
            render_device,
            session: FrameExecutionSession::new_native(capabilities),
            #[cfg(feature = "native-video")]
            videos,
            inference: InferenceTasks::new(),
            inference_sampler,
            inference_available,
            stable_previews,
            pending: None,
            completion: None,
            latest: None,
            last_key: None,
            inference_ready: HashMap::new(),
            input_versions: HashMap::new(),
            window: FrameWindow::default(),
            frame_budget,
            budget_wake: None,
        }
    }

    pub fn from_render_device_with_budget(
        render_device: RenderDevice,
        capabilities: PlaybackCapabilities,
        budget: Arc<crate::resources::FrameBudget>,
    ) -> Result<Self, String> {
        render_device.backend().set_frame_budget(budget)?;
        Ok(Self::from_render_device(render_device, capabilities))
    }

    /// Notification only: enqueue owner work, never run rendering/codec work
    /// inline. Callbacks can originate in native resource destructors.
    pub fn set_waker(&mut self, waker: Arc<dyn Fn() + Send + Sync>) -> Result<(), String> {
        #[cfg(feature = "native-video")]
        self.videos.set_waker(waker.clone());
        self.stable_previews.set_waker(waker.clone());
        self.budget_wake = Some(self.frame_budget.register_waker(waker.clone()));
        #[cfg(all(target_os = "macos", feature = "native-video"))]
        self.render_device.set_videotoolbox_waker(waker)?;
        Ok(())
    }

    pub fn frame_budget_usage(&self) -> (usize, u64) { self.frame_budget.usage() }


    #[cfg(feature = "native-video")]
    pub fn drain_source_stage_events(&mut self) -> Vec<crate::media::sources::SourceStageEvent> {
        self.videos.drain_prefetch_events()
    }

    pub fn set_graph(&mut self, graph_json: &str) -> Result<u32, String> {
        let graph: Graph = serde_json::from_str(graph_json)
            .map_err(|e| format!("Invalid native graph JSON: {e}"))?;
        self.set_graph_snapshot(&graph)
    }
    pub fn set_graph_snapshot(&mut self, graph: &Graph) -> Result<u32, String> {
        let prepared = self.prepare_graph(graph)?;
        self.commit_graph(prepared)
    }
    pub fn prepare_graph(&self, graph: &Graph) -> Result<PreparedNativeGraph, String> {
        for node in graph
            .nodes
            .iter()
            .filter(|node| node.node_type == NodeType::Inference)
        {
            let data = serde_json::to_value(&node.data).map_err(|error| {
                runtime_error(
                    SdkError::new(
                        crate::project::model::SdkErrorCode::InvalidResource,
                        format!("Cannot serialize inference policy: {error}"),
                    )
                    .for_node(&node.id),
                )
            })?;
            let policy = crate::resources::effective_inference_policy(&data).map_err(|error| {
                runtime_error(
                    SdkError::new(crate::project::model::SdkErrorCode::InvalidResource, error)
                        .for_node(&node.id),
                )
            })?;
            crate::inference::prepared::validate_resident_policy(&policy).map_err(|error| {
                runtime_error(
                    SdkError::new(crate::project::model::SdkErrorCode::InvalidResource, error)
                        .for_node(&node.id),
                )
            })?;
            if !self.inference_available {
                return Err(format!(
                    "Inference node {} requires the supported Metal GPU backend",
                    node.id
                ));
            }
        }
        #[cfg(not(feature = "native-video"))]
        if graph.nodes.iter().any(|node| {
            node.node_type == NodeType::Input
                && node.data.input_mode == Some(crate::project::model::InputMode::Video)
        }) {
            return Err(
                "Native video graph execution requires the native-video feature".to_owned(),
            );
        }
        graph.validate().map_err(runtime_error)?;
        let plan =
            crate::compiler::build_execution_plan_with_options(&graph.nodes, &graph.edges, false);
        if let Some((node, error)) = plan
            .nodes
            .iter()
            .find_map(|node| node.validation_errors.first().map(|error| (node, error)))
        {
            return Err(runtime_error(
                SdkError::new(
                    crate::project::model::SdkErrorCode::InvalidResource,
                    error.message.clone(),
                )
                .for_node(&node.id),
            ));
        }
        let prepared = self
            .render_device
            .prepare_plan(&plan, graph)
            .map_err(|error| {
                let mut sdk_error = SdkError::new(
                    crate::project::model::SdkErrorCode::InvalidResource,
                    error.message,
                );
                sdk_error.node_id = error.node_id;
                runtime_error(sdk_error)
            })?;
        #[cfg(feature = "native-video")]
        let prepared = {
            let mut prepared = prepared;
            for node_id in self.videos.node_ids() {
                if !graph.nodes.iter().any(|node| {
                    node.id == node_id
                        && node.node_type == NodeType::Input
                        && node.data.input_mode == Some(crate::project::model::InputMode::Video)
                }) {
                    prepared.stage_remove_texture(&node_id);
                }
            }
            prepared
        };
        Ok(PreparedNativeGraph {
            graph: graph.clone(),
            plan,
            rendering: prepared,
            backend: self.render_device.backend(),
            dirty_images: Vec::new(),
            #[cfg(feature = "native-video")]
            videos: Vec::new(),
            #[cfg(feature = "native-video")]
            detached_videos: Vec::new(),
        })
    }

    pub fn upload_image(
        &mut self,
        node_id: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        self.session
            .node_generation(node_id)
            .map_err(runtime_error)?;
        self.cancel_pending();
        self.latest = None;
        self.bump_input(node_id);
        self.render_device
            .upload_rgba(node_id, rgba, width, height)
            .map_err(|error| error.to_string())?;
        self.session.mark_dirty(node_id).map_err(runtime_error)
    }

    pub fn remove_texture(&mut self, node_id: &str) {
        self.cancel_pending();
        self.latest = None;
        self.bump_input(node_id);
        self.render_device.remove_texture(node_id);
        if self.session.node_generation(node_id).is_ok() {
            self.session
                .mark_dirty(node_id)
                .expect("active graph node remains prepared");
        }
    }

    pub fn read_output(&self, node_id: &str) -> Result<Vec<u8>, String> {
        self.render_device.read_output(node_id)
    }

    pub fn preview_source(&self, node_id: &str) -> Result<GpuOutputHandle, String> {
        self.render_device.preview_source(node_id)
    }

    pub fn try_stable_preview(
        &self,
        node_id: &str,
    ) -> Result<Option<StableFrame>, FrameCaptureError> {
        if !self.window.enabled && self.pending.as_ref()
            .is_some_and(|pending| pending.demand.output_node_id == node_id) {
            return Ok(None);
        }
        if let Some(frame) = self.latest.as_ref().filter(|frame| {
            frame.identity.output_node_id == node_id
                && frame.identity.graph_revision == self.session.revision()
        }) {
            return Ok(Some(frame.clone()));
        }
        let source = self.render_device.output_texture(node_id).ok_or_else(|| {
            FrameCaptureError::InvalidSource(format!("output {node_id} has no texture"))
        })?;
        if source.format != crate::rendering::TextureFormat::Rgba8Unorm {
            return Err(FrameCaptureError::InvalidSource(
                "native preview requires rgba8unorm".into(),
            ));
        }
        let budget = self.stable_previews.budget();
        let pair_bytes = u64::from(source.width)
            .checked_mul(u64::from(source.height))
            .and_then(|pixels| pixels.checked_mul(8));
        if budget.max_frames < 2 || pair_bytes.is_none_or(|bytes| bytes > budget.max_bytes) {
            return Err(FrameCaptureError::Capacity);
        }
        let identity = FrameSnapshotIdentity {
            output_node_id: node_id.to_owned(),
            graph_revision: self.session.revision(),
            evaluation_stamp: None,
            content_stamp: None,
            color_space: FrameColorSpace::Unknown,
            alpha_mode: FrameAlphaMode::Unknown,
        };
        match self
            .stable_previews
            .try_capture(&self.render_device, node_id, identity)
        {
            Ok(frame) => Ok(Some(frame)),
            Err(FrameCaptureError::WouldBlock) => Ok(None),
            Err(error) => Err(error),
        }
    }

    #[cfg(feature = "native-video")]
    pub fn attach_video(
        &mut self,
        node_id: &str,
        config: NativeVideoConfig,
    ) -> Result<NativeVideoInfo, String> {
        self.attach_video_source(node_id, config, false)
    }

    #[cfg(feature = "native-video")]
    pub fn attach_video_preview(
        &mut self,
        node_id: &str,
        config: NativeVideoConfig,
    ) -> Result<NativeVideoInfo, String> {
        self.attach_video_source(node_id, config, true)
    }

    #[cfg(feature = "native-video")]
    fn attach_video_source(
        &mut self,
        node_id: &str,
        config: NativeVideoConfig,
        preview_only: bool,
    ) -> Result<NativeVideoInfo, String> {
        self.session
            .node_generation(node_id)
            .map_err(runtime_error)?;
        let source =
            crate::media::sources::PreparedVideoSource::open_precise(config, preview_only)?;
        self.cancel_pending();
        self.latest = None;
        self.bump_input(node_id);
        self.render_device.remove_texture(node_id);
        let info = self.videos.attach_prepared(node_id, source);
        self.sync_video_nodes()?;
        self.session.mark_dirty(node_id).map_err(runtime_error)?;
        Ok(info)
    }

    #[cfg(feature = "native-video")]
    pub fn detach_video(&mut self, node_id: &str) -> Result<(), String> {
        self.cancel_pending();
        self.latest = None;
        self.bump_input(node_id);
        self.videos.detach(node_id);
        self.render_device.remove_texture(node_id);
        self.sync_video_nodes()
    }

    #[cfg(feature = "native-video")]
    fn sync_video_nodes(&mut self) -> Result<(), String> {
        self.session
            .set_video_nodes(&self.videos.node_ids())
            .map_err(runtime_error)
    }

    #[cfg(feature = "native-video")]
    fn upload_current_sources(&mut self) -> Result<(), String> {
        self.render_device.device().poll(wgpu::PollType::Poll).map_err(|e| e.to_string())?;
        if !self.videos.prefetch_at(u64::MAX, 0, 0, &[])? { return Ok(()); }
        let render_device = &mut self.render_device;
        let ready = self.videos.poll_upload_at(u64::MAX, |node_id, frame, _, _| match frame {
            NativeVideoFrame::Rgba(_) => Err("Hardware-resident video required; CPU upload forbidden".into()),
            #[cfg(target_os = "macos")]
            NativeVideoFrame::VideoToolbox(frame) => render_device.try_upload_videotoolbox(node_id, frame).map_err(|e| e.to_string()),
            #[cfg(all(windows, feature = "native-video-d3d12"))]
            NativeVideoFrame::D3d12(frame) => render_device.upload_d3d12_yuv(node_id, frame).map(|_| true).map_err(|e| e.to_string()),
        })?;
        if ready.is_some() {
            for node_id in self.videos.node_ids() {
                self.bump_input(&node_id);
                self.session.mark_dirty(&node_id).map_err(runtime_error)?;
            }
        }
        Ok(())
    }


    pub fn load_resident_inference(
        &mut self,
        node_id: String,
        mut session: crate::inference::ResidentInferenceSession,
        config: InferenceConfig,
    ) -> Result<crate::inference::InferenceSessionInfo, String> {
        crate::inference::prepared::validate_resident_config(&config)?;
        let backend = self.render_device.backend();
        if !Arc::ptr_eq(session.device_owner().device(), &backend.device)
            || !Arc::ptr_eq(session.device_owner().queue(), &backend.queue)
        {
            return Err(
                "Inference session must use the renderer's exact GPU device and queue".to_owned(),
            );
        }
        if !self.session.execution_plan().is_some_and(|plan| {
            plan.nodes
                .iter()
                .any(|node| node.id == node_id && node.node_type == NodeType::Inference)
        }) {
            return Err(format!(
                "Inference node {node_id} does not exist in the current graph"
            ));
        }
        let info =
            crate::inference::prepared::resident_session_info(session.model_plan(), &config)?;
        session.set_frame_budget(backend.frame_budget())?;
        self.cancel_pending();
        self.latest = None;
        self.inference_ready.remove(&node_id);
        let info = self
            .inference
            .load(node_id.clone(), session, config, info)?;
        self.render_device.remove_texture(&node_id);
        self.session.mark_dirty(&node_id).map_err(runtime_error)?;
        Ok(info)
    }

    pub fn inference_detection_buffer(
        &self,
        node_id: &str,
    ) -> Option<(Arc<wgpu::Buffer>, Arc<dyn std::any::Any + Send + Sync>)> {
        self.inference.detection_buffer(node_id)
    }

    pub fn inference_raw_result_buffer(
        &self,
        node_id: &str,
    ) -> Option<(Arc<wgpu::Buffer>, Arc<dyn std::any::Any + Send + Sync>)> {
        self.inference.raw_result_buffer(node_id)
    }

    #[cfg(feature = "inference-diagnostics")]
    pub fn request_inference_snapshot(&mut self, node_id: &str) -> Result<(), String> {
        self.drain_inference_completions()?;
        self.inference.request_snapshot(node_id)?;
        self.session.mark_dirty(node_id).map_err(runtime_error)
    }

    #[cfg(feature = "inference-diagnostics")]
    pub fn take_inference_snapshot(
        &mut self,
        node_id: &str,
    ) -> Option<crate::inference::InferenceDiagnosticSnapshot> {
        self.inference.take_snapshot(node_id)
    }

    pub fn unload_inference(&mut self, node_id: &str) {
        self.cancel_pending();
        self.latest = None;
        self.inference_ready.remove(node_id);
        self.inference.unload(node_id);
        self.render_device.remove_texture(node_id);
    }

    pub(crate) fn drain_inference_completions(&mut self) -> Result<(), String> {
        while let Some(accepted) = self.inference.next_completion(&self.session)? {
            if let Err(error) = self.session.validate_completion(&accepted.envelope) {
                self.inference.record_rejection();
                return Err(runtime_error(error));
            }
            if let Err(error) = self.render_device.register_external_texture(
                &accepted.node_id,
                accepted.image.handle(self.inference_sampler.clone()),
            ) {
                self.inference.record_rejection();
                return Err(error.to_string());
            }
            if let Err(error) = self.session.submit_completion(accepted.envelope.clone()) {
                self.inference.record_rejection();
                return Err(runtime_error(error));
            }
            self.inference_ready.insert(
                accepted.node_id.clone(),
                accepted.envelope.input_stamp.clone(),
            );
            let delivered = accepted
                .subscription_id
                .as_deref()
                .is_some_and(|id| self.session.take_latest_delivery(id));
            self.inference.record_delivery(accepted, delivered);
        }
        for node_id in self.inference.take_ready_nodes() {
            if self.session.node_generation(&node_id).is_ok() {
                self.session.mark_dirty(&node_id).map_err(runtime_error)?;
            }
        }
        Ok(())
    }

    #[cfg(windows)]
    pub fn set_shared_texture_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        self.render_device.set_shared_texture_enabled(enabled)
    }

    #[cfg(windows)]
    pub fn take_shared_texture(&mut self) -> Option<SharedTextureFrame> {
        self.render_device.take_shared_texture()
    }

    #[cfg(windows)]
    pub fn has_shared_texture_pending(&self) -> bool {
        self.render_device.has_shared_texture_pending()
    }

    #[cfg(windows)]
    pub fn release_shared_texture(&mut self, lease_id: u64) -> Result<(), String> {
        self.render_device.release_shared_texture(lease_id)
    }

    pub fn take_inference_events(&mut self) -> Vec<InferenceOutputEvent> {
        self.inference.take_output_events()
    }

    pub fn inference_metrics(&self) -> InferenceMetrics {
        self.inference.metrics()
    }

    pub fn inference_capabilities(&self) -> Result<oq_inference::InferenceCapabilities, String> {
        Ok(oq_inference::InferenceCapabilities {
            cpu: false,
            direct_ml: false,
            core_ml: false,
            shared_wgpu_device: self.inference_available,
        })
    }

    pub fn backend(&self) -> Arc<GpuBackend> {
        self.render_device.backend()
    }

    pub fn has_graph(&self) -> bool {
        self.session.execution_plan().is_some()
    }

    pub fn has_output_texture(&self, node_id: &str) -> bool {
        self.render_device.has_output_texture(node_id)
    }

    #[cfg(feature = "native-video")]
    pub fn video_metrics(&self, node_id: &str) -> Result<NativeVideoMetrics, String> {
        self.videos.metrics(node_id)
    }

    pub fn drain_events(&mut self) -> Vec<crate::evaluation::EngineEvent> {
        self.session.drain_events()
    }

    pub fn drain_events_json(&mut self) -> Result<String, String> {
        serde_json::to_string(&self.drain_events())
            .map_err(|error| format!("Cannot serialize runtime events: {error}"))
    }

    pub fn commit_graph(&mut self, prepared: PreparedNativeGraph) -> Result<u32, String> {
        if !Arc::ptr_eq(&prepared.backend, &self.render_device.backend()) {
            return Err("Native graph was prepared on a different render device".to_owned());
        }
        self.cancel_pending();
        self.latest = None;
        self.inference_ready.clear();
        let graph = &prepared.graph;
        let revision = self.session.set_graph(graph).map_err(runtime_error)?;
        self.render_device
            .commit_plan(prepared.rendering)
            .expect("native graph plan was prepared on this render device");
        for node_id in prepared.dirty_images {
            self.session
                .mark_dirty(&node_id)
                .expect("staged image node belongs to the committed graph");
        }
        self.inference.sync_graph(graph, &mut self.session)?;
        #[cfg(feature = "native-video")]
        {
            let node_ids = graph
                .nodes
                .iter()
                .filter(|node| {
                    node.node_type == NodeType::Input
                        && node.data.input_mode == Some(crate::project::model::InputMode::Video)
                })
                .map(|node| node.id.clone())
                .collect();
            self.videos.retain(&node_ids);
            for node_id in prepared.detached_videos {
                self.videos.detach(&node_id);
            }
            self.videos.pause();
            for (node_id, source) in prepared.videos {
                self.videos.attach_prepared(&node_id, source);
            }
            self.sync_video_nodes()?;
        }
        self.input_versions.clear();
        Ok(revision)
    }

    pub fn revision(&self) -> u32 {
        self.session.revision()
    }
    pub fn has_in_flight(&self) -> bool {
        self.pending.is_some() || self.completion.is_some()
            || !self.window.queued.is_empty() || !self.window.completed.is_empty()
    }
    pub fn frame_state(&self) -> crate::evaluation::EngineState {
        self.session.state()
    }
    pub fn pending_stamp(&self) -> Option<&FrameStamp> {
        self.pending.as_ref().map(|p| &p.demand.input.stamp)
    }
    pub fn subscribe_output(
        &mut self,
        subscription: crate::outputs::OutputSubscription,
    ) -> Result<(), SdkError> {
        self.session.subscribe_output(subscription)
    }
    pub fn drain_deliveries(&mut self) -> crate::outputs::OutputDeliveryBatch {
        self.session.drain_deliveries()
    }
    pub fn set_every_queue_capacity(&mut self, capacity: usize) {
        self.session.set_every_queue_capacity(capacity);
    }

    fn bump_input(&mut self, node: &str) {
        let version = self.input_versions.entry(node.to_owned()).or_default();
        *version = version.checked_add(1).expect("input version exhausted");
    }

    #[cfg(feature = "native-video")]
    pub fn upload_video_frames(&mut self) -> Result<(), String> {
        // Preview requests retain their bounded slot across polls. Graph work must
        // be explicitly stopped before replacing its source-only preview.
        if self.pending.is_some() { self.cancel_pending(); }
        self.upload_current_sources()
    }

    pub fn submit(&mut self, demand: FrameDemand) -> FrameAdmission {
        if self.has_in_flight() {
            return FrameAdmission::WouldBlock(demand);
        }
        if let Err(error) = self.validate_demand(&demand) {
            return FrameAdmission::Rejected { demand, error };
        }
        let targets = match self.prepare_demand_targets(&demand) {
            Ok(Some(targets)) => targets,
            Ok(None) => return FrameAdmission::WouldBlock(demand),
            Err(error) => return FrameAdmission::Rejected { demand, error },
        };
        self.submit_prepared(demand, targets)
    }

    fn submit_prepared(&mut self, demand: FrameDemand, targets: crate::rendering::PreparedFrameTargets) -> FrameAdmission {
        if self.pending.is_some() || self.completion.is_some() {
            return FrameAdmission::WouldBlock(demand);
        }
        if let Err(error) = self.validate_demand(&demand) {
            return FrameAdmission::Rejected { demand, error };
        }
        // Texture registration itself enqueues no GPU mutation. Keep original
        // bindings until graph/output preflight accepts the whole request.
        let mut original = Vec::with_capacity(demand.sources.len());
        for source in &demand.sources {
            original.push((
                source.node_id.clone(),
                self.render_device.output_handle(&source.node_id),
            ));
            if let Err(error) = self
                .render_device
                .register_external_texture(&source.node_id, source.frame.output().clone())
            {
                self.restore_inputs(original);
                return FrameAdmission::Rejected {
                    demand,
                    error: error.to_string(),
                };
            }
        }
        let changed: Vec<_> = demand
            .sources
            .iter()
            .filter(|source| {
                self.input_versions.get(&source.node_id).copied() != Some(source.version)
            })
            .map(|source| source.node_id.as_str())
            .collect();
        if let Err(error) = self.session.evaluate_with_dirty(&demand.input, &changed) {
            self.restore_inputs(original);
            return FrameAdmission::Rejected {
                demand,
                error: error.to_json(),
            };
        }
        self.render_device.commit_frame_targets(targets)
            .expect("frame targets were prepared on this exact device");
        for source in &demand.sources {
            self.input_versions
                .insert(source.node_id.clone(), source.version);
        }
        self.last_key = Some((demand.input.stamp.epoch, demand.request_id));
        self.inference_ready.clear();
        let commands = self.session.drain_commands();
        self.pending = Some(PendingFrame {
            demand,
            commands,
            next: 0,
            started: false,
            waiting_inference: None,
            sources_uploaded: false,
            media_pts_ns: None,
        });
        FrameAdmission::Accepted
    }

    fn restore_inputs(&mut self, originals: Vec<(String, Option<GpuOutputHandle>)>) {
        for (node, original) in originals {
            if let Some(original) = original {
                self.render_device
                    .register_external_texture(&node, original)
                    .expect("previously validated source remains valid");
            } else {
                self.render_device.remove_texture(&node);
            }
        }
    }

    fn validate_demand(&self, demand: &FrameDemand) -> Result<(), String> {
        if self.session.state() == EngineState::Disposed {
            return Err("Frame pipeline has been disposed".into());
        }
        if demand.graph_revision != self.session.revision() {
            return Err("Frame demand graph revision is stale".into());
        }
        let key = (demand.input.stamp.epoch, demand.request_id);
        if demand.request_id == 0 || self.last_key.is_some_and(|previous| key <= previous) {
            return Err("Frame demand epoch/request id is stale or repeated".into());
        }
        let plan = self
            .session
            .execution_plan()
            .ok_or("Frame pipeline has no graph")?;
        if !plan
            .nodes
            .iter()
            .any(|node| node.id == demand.output_node_id)
        {
            return Err("Frame demand output does not exist".into());
        }
        let mut seen = std::collections::HashSet::new();
        for source in &demand.sources {
            if source.version == 0 || !seen.insert(&source.node_id) {
                return Err("External source versions must be nonzero and node ids unique".into());
            }
            if !plan
                .nodes
                .iter()
                .any(|node| node.id == source.node_id && node.node_type == NodeType::Input)
            {
                return Err(format!(
                    "External source {} is not an input node",
                    source.node_id
                ));
            }
            if !source.frame.belongs_to(&self.render_device.backend()) {
                return Err("External source frame belongs to another GPU owner".into());
            }
            if let Some(previous) = self.input_versions.get(&source.node_id) {
                if source.version < *previous {
                    return Err("External source version is stale".into());
                }
                if source.version == *previous
                    && self
                        .render_device
                        .output_texture(&source.node_id)
                        .is_none_or(|output| {
                            output.texture != source.frame.output().texture.as_ref()
                        })
                {
                    return Err(
                        "External source version was reused for a different snapshot".into(),
                    );
                }
            }
        }
        Ok(())
    }

    /// Progress only on the owning worker/executor. It never advances a clock or
    /// blocks for model, media or snapshot capacity; poll_completion just drains.
    pub fn progress(&mut self) {
        if let Err(error) = self.render_device.device().poll(wgpu::PollType::Poll) {
            if let Some(pending) = self.pending.take() {
                self.finish(pending.demand, FrameOutcome::Failed(error.to_string()));
            }
        }
        self.progress_window();
        let Some(mut pending) = self.pending.take() else {
            return;
        };
        let result = self.progress_frame(&mut pending);
        match result {
            Ok(Some(frame)) => {
                self.record_stage(&pending.demand, "GpuSubmitted");
                if !self.window.enabled {
                    self.latest = Some(frame.clone());
                }
                self.finish(pending.demand, FrameOutcome::Submitted(frame));
            }
            Ok(None) => {
                self.pending = Some(pending);
            }
            Err(error) => {
                self.inference.cancel_pending();
                self.inference_ready.clear();
                for command in &pending.commands {
                    let _ = self.session.mark_dirty(&command.node_id);
                }
                self.finish(pending.demand, FrameOutcome::Failed(error));
            }
        }
        self.collect_window_completion();
    }

    fn source_work_pending(&self) -> bool {
        let pending = self.inference.has_pending();
        #[cfg(feature = "native-video")]
        let pending = pending || self.videos.is_active();
        pending
    }

    fn progress_frame(
        &mut self,
        pending: &mut PendingFrame,
    ) -> Result<Option<StableFrame>, String> {
        self.render_device
            .device()
            .poll(wgpu::PollType::Poll)
            .map_err(|e| e.to_string())?;
        if pending.demand.graph_revision != self.session.revision() {
            return Err("Admitted frame graph was replaced".into());
        }
        if !pending.sources_uploaded {
            #[cfg(feature = "native-video")]
            {
                let excluded = pending.demand.sources.iter()
                    .map(|source| source.node_id.clone()).collect::<Vec<_>>();
                if !self.videos.prefetch_at(pending.demand.request_id,
                    pending.demand.input.stamp.timeline_ns,
                    pending.demand.input.stamp.epoch, &excluded)? {
                    return Ok(None);
                }
                let render_device = &mut self.render_device;
                let Some(selected_pts) = self.videos.poll_upload_at(pending.demand.request_id,
                    |node_id, frame, _, _| match frame {
                        NativeVideoFrame::Rgba(_) => Err("Hardware-resident video required; CPU upload forbidden".into()),
                        #[cfg(target_os = "macos")]
                        NativeVideoFrame::VideoToolbox(frame) => render_device.try_upload_videotoolbox(node_id, frame).map_err(|e| e.to_string()),
                        #[cfg(all(windows, feature = "native-video-d3d12"))]
                        NativeVideoFrame::D3d12(frame) => render_device.upload_d3d12_yuv(node_id, frame).map(|_| true).map_err(|e| e.to_string()),
                    })? else { return Ok(None); };
                pending.media_pts_ns = if excluded.is_empty() { selected_pts } else { None };
            }
            pending.sources_uploaded = true;
            self.record_stage(&pending.demand, "SourceReady");
        }
        self.drain_inference_completions()?;
        if let Some(node) = pending.waiting_inference.as_ref() {
            if self.inference_ready.get(node) != Some(&pending.demand.input.stamp) {
                return Ok(None);
            }
            pending.waiting_inference = None;
            pending.next += 1;
        }
        while pending.next < pending.commands.len() {
            let command = &pending.commands[pending.next];
            if command.kind == "inference" {
                if self.inference_ready.get(&command.node_id) == Some(&pending.demand.input.stamp) {
                    pending.next += 1;
                    continue;
                }
                if !self.inference.is_loaded(&command.node_id) {
                    return Ok(None);
                }
                if command
                    .texture_inputs
                    .values()
                    .any(|node| !self.render_device.has_output_texture(node))
                {
                    if self.source_work_pending() {
                        return Ok(None);
                    }
                    return Err(format!(
                        "Inference dependency for {} has no GPU texture",
                        command.node_id
                    ));
                }
                self.inference.schedule(
                    command,
                    &pending.demand.input.stamp,
                    &self.session,
                    &self.render_device,
                )?;
                if self.inference.is_active(&command.node_id) {
                    pending.started = true;
                    pending.waiting_inference = Some(command.node_id.clone());
                }
                return Ok(None);
            }
            let start = pending.next;
            while pending.next < pending.commands.len() {
                let command = &pending.commands[pending.next];
                if command.kind == "inference" {
                    break;
                }
                if command
                    .texture_inputs
                    .values()
                    .any(|node| !self.render_device.has_output_texture(node))
                {
                    break;
                }
                pending.next += 1;
            }
            if start == pending.next {
                if self.source_work_pending() {
                    return Ok(None);
                }
                return Err(format!(
                    "Frame dependency for {} has no GPU texture",
                    pending.commands[start].node_id
                ));
            }
            self.session
                .execute_gpu(
                    &mut self.render_device,
                    &pending.commands[start..pending.next],
                )
                .map_err(runtime_error)?;
            pending.started = true;
        }
        if !self
            .render_device
            .has_output_texture(&pending.demand.output_node_id)
        {
            if self.source_work_pending() {
                return Ok(None);
            }
            return Err("Requested output has no GPU texture".into());
        }
        let stamp = pending.demand.input.stamp.clone();
        let identity = FrameSnapshotIdentity {
            output_node_id: pending.demand.output_node_id.clone(),
            graph_revision: pending.demand.graph_revision,
            evaluation_stamp: Some(stamp.clone()),
            content_stamp: Some(ContentStamp {
                epoch: stamp.epoch,
                timeline_ns: stamp.timeline_ns,
                media_pts_ns: pending.media_pts_ns,
            }),
            color_space: FrameColorSpace::Unknown,
            alpha_mode: FrameAlphaMode::Unknown,
        };
        match self.stable_previews.try_capture(
            &self.render_device,
            &pending.demand.output_node_id,
            identity,
        ) {
            Ok(frame) => Ok(Some(frame)),
            Err(FrameCaptureError::WouldBlock) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn finish(&mut self, demand: FrameDemand, outcome: FrameOutcome) {
        debug_assert!(self.completion.is_none());
        if self.window.enabled && matches!(outcome, FrameOutcome::Failed(_)) {
            self.window.failed = true;
            #[cfg(feature = "native-video")]
            self.videos.cancel_prefetch();
        }
        self.completion = Some(FrameCompletion {
            request_id: demand.request_id,
            stamp: demand.input.stamp,
            graph_revision: demand.graph_revision,
            outcome,
        });
    }

    pub fn poll_completion(&mut self) -> Option<FrameCompletion> {
        if self.window.enabled {
            self.collect_window_completion();
            let completion = self.window.completed.pop_front();
            if let Some(FrameCompletion { outcome: FrameOutcome::Submitted(frame), .. }) = &completion {
                self.latest = Some(frame.clone());
            }
            return completion;
        }
        self.completion.take()
    }

    pub fn cancel_pending(&mut self) -> bool {
        self.window.failed = false;
        if self.window.enabled {
            for completion in &mut self.window.completed {
                completion.outcome = FrameOutcome::Cancelled;
            }
            if let Some(completion) = &mut self.completion {
                completion.outcome = FrameOutcome::Cancelled;
            }
        }
        let mut cancelled = false;
        #[cfg(feature = "native-video")]
        self.videos.cancel_prefetch();
        let Some(pending) = self.pending.take() else {
            while let Some(queued) = self.window.queued.pop_front() {
                self.window.completed.push_back(FrameCompletion { request_id: queued.demand.request_id,
                    stamp: queued.demand.input.stamp, graph_revision: queued.demand.graph_revision,
                    outcome: FrameOutcome::Cancelled });
                cancelled = true;
            }
            return cancelled;
        };
        self.inference.cancel_pending();
        self.inference_ready.clear();
        if !pending.started {
            self.session.rollback_feedback(&pending.commands);
        }
        // Cancelled CPU work is retryable; submitted GPU work is never rolled back.
        for command in &pending.commands {
            let _ = self.session.mark_dirty(&command.node_id);
        }
        self.finish(pending.demand, FrameOutcome::Cancelled);
        self.collect_window_completion();
        while let Some(queued) = self.window.queued.pop_front() {
            self.window.completed.push_back(FrameCompletion {
                request_id: queued.demand.request_id,
                stamp: queued.demand.input.stamp,
                graph_revision: queued.demand.graph_revision,
                outcome: FrameOutcome::Cancelled,
            });
        }
        true
    }

    pub fn pause_execution(&mut self) -> Result<(), String> {
        self.cancel_pending();
        self.session.pause().map_err(runtime_error)
    }
    pub fn resume_execution(&mut self) -> Result<(), String> {
        self.session.resume().map_err(runtime_error)
    }
    pub fn stop_execution(&mut self) -> Result<(), String> {
        self.cancel_pending();
        self.inference.cancel_pending();
        self.inference_ready.clear();
        self.session.stop().map_err(runtime_error)
    }
    pub fn dispose(&mut self) -> Result<(), String> {
        self.cancel_pending();
        self.session.dispose().map_err(runtime_error)?;
        #[cfg(feature = "native-video")]
        self.videos.clear();
        self.inference.clear();
        self.inference_ready.clear();
        self.latest = None;
        self.window.queued.clear();
        self.stable_previews.clear_allocations();
        self.budget_wake = None;
        self.render_device.reset()
    }
}

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;
