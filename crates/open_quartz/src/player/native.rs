#[cfg(all(test, not(feature = "native-video")))]
#[path = "native_feature_coverage_tests.rs"]
mod feature_tests;
#[cfg(all(test, target_os = "macos"))]
#[path = "inference_lifecycle_tests.rs"]
mod inference_lifecycle_tests;
#[cfg(all(test, target_os = "macos"))]
#[path = "inference_output_validation_tests.rs"]
mod inference_output_validation_tests;
#[cfg(all(test, feature = "native-video"))]
#[path = "video_lifecycle_tests.rs"]
mod video_lifecycle_tests;
#[path = "native_support.rs"]
mod support;

#[cfg(test)]
use std::sync::Arc;
use std::time::{Instant, SystemTime};
use crate::execution::{FrameAdmission, FrameDemand, FrameEvaluationInput, FrameOutcome, FramePipeline};
use crate::player::CompositionClock;
pub use crate::execution::PreparedNativeGraph;

#[cfg(test)]
use crate::evaluation::ExecutionCommand;
use crate::rendering::{RenderDevice, RenderDeviceInfo};
#[cfg(all(test, feature = "native-video"))]
use crate::player::SessionFrameInput;

use crate::outputs::{DataPathMode, FrameStamp};
use crate::player::PlaybackCapabilities;
use crate::project::model::Graph;
use crate::SdkError;
use serde::Serialize;
use support::{utc_date_uniform, NativeRenderPerf};

pub use crate::inference::{tasks::{InferenceMetrics, InferenceOutputEvent}, InferenceConfig};


#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativePlayerInfo {
    pub adapter_name: String,
    pub backend: String,
    pub device_type: String,
    pub output_mode: String,
    pub native_inference_cpu: bool,
    pub native_inference_direct_ml: bool,
    pub shared_inference_wgpu_device: bool,
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
    pipeline: FramePipeline,
    clock: CompositionClock,
    clock_origin: Instant,
    mouse: [f32; 4],
    perf: NativeRenderPerf,
    last_stamp: Option<FrameStamp>,
}

impl std::ops::Deref for NativePlayer {
    type Target = FramePipeline;
    fn deref(&self) -> &Self::Target { &self.pipeline }
}
impl std::ops::DerefMut for NativePlayer {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.pipeline }
}

