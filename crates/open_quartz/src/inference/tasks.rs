use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceOutputEvent {
    pub node_id: String,
    pub width: u32,
    pub height: u32,
    pub backend: String,
    pub data: Option<serde_json::Value>,
}

/// Host wall durations describe submission overhead, not measured GPU execution.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceMetrics {
    pub submitted: u64,
    /// Completions ready for graph publication; not physical GPU completion fences.
    pub completed: u64,
    pub accepted: u64,
    pub failed: u64,
    pub rejected: u64,
    pub stale: u64,
    pub cancelled: u64,
    pub queued: u64,
    pub in_flight: usize,
    pub queue_depth: usize,
    pub readback_bytes: u64,
    pub readback_ns: u64,
    pub upload_bytes: u64,
    pub upload_ns: u64,
    pub preprocess_ns: u64,
    pub runtime_ns: u64,
    pub postprocess_ns: u64,
    pub worker_queue_ns: u64,
    pub input_cpu_bytes: u64,
    pub output_cpu_bytes: u64,
    pub last_inference_ns: u64,
    pub last_readback_ns: u64,
    pub last_upload_ns: u64,
    pub device_submissions: u64,
    pub compute_dispatches: u64,
    pub submission_wall_ns: u64,
    pub gpu_time_ns: Option<u64>,
}

#[cfg(feature = "native-runtime")]
pub(crate) use resident::*;

#[cfg(feature = "native-runtime")]
mod resident {
    use std::collections::{HashMap, HashSet, VecDeque};
    use std::sync::Arc;
    use oq_inference::gpu::GpuOutput;
    use oq_inference::catalog_gpu::CatalogGpuOutput;
    use crate::inference::ResidentInferenceSession;
    use crate::evaluation::ExecutionCommand;
    use crate::inference::{InferenceConfig, InferenceSessionInfo, RequestStamp, SessionGeneration};
    use crate::outputs::{AsyncCompletionEnvelope, ContentStamp, DeliveryPolicy, FrameStamp,
        OutputKey, OutputPayload, OutputSubscription, OutputTransport};
    use crate::player::PlaybackSession;
    use crate::project::model::{DataType, Graph, NodeType};
    use crate::rendering::{GpuOutputHandle, RenderDevice, TextureFormat};
    use super::{InferenceMetrics, InferenceOutputEvent};

    struct Resource {
        session: ResidentInferenceSession,
        info: InferenceSessionInfo,
    }

