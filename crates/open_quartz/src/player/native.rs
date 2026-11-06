#[cfg(all(test, not(feature = "native-onnx"), not(feature = "native-video")))]
#[path = "native_feature_coverage_tests.rs"]
mod feature_tests;
#[cfg(all(test, feature = "native-onnx"))]
#[path = "inference_lifecycle_tests.rs"]
mod inference_lifecycle_tests;
#[cfg(all(test, feature = "native-onnx"))]
#[path = "inference_output_validation_tests.rs"]
mod inference_output_validation_tests;
#[cfg(all(test, feature = "native-video"))]
#[path = "video_lifecycle_tests.rs"]
mod video_lifecycle_tests;
#[path = "native_support.rs"]
mod support;

use std::sync::Arc;
use std::time::{Instant, SystemTime};

use crate::evaluation::ExecutionCommand;
use crate::rendering::{GpuBackend, GpuOutputHandle, RenderDevice, RenderDeviceInfo};
#[cfg(windows)]
use crate::rendering::SharedTextureFrame;
#[cfg(feature = "native-video")]
use crate::media::native::{NativeVideoConfig, NativeVideoFrame, NativeVideoInfo, NativeVideoMetrics};
#[cfg(feature = "native-video")]
use crate::media::sources::VideoSources;
#[cfg(feature = "native-onnx")]
use crate::inference::{tasks::InferenceTasks, OnnxSession};
use crate::outputs::{DataPathMode, FrameStamp};
use crate::player::{PlaybackCapabilities, PlaybackSession, SessionFrameInput};
use crate::project::model::{Graph, NodeType};
use crate::SdkError;
use serde::Serialize;
use support::{utc_date_uniform, NativeRenderPerf};

pub use crate::inference::{tasks::NativeOutputEvent, NativeOnnxConfig};


#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativePlayerInfo {
    pub adapter_name: String,
    pub backend: String,
    pub device_type: String,
    pub output_mode: String,
    pub native_onnx_cpu: bool,
    pub native_onnx_direct_ml: bool,
    pub shared_onnx_wgpu_device: bool,
    pub native_video: bool,
    pub video_data_path: String,
    pub tensor_data_path: String,
    pub shared_texture: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeFrameRendered {
    pub frame: u64,
    /// Composition playback time; not source-video PTS or wall time.
    pub timeline_ns: u64,
    pub revision: u32,
    pub output_node_id: String,
    pub width: u32,
    pub height: u32,
}
/// Coordinates playback with the rendering, video, and inference owners.
pub struct NativePlayer {
    render_device: RenderDevice,
    session: PlaybackSession,
    clock_origin: Instant,
    mouse: [f32; 4],
    #[cfg(feature = "native-video")]
    videos: VideoSources,
    #[cfg(feature = "native-onnx")]
    inference: InferenceTasks,
    perf: NativeRenderPerf,
}

/// A validated graph and its GPU resources, ready to commit on the preparing device.
pub struct PreparedNativeGraph {
    graph: Graph,
    plan: crate::compiler::ExecutionPlan,
    rendering: crate::rendering::PreparedRenderPlan,
    backend: Arc<GpuBackend>,
    dirty_images: Vec<String>,
    #[cfg(feature = "native-video")]
    videos: Vec<(String, crate::media::sources::PreparedVideoSource)>,
    #[cfg(feature = "native-video")]
    detached_videos: Vec<String>,
}

impl PreparedNativeGraph {
    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    pub fn execution_plan(&self) -> &crate::compiler::ExecutionPlan {
        &self.plan
    }

    pub fn stage_image(&mut self, node_id: &str, rgba: &[u8], width: u32, height: u32) -> Result<(), String> {
        if !self.graph.nodes.iter().any(|node| node.id == node_id) {
            return Err(format!("Image node {node_id} does not exist in the prepared graph"));
        }
        self.rendering.stage_image(node_id, rgba, width, height).map_err(|error| error.to_string())?;
        self.dirty_images.push(node_id.to_owned());
        Ok(())
    }

    pub fn stage_remove_texture(&mut self, node_id: &str) {
        self.rendering.stage_remove_texture(node_id);
    }