fn runtime_error(error: SdkError) -> String { error.to_json() }

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
        let metal_inference = cfg!(target_os = "macos") && device_info.backend == "Metal";
        let info = NativePlayerInfo {
            adapter_name: device_info.adapter_name,
            backend: device_info.backend,
            device_type: device_info.device_type,
            output_mode: device_info.output_mode,
            native_inference_cpu: false,
            native_video: {
                #[cfg(feature = "native-video")]
                { crate::media::native::find_ffmpeg().is_ok() }
                #[cfg(not(feature = "native-video"))]
                { false }
            },
            native_inference_direct_ml: false,
            shared_inference_wgpu_device: metal_inference,
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
            tensor_data_path: if metal_inference { "device" } else { "unsupported" }.to_owned(),
            shared_texture,
        };
        let pipeline = FramePipeline::from_render_device(render_device, PlaybackCapabilities {
            data_paths: if shared_texture { vec![DataPathMode::NativePresent] } else { vec![DataPathMode::CpuCopy] },
        });
        (Self { pipeline, clock: CompositionClock::new(16_666_667), clock_origin: Instant::now(),
            mouse: [0.0; 4], perf: NativeRenderPerf::new(), last_stamp: None }, info)
    }

    pub fn set_graph(&mut self, graph_json: &str) -> Result<u32, String> {
        let graph: Graph = serde_json::from_str(graph_json).map_err(|error| format!("Invalid native graph JSON: {error}"))?;
        self.set_graph_snapshot(&graph)
    }
    pub fn set_graph_snapshot(&mut self, graph: &Graph) -> Result<u32, String> {
        let prepared = self.pipeline.prepare_graph(graph)?;
        self.commit_graph(prepared)
    }
    pub fn commit_graph(&mut self, prepared: PreparedNativeGraph) -> Result<u32, String> {
        let revision = self.pipeline.commit_graph(prepared)?;
        self.last_stamp = None;
        Ok(revision)
    }
    pub fn output_payload(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
        RenderDevice::output_payload(rgba, width, height)
    }
    pub fn start_playback(&mut self) -> Result<(), String> {
        if self.pipeline.frame_state() != crate::evaluation::EngineState::Ready {
            return Err("Playback can only start from a ready graph".into());
        }
        #[cfg(feature = "native-video")]
        self.pipeline.videos.resume()?;
        self.clock.start(self.clock_now_ns());
        Ok(())
    }
    fn clock_now_ns(&self) -> u64 {
        self.clock_origin.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
    }
    pub fn render_next(&mut self) -> Result<NativeFrameRendered, String> {
        self.render(self.clock_now_ns())
    }
    pub fn pause(&mut self) -> Result<(), String> {
        let now = self.clock_now_ns();
        self.pipeline.pause_execution()?;
        self.clock.pause(now).map_err(runtime_error)?;
        #[cfg(feature = "native-video")]
        self.pipeline.videos.pause();
        Ok(())
    }
    pub fn resume(&mut self) -> Result<(), String> {
        let now = self.clock_now_ns();
        self.pipeline.resume_execution()?;
        #[cfg(feature = "native-video")]
        self.pipeline.videos.resume()?;
        self.clock.resume(now).map_err(runtime_error)
    }
    pub fn stop(&mut self) -> Result<(), String> {
        self.pipeline.stop_execution()?;
        #[cfg(feature = "native-video")]
        self.pipeline.videos.pause();
        self.clock.stop();
        Ok(())
    }
    pub fn dispose(&mut self) -> Result<(), String> {
        self.pipeline.dispose()?;
        self.clock.stop();
        Ok(())
    }
    pub fn close(&mut self) -> Result<(), String> { self.dispose() }
    pub fn set_mouse(&mut self, mouse: [f32; 4]) { self.mouse = mouse; }

    fn render(&mut self, now_ns: u64) -> Result<NativeFrameRendered, String> {
        let started = Instant::now();
        let mut completion = self.pipeline.poll_completion();
        if completion.as_ref().is_some_and(|value| matches!(value.outcome, FrameOutcome::Cancelled)) {
            completion = None;
        }
        let output_node_id = self.pipeline.session.execution_plan()
            .ok_or("Native engine has no execution plan")?.output_nodes.first().cloned()
            .ok_or("Graph has no renderer or terminal texture output")?;
        if completion.is_none() && !self.pipeline.has_in_flight() {
            let mut candidate_clock = self.clock.clone();
            let clock = candidate_clock.tick(now_ns).map_err(runtime_error)?;
            let stamp = FrameStamp { epoch: clock.epoch, frame: clock.frame,
                timeline_ns: clock.timeline_ns, deadline_ns: clock.next_deadline_ns };
            let plan = self.pipeline.session.execution_plan().expect("graph checked");
            let demand = FrameDemand { request_id: clock.frame, graph_revision: self.pipeline.revision(),
                input: FrameEvaluationInput { stamp: stamp.clone(), previous_timeline_ns: clock.previous_timeline_ns,
                    date: utc_date_uniform(SystemTime::now()), mouse: self.mouse,
                    resolution: [plan.default_width.max(1) as f32, plan.default_height.max(1) as f32, 1.0] },
                output_node_id: output_node_id.clone(), sources: Vec::new() };
            match self.pipeline.submit(demand) {
                FrameAdmission::Accepted => { self.clock = candidate_clock; self.last_stamp = Some(stamp); }
                FrameAdmission::WouldBlock(_) => {}
                FrameAdmission::Rejected { error, .. } => return Err(error),
            }
        }
        self.pipeline.progress();
        let mut stamp = self.last_stamp.clone().unwrap_or(FrameStamp { epoch: 0,frame: 0,timeline_ns:0,deadline_ns:0 });
        let mut dimensions = (0, 0);
        let mut output_node_id = output_node_id;
        if let Some(completion) = completion.or_else(|| self.pipeline.poll_completion()) {
            stamp = completion.stamp;
            match completion.outcome {
                FrameOutcome::Submitted(frame) => {
                    dimensions = (frame.image.width, frame.image.height);
                    output_node_id = frame.identity.output_node_id;
                    self.pipeline.render_device.present(&output_node_id, stamp.frame, stamp.timeline_ns)?;
                }
                FrameOutcome::Cancelled => {}
                FrameOutcome::Failed(error) => return Err(error),
            }
        }
        #[cfg(feature = "native-video")]
        let (decoded, uploaded) = self.pipeline.videos.frame_counts();
        #[cfg(not(feature = "native-video"))]
        let (decoded, uploaded) = (0,0);
        self.perf.record(std::time::Duration::ZERO, std::time::Duration::ZERO,
            started.elapsed(), started.elapsed(), decoded, uploaded);
        Ok(NativeFrameRendered { frame: stamp.frame, timeline_ns: stamp.timeline_ns,
            revision: self.pipeline.revision(), output_node_id, width: dimensions.0, height: dimensions.1 })
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

    #[test]
    fn strict_inference_policy_rejects_preparation_before_allocation_or_readback() {
        let (mut host, _) = pollster::block_on(NativePlayer::new()).unwrap();
        let graph: crate::Graph = serde_json::from_value(serde_json::json!({
            "nodes": [{"id":"image", "type":"input", "position":{"x":0,"y":0},
                "data":{"type":"input", "label":"Image", "shaderCode":"", "inputs":[],
                    "outputs":[{"id":"out","label":"image","dataType":"sampler2D","direction":"output"}],
                    "uniforms":{}, "inputMode":"image", "inputDataType":"sampler2D"}}],
            "edges":[]
        })).unwrap();
        let revision = host.set_graph_snapshot(&graph).unwrap();
        host.upload_image("image", &[1, 2, 3, 255], 1, 1).unwrap();
        let generation = host.pipeline.session.node_generation("image").unwrap();
        let plan = serde_json::to_value(host.pipeline.session.execution_plan().unwrap()).unwrap();
        let state = host.pipeline.session.state();
        let mut candidate = graph.clone();
        let mut inference = candidate.nodes[0].clone();
        inference.id = "strict_model".to_owned();
        inference.node_type = crate::NodeType::Inference;
        inference.data.node_type = crate::NodeType::Inference;
        inference.data.width = Some(host.pipeline.render_device.device().limits().max_texture_dimension_2d + 1);
        inference.data.auto_size = Some(false);
        inference.data.inference_policy = Some(crate::inference::ExecutionPolicy {
            residency: crate::inference::ResidencyPolicy::RequireResident,
            ..crate::inference::ExecutionPolicy::default()
        });
        candidate.nodes.push(inference);

        let error = match host.prepare_graph(&candidate) {
            Ok(_) => panic!("strict residency must be rejected before render preparation"),
            Err(error) => error,
        };
        assert!(error.contains("strict_model"), "{error}");
        assert!(error.to_lowercase().contains("residen"), "{error}");
        assert_eq!(host.session.revision(), revision);
        assert_eq!(host.session.node_generation("image").unwrap(), generation);
        assert_eq!(host.session.state(), state);
        assert_eq!(serde_json::to_value(host.session.execution_plan().unwrap()).unwrap(), plan);
        assert!(host.has_output_texture("image"));
        assert!(!host.has_output_texture("strict_model"));
        assert_eq!(host.inference_metrics().readback_bytes, 0);
        assert_eq!(host.inference_metrics().submitted, 0);
    }


    #[test]
    fn stable_monitor_rejects_impossible_working_sets_instead_of_permanent_backpressure() {
        use crate::rendering::{FrameCaptureError, GpuOutputHandle, StableFrameBudget, StableFramePool};
        let (mut host, _) = pollster::block_on(NativePlayer::new()).unwrap();
        assert!(matches!(host.try_stable_preview("missing"), Err(FrameCaptureError::InvalidSource(_))));
        host.pipeline.render_device.upload_rgba("image", &[10, 20, 30, 255], 1, 1).unwrap();
        for budget in [StableFrameBudget { max_frames: 1, max_bytes: 8 }, StableFrameBudget { max_frames: 2, max_bytes: 7 }] {
            host.pipeline.stable_previews = StableFramePool::new(host.backend(), budget).unwrap();
            assert!(matches!(host.try_stable_preview("image"), Err(FrameCaptureError::Capacity)));
        }
        host.pipeline.stable_previews = StableFramePool::new(host.backend(), StableFrameBudget { max_frames: 2, max_bytes: 8 }).unwrap();
        let first = host.try_stable_preview("image").unwrap().unwrap();
        let second = host.try_stable_preview("image").unwrap().unwrap();
        assert!(host.try_stable_preview("image").unwrap().is_none());
        first.ready.wait(std::time::Duration::from_secs(5)).unwrap();
        second.ready.wait(std::time::Duration::from_secs(5)).unwrap();
        let float = host.backend().create_target(1, 1, crate::rendering::TextureFormat::Rgba32Float);
        let sampler = std::sync::Arc::new(host.backend().device.create_sampler(&Default::default()));
        host.pipeline.render_device.register_external_texture("float", GpuOutputHandle {
            texture: float.texture, view: float.view, sampler,
            width: 1, height: 1, format: crate::rendering::TextureFormat::Rgba32Float, lease: None,
        }).unwrap();
        assert!(matches!(host.try_stable_preview("float"), Err(FrameCaptureError::InvalidSource(_))));
    }

}