    pub(crate) enum ResidentImage {
        Gpu(GpuOutput),
        Catalog(CatalogGpuOutput),
        #[cfg(test)]
        Test(GpuOutputHandle),
    }
    impl ResidentImage {
        pub(crate) fn dimensions(&self) -> (u32, u32) {
            match self {
                Self::Gpu(output) => (output.width(), output.height()),
                Self::Catalog(output) => (output.width(), output.height()),
                #[cfg(test)]
                Self::Test(output) => (output.width, output.height),
            }
        }
        pub(crate) fn handle(&self, sampler: Arc<wgpu::Sampler>) -> GpuOutputHandle {
            match self {
                Self::Gpu(output) => GpuOutputHandle {
                    texture: output.texture_arc(), view: output.view_arc(), sampler,
                    width: output.width(), height: output.height(), format: TextureFormat::Rgba8Unorm,
                    lease: Some(output.lease()),
                },
                Self::Catalog(output) => GpuOutputHandle {
                    texture: output.texture_arc(), view: output.view_arc(), sampler,
                    width: output.width(), height: output.height(), format: TextureFormat::Rgba8Unorm,
                    lease: Some(output.lease()),
                },
                #[cfg(test)]
                Self::Test(output) => output.clone(),
            }
        }
        pub(crate) fn detection_buffer(&self) -> Option<Arc<wgpu::Buffer>> {
            match self { Self::Catalog(output) => output.detection_buffer(), _ => None }
        }
    }
    struct Completion {
        node_id: String,
        task_id: u64,
        stamp: RequestStamp,
        revision: u32,
        generation: u32,
        input_stamp: FrameStamp,
        result: Result<ResidentImage, String>,
    }
    #[derive(Clone, Copy)]
    struct Active {
        task_id: u64,
        session_generation: SessionGeneration,
        revision: u32,
        generation: u32,
    }
    pub(crate) struct AcceptedInferenceCompletion {
        pub node_id: String,
        pub image: ResidentImage,
        pub envelope: AsyncCompletionEnvelope,
        pub event: InferenceOutputEvent,
        subscription_id: Option<String>,
    }
    /// Render-thread coordinator. All inference submissions use the renderer's device and queue.
    pub(crate) struct InferenceTasks {
        resources: HashMap<String, Resource>,
        active: HashMap<String, Active>,
        completions: VecDeque<Completion>,
        queued_nodes: HashSet<String>,
        ready_nodes: HashSet<String>,
        retained_images: HashMap<String, ResidentImage>,
        outputs: HashMap<String, Vec<(OutputKey, DataType)>>,
        event_subscriptions: HashMap<String, String>,
        output_events: Vec<InferenceOutputEvent>,
        metrics: InferenceMetrics,
        next_task: u64,
    }
    impl InferenceTasks {
        pub(crate) fn new() -> Self {
            Self { resources: HashMap::new(), active: HashMap::new(), completions: VecDeque::new(),
                queued_nodes: HashSet::new(), ready_nodes: HashSet::new(), retained_images: HashMap::new(),
                outputs: HashMap::new(), event_subscriptions: HashMap::new(), output_events: Vec::new(),
                metrics: InferenceMetrics::default(), next_task: 0 }
        }
        pub(crate) fn sync_graph(&mut self, graph: &Graph, session: &mut PlaybackSession) -> Result<(), String> {
            self.cancel_pending();
            for (_, id) in self.event_subscriptions.drain() { let _ = session.unsubscribe_output(&id); }
            self.outputs = graph.nodes.iter().filter(|node| node.node_type == NodeType::Inference)
                .map(|node| (node.id.clone(), node.data.outputs.iter()
                    .map(|port| (OutputKey::new(&node.id, &port.id), port.data_type)).collect())).collect();
            self.resources.retain(|node, _| self.outputs.contains_key(node));
            self.retained_images.retain(|node, _| self.outputs.contains_key(node));
            for (node, outputs) in &self.outputs {
                let Some((output, _)) = outputs.iter().find(|(_, kind)| matches!(kind, DataType::Sampler2d | DataType::SamplerCube)) else { continue; };
                let id = format!("native-output-{node}");
                session.subscribe_output(OutputSubscription { subscription_id: id.clone(), output: output.clone(),
                    delivery: DeliveryPolicy::OnChange, transport: OutputTransport::Preview,
                    max_width: None, max_height: None }).map_err(|error| error.to_json())?;
                self.event_subscriptions.insert(node.clone(), id);
            }
            Ok(())
        }
        pub(crate) fn load(&mut self, node: String, session: ResidentInferenceSession,
            config: InferenceConfig, info: InferenceSessionInfo) -> Result<InferenceSessionInfo, String> {
            super::super::prepared::validate_resident_config(&config)?;
            crate::inference::image_task_contract(&config, &info)?;
            self.cancel_node(&node);
            self.resources.insert(node.clone(), Resource { session, info: info.clone() });
            self.retained_images.remove(&node);
            self.output_events.retain(|event| event.node_id != node);
            Ok(info)
        }
        pub(crate) fn unload(&mut self, node: &str) {
            self.cancel_node(node);
            self.resources.remove(node);
            self.retained_images.remove(node);
            self.output_events.retain(|event| event.node_id != node);
        }
        fn cancel_node(&mut self, node: &str) {
            if self.active.remove(node).is_some() { self.metrics.cancelled += 1; }
            self.completions.retain(|completion| completion.node_id != node);
            self.queued_nodes.remove(node);
            self.ready_nodes.remove(node);
        }
        pub(crate) fn cancel_pending(&mut self) {
            self.metrics.cancelled = self.metrics.cancelled.saturating_add(self.active.len() as u64);
            self.active.clear();
            self.completions.clear();
            self.queued_nodes.clear();
            self.ready_nodes.clear();
            self.output_events.clear();
        }
        pub(crate) fn clear(&mut self) {
            self.cancel_pending(); self.resources.clear(); self.retained_images.clear();
            self.outputs.clear(); self.event_subscriptions.clear();
        }
        pub(crate) fn has_pending(&self) -> bool { !self.active.is_empty() || !self.queued_nodes.is_empty() }
        pub(crate) fn metrics(&self) -> InferenceMetrics {
            let mut metrics = self.metrics.clone();
            metrics.in_flight = self.active.len(); metrics.queue_depth = self.queued_nodes.len(); metrics
        }
        pub(crate) fn take_ready_nodes(&mut self) -> Vec<String> { self.ready_nodes.drain().collect() }
        pub(crate) fn detection_buffer(&self, node: &str) -> Option<(Arc<wgpu::Buffer>, Arc<dyn std::any::Any + Send + Sync>)> {
            match self.retained_images.get(node)? {
                ResidentImage::Catalog(output) => Some((output.detection_buffer()?, output.lease())),
                _ => None,
            }
        }
        pub(crate) fn raw_result_buffer(&self, node: &str) -> Option<(Arc<wgpu::Buffer>, Arc<dyn std::any::Any + Send + Sync>)> {
            match self.retained_images.get(node)? {
                ResidentImage::Catalog(output) => Some((output.raw_result_buffer(), output.lease())),
                _ => None,
            }
        }
        pub(crate) fn schedule(&mut self, command: &ExecutionCommand, input_stamp: &FrameStamp,
            playback: &PlaybackSession, render: &RenderDevice) -> Result<(), String> {
            if self.active.contains_key(&command.node_id) {
                self.metrics.queued += 1; self.queued_nodes.insert(command.node_id.clone()); return Ok(());
            }
            let Some(resource) = self.resources.get_mut(&command.node_id) else { return Ok(()); };
            if command.texture_inputs.len() != 1 {
                self.metrics.rejected += 1;
                return Err(format!("Inference {} requires exactly one image input", command.node_id));
            }
            let source_id = command.texture_inputs.values().next().expect("single input checked");
            let Some(source) = render.output_texture(source_id) else { return Ok(()); };
            let generation = playback.node_generation(&command.node_id).map_err(|error| error.to_json())?;
            let task_id = self.next_task.checked_add(1).ok_or("Inference request ID exhausted")?;
            let before = resource.session.counters();
            let result = match resource.session.submit_texture(source.texture, source.width, source.height) {
                Ok(output) => Ok(output),
                Err(oq_inference::gpu::GpuError::Capacity) => {
                    self.metrics.queued += 1;
                    self.queued_nodes.insert(command.node_id.clone());
                    self.ready_nodes.insert(command.node_id.clone());
                    return Ok(());
                }
                Err(error) => Err(error.to_string()),
            };
            self.next_task = task_id;
            self.queued_nodes.remove(&command.node_id);
            self.ready_nodes.remove(&command.node_id);
            let stamp = RequestStamp { request_id: task_id, session_generation: resource.info.generation };
            let after = resource.session.counters();
            let submissions = after.submissions.saturating_sub(before.submissions);
            self.metrics.device_submissions += submissions;
            self.metrics.compute_dispatches += after.compute_dispatches.saturating_sub(before.compute_dispatches);
            let wall = if submissions == 0 { 0 } else { after.last_wall_time.as_nanos().min(u64::MAX as u128) as u64 };
            self.metrics.submission_wall_ns = self.metrics.submission_wall_ns.saturating_add(wall);
            self.metrics.last_inference_ns = wall;
            self.metrics.gpu_time_ns = after.gpu_time.map(|duration| duration.as_nanos().min(u64::MAX as u128) as u64);
            self.active.insert(command.node_id.clone(), Active { task_id, session_generation: stamp.session_generation,
                revision: playback.revision(), generation });
            self.completions.push_back(Completion { node_id: command.node_id.clone(), task_id, stamp,
                revision: playback.revision(), generation, input_stamp: input_stamp.clone(), result });
            self.metrics.submitted += 1;
            Ok(())
        }
        pub(crate) fn next_completion(&mut self, playback: &PlaybackSession) -> Result<Option<AcceptedInferenceCompletion>, String> {
            while let Some(completion) = self.completions.pop_front() {
                self.metrics.completed += 1;
                let matches = self.active.get(&completion.node_id).is_some_and(|active|
                    active.task_id == completion.task_id && completion.stamp.request_id == active.task_id
                    && active.session_generation == completion.stamp.session_generation
                    && active.revision == completion.revision && active.generation == completion.generation);
                if !matches || completion.revision != playback.revision()
                    || playback.node_generation(&completion.node_id).ok() != Some(completion.generation) {
                    self.metrics.stale += 1; continue;
                }
                self.active.remove(&completion.node_id);
                if self.queued_nodes.remove(&completion.node_id) { self.ready_nodes.insert(completion.node_id.clone()); }
                let image = completion.result.map_err(|error| {
                    self.metrics.failed += 1; format!("Inference {} failed: {error}", completion.node_id)
                })?;
                let (width, height) = image.dimensions();
                let outputs = self.outputs.get(&completion.node_id).into_iter().flatten()
                    .filter_map(|(key, data_type)| (matches!(data_type, DataType::Sampler2d | DataType::SamplerCube)
                        || (*data_type == DataType::Roi && image.detection_buffer().is_some()))
                        .then(|| (key.clone(), OutputPayload::Resource { handle: u64::from(completion.generation) }))).collect();
                let envelope = AsyncCompletionEnvelope { node_id: completion.node_id.clone(),
                    graph_revision: completion.revision, node_generation: completion.generation,
                    input_stamp: completion.input_stamp.clone(), content_stamp: ContentStamp {
                        epoch: completion.input_stamp.epoch, timeline_ns: completion.input_stamp.timeline_ns, media_pts_ns: None }, outputs };
                let event = InferenceOutputEvent { node_id: completion.node_id.clone(), width, height,
                    backend: "metal".to_owned(), data: None };
                let subscription_id = self.event_subscriptions.get(&completion.node_id).cloned();
                return Ok(Some(AcceptedInferenceCompletion { node_id: completion.node_id, image, envelope, event, subscription_id }));
            }
            Ok(None)
        }
        /// Retain the output lease while the graph can sample its registered native texture.
        pub(crate) fn record_delivery(&mut self, accepted: AcceptedInferenceCompletion,
            subscriptions: impl IntoIterator<Item = String>) {
            self.metrics.accepted += 1;
            self.retained_images.insert(accepted.node_id.clone(), accepted.image);
            if let Some(id) = accepted.subscription_id {
                if self.event_subscriptions.get(&accepted.node_id) == Some(&id)
                    && subscriptions.into_iter().any(|delivered| delivered == id) { self.output_events.push(accepted.event); }
            }
        }
        pub(crate) fn record_rejection(&mut self) { self.metrics.rejected += 1; }
        pub(crate) fn take_output_events(&mut self) -> Vec<InferenceOutputEvent> { std::mem::take(&mut self.output_events) }
        #[cfg(test)]
        pub(crate) fn queue_test_completion(&mut self, node: &str, playback: &PlaybackSession, image: GpuOutputHandle) {
            self.next_task += 1;
            let active = Active { task_id: self.next_task, session_generation: SessionGeneration(0),
                revision: playback.revision(), generation: playback.node_generation(node).unwrap() };
            self.active.insert(node.to_owned(), active);
            self.completions.push_back(Completion { node_id: node.to_owned(), task_id: active.task_id,
                stamp: RequestStamp { request_id: active.task_id, session_generation: active.session_generation },
                revision: active.revision, generation: active.generation,
                input_stamp: FrameStamp { epoch: 1, frame: active.task_id, timeline_ns: active.task_id, deadline_ns: active.task_id },
                result: Ok(ResidentImage::Test(image)) });
        }
    }
    #[cfg(test)]
    mod tests { include!("resident_tasks_coverage_tests.rs"); }
}