    #[cfg(feature = "native-video")]
    pub fn stage_video(&mut self, node_id: &str, config: NativeVideoConfig, preview_only: bool) -> Result<NativeVideoInfo, String> {
        if !self.graph.nodes.iter().any(|node| node.id == node_id && node.node_type == NodeType::Input
            && node.data.input_mode == Some(crate::project::model::InputMode::Video)) {
            return Err(format!("Video node {node_id} does not exist in the prepared graph"));
        }
        let source = crate::media::sources::PreparedVideoSource::open(config, preview_only)?;
        let info = source.info().clone();
        self.rendering.stage_remove_texture(node_id);
        self.videos.retain(|(id, _)| id != node_id);
        self.videos.push((node_id.to_owned(), source));
        Ok(info)
    }

    #[cfg(feature = "native-video")]
    pub fn stage_detach_video(&mut self, node_id: &str) {
        self.rendering.stage_remove_texture(node_id);
        self.videos.retain(|(id, _)| id != node_id);
        self.detached_videos.push(node_id.to_owned());
    }
}

fn runtime_error(error: SdkError) -> String {
    error.to_json()
}

impl NativePlayer {
    pub async fn new() -> Result<(Self, NativePlayerInfo), String> {
        let (device, info) = RenderDevice::new().await?;
        Ok(Self::from_render_device(device, info))
    }

    pub fn from_shared_wgpu(
        device: wgpu::Device,
        queue: wgpu::Queue,
        adapter: &wgpu::Adapter,
    ) -> Result<(Self, NativePlayerInfo), String> {
        let (device, info) = RenderDevice::from_shared_wgpu(device, queue, adapter)?;
        Ok(Self::from_render_device(device, info))
    }

    fn from_render_device(render_device: RenderDevice, device_info: RenderDeviceInfo) -> (Self, NativePlayerInfo) {
        let required_features = render_device.device().features();
        let shared_texture = device_info.shared_texture;
        let info = NativePlayerInfo {
            adapter_name: device_info.adapter_name,
            backend: device_info.backend,
            device_type: device_info.device_type,
            output_mode: device_info.output_mode,
            native_onnx_cpu: cfg!(feature = "native-onnx"),
            native_video: {
                #[cfg(feature = "native-video")]
                { crate::media::native::find_ffmpeg().is_ok() }
                #[cfg(not(feature = "native-video"))]
                { false }
            },
            native_onnx_direct_ml: cfg!(all(feature = "native-onnx", target_os = "windows")),
            shared_onnx_wgpu_device: false,
            video_data_path: if !cfg!(feature = "native-video") {
                "disabled".to_owned()
            } else if cfg!(all(windows, feature = "native-video-d3d12"))
                && required_features.intersects(wgpu::Features::TEXTURE_FORMAT_NV12 | wgpu::Features::TEXTURE_FORMAT_P010) {
                "d3d12va-yuv-detached-gpu-copy".to_owned()
            } else if cfg!(target_os = "macos") {
                "negotiated-per-source-videotoolbox-metal-or-cpu".to_owned()
            } else {
                "cpu-copy".to_owned()
            },
            tensor_data_path: if cfg!(feature = "native-onnx") { "cpu-copy" } else { "disabled" }.to_owned(),
            shared_texture,
        };
        (Self {
            render_device,
            session: PlaybackSession::new_native(PlaybackCapabilities {
                data_paths: if shared_texture { vec![DataPathMode::NativePresent] } else { vec![DataPathMode::CpuCopy] },
            }),
            clock_origin: Instant::now(),
            mouse: [0.0; 4],
            #[cfg(feature = "native-video")]
            videos: VideoSources::new(),
            #[cfg(feature = "native-onnx")]
            inference: InferenceTasks::new(),
            perf: NativeRenderPerf::new(),
        }, info)
    }

    pub fn set_graph(&mut self, graph_json: &str) -> Result<u32, String> {
        let graph: Graph = serde_json::from_str(graph_json)
            .map_err(|error| format!("Invalid native graph JSON: {error}"))?;
        self.set_graph_snapshot(&graph)
    }

    /// In-process callers pass the typed graph; serialization stays at host boundaries.
    pub fn set_graph_snapshot(&mut self, graph: &Graph) -> Result<u32, String> {
        let prepared = self.prepare_graph(graph)?;
        self.commit_graph(prepared)
    }

    /// Validate and allocate a replacement without changing live playback or resource owners.
    pub fn prepare_graph(&self, graph: &Graph) -> Result<PreparedNativeGraph, String> {
        #[cfg(not(feature = "native-onnx"))]
        if graph.nodes.iter().any(|node| node.node_type == NodeType::Onnx) {
            return Err("Native ONNX graph execution requires the native-onnx feature".to_owned());
        }
        #[cfg(not(feature = "native-video"))]
        if graph.nodes.iter().any(|node| node.node_type == NodeType::Input && node.data.input_mode == Some(crate::project::model::InputMode::Video)) {
            return Err("Native video graph execution requires the native-video feature".to_owned());
        }
        graph.validate().map_err(runtime_error)?;
        let plan = crate::compiler::build_execution_plan_with_options(&graph.nodes, &graph.edges, false);
        if let Some((node, error)) = plan.nodes.iter().find_map(|node| {
            node.validation_errors.first().map(|error| (node, error))
        }) {
            return Err(runtime_error(SdkError::new(
                crate::project::model::SdkErrorCode::InvalidResource,
                error.message.clone(),
            ).for_node(&node.id)));
        }
        let prepared = self.render_device.prepare_plan(&plan, graph).map_err(|error| {
            let mut sdk_error = SdkError::new(crate::project::model::SdkErrorCode::InvalidResource, error.message);
            sdk_error.node_id = error.node_id;
            runtime_error(sdk_error)
        })?;
        #[cfg(feature = "native-video")]
        let prepared = {
            let mut prepared = prepared;
            for node_id in self.videos.node_ids() {
                if !graph.nodes.iter().any(|node| node.id == node_id
                    && node.node_type == NodeType::Input
                    && node.data.input_mode == Some(crate::project::model::InputMode::Video)) {
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

    pub fn commit_graph(&mut self, prepared: PreparedNativeGraph) -> Result<u32, String> {
        if !Arc::ptr_eq(&prepared.backend, &self.render_device.backend()) {
            return Err("Native graph was prepared on a different render device".to_owned());
        }
        let graph = &prepared.graph;
        let revision = self.session.set_graph(graph).map_err(runtime_error)?;
        self.render_device.commit_plan(prepared.rendering)
            .expect("native graph plan was prepared on this render device");
        for node_id in prepared.dirty_images {
            self.session.mark_dirty(&node_id)
                .expect("staged image node belongs to the committed graph");
        }
        #[cfg(feature = "native-onnx")]
        self.inference.sync_graph(graph, &mut self.session)?;
        #[cfg(feature = "native-video")]
        {
            let node_ids = graph.nodes.iter()
                .filter(|node| node.node_type == NodeType::Input
                    && node.data.input_mode == Some(crate::project::model::InputMode::Video))
                .map(|node| node.id.clone()).collect();
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
        self.clock_origin = Instant::now();
        Ok(revision)
    }

    pub fn upload_image(&mut self, node_id: &str, rgba: &[u8], width: u32, height: u32) -> Result<(), String> {
        self.session.node_generation(node_id).map_err(runtime_error)?;
        self.render_device.upload_rgba(node_id, rgba, width, height).map_err(|error| error.to_string())?;
        self.session.mark_dirty(node_id).map_err(runtime_error)
    }

    pub fn remove_texture(&mut self, node_id: &str) {
        self.render_device.remove_texture(node_id);
    }

    pub fn read_output(&self, node_id: &str) -> Result<Vec<u8>, String> {
        self.render_device.read_output(node_id)
    }

    pub fn preview_source(&self, node_id: &str) -> Result<GpuOutputHandle, String> {
        self.render_device.preview_source(node_id)
    }


    pub fn output_payload(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
        RenderDevice::output_payload(rgba, width, height)
    }

    #[cfg(feature = "native-video")]
    pub fn attach_video(&mut self, node_id: &str, config: NativeVideoConfig) -> Result<NativeVideoInfo, String> {
        self.attach_video_source(node_id, config, false)
    }

    /// Prepare a single decoded source frame without starting engine playback.
    #[cfg(feature = "native-video")]
    pub fn attach_video_preview(&mut self, node_id: &str, config: NativeVideoConfig) -> Result<NativeVideoInfo, String> {
        self.attach_video_source(node_id, config, true)
    }

    #[cfg(feature = "native-video")]
    fn attach_video_source(&mut self, node_id: &str, config: NativeVideoConfig, preview_only: bool) -> Result<NativeVideoInfo, String> {
        self.session.node_generation(node_id).map_err(runtime_error)?;
        let info = self.videos.attach(node_id, config, preview_only)?;
        self.sync_video_nodes()?;
        self.session.mark_dirty(node_id).map_err(runtime_error)?;
        Ok(info)
    }

    #[cfg(feature = "native-video")]
    pub fn detach_video(&mut self, node_id: &str) -> Result<(), String> {
        self.videos.detach(node_id);
        self.render_device.remove_texture(node_id);
        self.sync_video_nodes()
    }

    #[cfg(feature = "native-video")]
    fn sync_video_nodes(&mut self) -> Result<(), String> {
        self.session.set_video_nodes(&self.videos.node_ids()).map_err(runtime_error)
    }

    #[cfg(feature = "native-video")]
    pub fn upload_video_frames(&mut self) -> Result<(), String> {
        let render_device = &mut self.render_device;
        let dirty = self.videos.upload_frames(|node_id, frame, width, height| match frame {
            NativeVideoFrame::Rgba(rgba) => render_device.upload_rgba(node_id, rgba, width, height).map_err(|error| error.to_string()),
            #[cfg(target_os = "macos")]
            NativeVideoFrame::VideoToolbox(frame) => render_device.upload_videotoolbox(node_id, frame).map_err(|error| error.to_string()),
            #[cfg(all(windows, feature = "native-video-d3d12"))]
            NativeVideoFrame::D3d12(frame) => render_device.upload_d3d12_yuv(node_id, frame).map_err(|error| error.to_string()),
        })?;
        for node_id in dirty {
            self.session.mark_dirty(&node_id).map_err(runtime_error)?;
        }
        Ok(())
    }

    pub fn start_playback(&mut self) -> Result<(), String> {
        #[cfg(feature = "native-video")]
        self.videos.resume()?;
        let now_ns = self.clock_now_ns();
        self.session.play(now_ns).map_err(runtime_error)
    }

    fn clock_now_ns(&self) -> u64 {
        self.clock_origin.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
    }

    #[cfg(feature = "native-onnx")]
    pub fn load_onnx(&mut self, node_id: String, session: OnnxSession, config: NativeOnnxConfig) -> Result<crate::inference::OnnxSessionInfo, String> {
        self.session.node_generation(&node_id).map_err(runtime_error)?;
        let info = self.inference.load(node_id.clone(), session, config);
        self.render_device.remove_texture(&node_id);
        self.session.mark_dirty(&node_id).map_err(runtime_error)?;
        Ok(info)
    }

    #[cfg(feature = "native-onnx")]
    pub fn unload_onnx(&mut self, node_id: &str) {
        self.inference.unload(node_id);
        self.render_device.remove_texture(node_id);
    }

    #[cfg(feature = "native-onnx")]
    fn drain_inference_completions(&mut self) -> Result<(), String> {
        while let Some(accepted) = self.inference.next_completion(&self.session)? {
            self.session.validate_completion(&accepted.envelope).map_err(runtime_error)?;
            self.render_device.upload_rgba(
                &accepted.node_id,
                &accepted.image.rgba,
                accepted.image.width,
                accepted.image.height,
            ).map_err(|error| error.to_string())?;
            self.session.submit_completion(accepted.envelope.clone()).map_err(runtime_error)?;
            let deliveries = self.session.drain_deliveries();
            self.inference.record_delivery(
                accepted,
                deliveries.deliveries.into_iter().map(|delivery| delivery.subscription_id),
            );
        }
        Ok(())
    }

    fn execute_commands(&mut self, commands: &[ExecutionCommand], input_stamp: &FrameStamp) -> Result<(), String> {
        let mut batch = Vec::new();
        for command in commands {
            if command.kind == "onnx" {
                if !batch.is_empty() {
                    self.session.execute_gpu(&mut self.render_device, &batch).map_err(runtime_error)?;
                    batch.clear();
                }
                #[cfg(feature = "native-onnx")]
                {
                    let render_device = &self.render_device;
                    self.inference.schedule(command, input_stamp, &self.session, |source_id| {
                        if !render_device.has_output_texture(source_id) { return Ok(None); }
                        render_device.read_output_rgba(source_id).map(Some)
                    })?;
                }
                #[cfg(not(feature = "native-onnx"))]
                return Err("Native ONNX execution requires the native-onnx feature".to_owned());
                continue;
            }
            if command.texture_inputs.values().all(|source_id| self.render_device.has_output_texture(source_id)) {
                batch.push(command.clone());
            }
        }
        if !batch.is_empty() {
            self.session.execute_gpu(&mut self.render_device, &batch).map_err(runtime_error)?;
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

    pub fn take_output_events(&mut self) -> Vec<NativeOutputEvent> {
        #[cfg(feature = "native-onnx")]
        { self.inference.take_output_events() }
        #[cfg(not(feature = "native-onnx"))]
        { Vec::new() }
    }

    pub fn render_next(&mut self) -> Result<NativeFrameRendered, String> {
        self.render(self.clock_now_ns())
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

    pub fn pause(&mut self) -> Result<(), String> {
        #[cfg(feature = "native-video")]
        self.videos.pause();
        let now_ns = self.clock_now_ns();
        self.session.pause(now_ns).map_err(runtime_error)
    }

    pub fn resume(&mut self) -> Result<(), String> {
        #[cfg(feature = "native-video")]
        self.videos.resume()?;
        let now_ns = self.clock_now_ns();
        self.session.resume(now_ns).map_err(runtime_error)
    }

    pub fn stop(&mut self) -> Result<(), String> {
        #[cfg(feature = "native-video")]
        self.videos.pause();
        self.session.stop().map_err(runtime_error)?;
        #[cfg(feature = "native-onnx")]
        self.inference.cancel_pending();
        Ok(())
    }

    pub fn dispose(&mut self) -> Result<(), String> {
        self.session.dispose().map_err(runtime_error)?;
        #[cfg(feature = "native-video")]
        self.videos.clear();
        #[cfg(feature = "native-onnx")]
        self.inference.clear();
        self.render_device.reset()
    }

    pub fn close(&mut self) -> Result<(), String> {
        self.dispose()
    }

    pub fn set_mouse(&mut self, mouse: [f32; 4]) {
        self.mouse = mouse;
    }

    pub fn drain_events(&mut self) -> Vec<crate::evaluation::EngineEvent> {
        self.session.drain_events()
    }

    pub fn drain_events_json(&mut self) -> Result<String, String> {
        serde_json::to_string(&self.drain_events()).map_err(|error| format!("Cannot serialize runtime events: {error}"))
    }

    fn render(&mut self, now_ns: u64) -> Result<NativeFrameRendered, String> {
        let total_started = Instant::now();
        let (width, height) = self.session.execution_plan()
            .map(|plan| (plan.default_width.max(1), plan.default_height.max(1)))
            .unwrap_or((960, 540));
        let upload_started = Instant::now();
        #[cfg(feature = "native-video")]
        self.upload_video_frames()?;
        let upload_video = upload_started.elapsed();
        let engine_started = Instant::now();
        #[cfg(feature = "native-onnx")]
        self.drain_inference_completions()?;
        let clock = self.session.advance(&SessionFrameInput {
            now_ns,
            date: utc_date_uniform(SystemTime::now()),
            mouse: self.mouse,
            resolution: [width as f32, height as f32, 1.0],
        }).map_err(runtime_error)?;
        let frame_stamp = FrameStamp {
            epoch: clock.epoch,
            frame: clock.frame,
            timeline_ns: clock.timeline_ns,
            deadline_ns: clock.next_deadline_ns,
        };
        let output_node_id = self.session.execution_plan()
            .ok_or_else(|| "Native engine has no execution plan".to_owned())?
            .output_nodes.first().cloned()
            .ok_or_else(|| "Graph has no renderer or terminal texture output".to_owned())?;
        let commands = self.session.drain_commands();
        let engine_plan = engine_started.elapsed();
        let execute_started = Instant::now();
        self.execute_commands(&commands, &frame_stamp)?;
        let execute_gpu = execute_started.elapsed();
        let dimensions = self.render_device.output_texture(&output_node_id).map(|output| (output.width, output.height));
        let (width, height) = match dimensions {
            Some(dimensions) => dimensions,
            None => {
                let pending = false;
                #[cfg(feature = "native-onnx")]
                let pending = pending || self.inference.has_pending();
                #[cfg(feature = "native-video")]
                let pending = pending || self.videos.is_active();
                if !pending {
                    return Err(format!("Output node {output_node_id} has no native GPU texture"));
                }
                return Ok(NativeFrameRendered {
                    frame: clock.frame,
                    timeline_ns: clock.timeline_ns,
                    revision: self.session.revision(),
                    output_node_id,
                    width: 0,
                    height: 0,
                });
            }
        };
        self.render_device.present(&output_node_id, clock.frame, clock.timeline_ns)?;
        #[cfg(feature = "native-video")]
        let (decoded_frames, uploaded_frames) = self.videos.frame_counts();
        #[cfg(not(feature = "native-video"))]
        let (decoded_frames, uploaded_frames) = (0, 0);
        self.perf.record(upload_video, engine_plan, execute_gpu, total_started.elapsed(), decoded_frames, uploaded_frames);
        Ok(NativeFrameRendered {
            frame: clock.frame,
            timeline_ns: clock.timeline_ns,
            revision: self.session.revision(),
            output_node_id,
            width,
            height,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::NativePlayer;

    #[test]
    fn output_payload_prefixes_little_endian_dimensions() {
        let payload = NativePlayer::output_payload(&[1, 2, 3, 4], 640, 360);
        assert_eq!(&payload[..4], &640_u32.to_le_bytes());
        assert_eq!(&payload[4..8], &360_u32.to_le_bytes());
        assert_eq!(&payload[8..], &[1, 2, 3, 4]);
    }

    #[test]
    fn first_play_after_stopped_shader_edit_and_connection_renders_blue() {
        let (mut host, _) = pollster::block_on(NativePlayer::new()).unwrap();
        let mut graph = serde_json::json!({
            "nodes": [
                {
                    "id": "shader", "type": "shader", "position": {"x": 0, "y": 0},
                    "data": {
                        "type": "shader", "label": "Custom shader",
                        "shaderCode": "@fragment fn main() -> @location(0) vec4f { return vec4f(1.0, 0.0, 0.0, 1.0); }",
                        "inputs": [],
                        "outputs": [{"id": "out", "label": "output", "dataType": "sampler2D", "direction": "output"}],
                        "uniforms": {}, "autoSize": false, "width": 4, "height": 4
                    }
                },
                {
                    "id": "renderer", "type": "renderer", "position": {"x": 1, "y": 0},
                    "data": {
                        "type": "renderer", "label": "Renderer", "shaderCode": "",
                        "inputs": [{"id": "in", "label": "image", "dataType": "sampler2D", "direction": "input"}],
                        "outputs": [], "uniforms": {}
                    }
                }
            ],
            "edges": []
        });
        host.set_graph(&graph.to_string()).unwrap();
        host.stop().unwrap();
        graph["nodes"][0]["data"]["shaderCode"] = serde_json::json!(
            "@fragment fn main() -> @location(0) vec4f { return vec4f(0.0, 0.0, 1.0, 1.0); }"
        );
        host.set_graph(&graph.to_string()).unwrap();
        host.stop().unwrap();
        graph["edges"] = serde_json::json!([
            {"id": "edge", "source": "shader", "sourceHandle": "out", "target": "renderer", "targetHandle": "in"}
        ]);
        host.set_graph(&graph.to_string()).unwrap();

        host.start_playback().unwrap();
        let frame = host.render_next().unwrap();
        assert_eq!(frame.frame, 1);
        assert_eq!(frame.output_node_id, "renderer");
        let payload = host.read_output("renderer").unwrap();
        assert_eq!(&payload[..4], &4_u32.to_le_bytes());
        assert_eq!(&payload[4..8], &4_u32.to_le_bytes());
        assert_eq!(payload.len(), 8 + 4 * 4 * 4);
        assert!(payload[8..].chunks_exact(4).all(|pixel| pixel == [0, 0, 255, 255]));
        assert_eq!(host.read_output("shader").unwrap(), payload);

        // Static frames may emit no more draws but must retain the rendered pixels.
        host.render_next().unwrap();
        assert_eq!(host.read_output("renderer").unwrap(), payload);
        host.stop().unwrap();
    }


}
