use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeOutputEvent {
    pub node_id: String,
    pub width: u32,
    pub height: u32,
    pub backend: String,
    pub data: Option<serde_json::Value>,
}

#[cfg(all(feature = "native-onnx", not(target_arch = "wasm32")))]
pub(crate) use native::InferenceTasks;

#[cfg(all(feature = "native-onnx", not(target_arch = "wasm32")))]
mod native {
    use std::collections::HashMap;
    use std::sync::{mpsc, Arc, Mutex};
    use std::thread::JoinHandle;

    use crate::evaluation::ExecutionCommand;
    use crate::inference::{NativeOnnxConfig, NativeOnnxImageOutput, OnnxSession, OnnxSessionInfo};
    use crate::outputs::{
        AsyncCompletionEnvelope, ContentStamp, DeliveryPolicy, FrameStamp, OutputKey,
        OutputPayload, OutputSubscription, OutputTransport,
    };
    use crate::player::PlaybackSession;
    use crate::project::model::{DataType, Graph, NodeType};

    use super::NativeOutputEvent;

    struct NativeOnnxResource {
        session: Arc<Mutex<OnnxSession>>,
        config: NativeOnnxConfig,
        backend: String,
    }

    struct NativeOnnxCompletion {
        node_id: String,
        task_id: u64,
        revision: u32,
        generation: u32,
        input_stamp: FrameStamp,
        result: Result<NativeOnnxImageOutput, String>,
    }

    /// Validated on the render thread before the coordinator changes any GPU resources.
    pub(crate) struct AcceptedInferenceCompletion {
        pub node_id: String,
        pub image: NativeOnnxImageOutput,
        pub envelope: AsyncCompletionEnvelope,
        pub event: NativeOutputEvent,
        subscription_id: Option<String>,
    }

    /// Owns inference resources and workers independently of GPU resource mutation.
    pub(crate) struct InferenceTasks {
        resources: HashMap<String, NativeOnnxResource>,
        pending: HashMap<String, u64>,
        next_task: u64,
        outputs: HashMap<String, Vec<(OutputKey, DataType)>>,
        event_subscriptions: HashMap<String, String>,
        sender: mpsc::Sender<NativeOnnxCompletion>,
        receiver: mpsc::Receiver<NativeOnnxCompletion>,
        workers: Vec<JoinHandle<()>>,
        output_events: Vec<NativeOutputEvent>,
    }

    impl InferenceTasks {
        pub(crate) fn new() -> Self {
            let (sender, receiver) = mpsc::channel();
            Self {
                resources: HashMap::new(),
                pending: HashMap::new(),
                next_task: 0,
                outputs: HashMap::new(),
                event_subscriptions: HashMap::new(),
                sender,
                receiver,
                workers: Vec::new(),
                output_events: Vec::new(),
            }
        }

        #[cfg(test)]
        pub(crate) fn queue_test_completion(
            &mut self,
            node_id: &str,
            session: &PlaybackSession,
            image: NativeOnnxImageOutput,
        ) {
            let generation = session.node_generation(node_id)
                .expect("test completion requires a current graph node");
            self.next_task = self.next_task.checked_add(1)
                .expect("test completion task ID exhausted");
            let task_id = self.next_task;
            self.pending.insert(node_id.to_owned(), task_id);
            self.sender.send(NativeOnnxCompletion {
                node_id: node_id.to_owned(),
                task_id,
                revision: session.revision(),
                generation,
                input_stamp: FrameStamp {
                    epoch: 1,
                    frame: task_id,
                    timeline_ns: task_id,
                    deadline_ns: task_id,
                },
                result: Ok(image),
            }).expect("test completion receiver remains owned by InferenceTasks");
        }

        /// Call after PlaybackSession::set_graph has installed the new output contracts.
        pub(crate) fn sync_graph(
            &mut self,
            graph: &Graph,
            session: &mut PlaybackSession,
        ) -> Result<(), String> {
            self.cancel_pending();
            for (_, subscription_id) in self.event_subscriptions.drain() {
                // Graph reconciliation already removes subscriptions for deleted or changed ports.
                // Unsubscribe the remaining ones before registering this graph's subscriptions.
                let _ = session.unsubscribe_output(&subscription_id);
            }
            self.outputs = graph
                .nodes
                .iter()
                .filter(|node| node.node_type == NodeType::Onnx)
                .map(|node| {
                    let outputs = node
                        .data
                        .outputs
                        .iter()
                        .map(|port| {
                            (OutputKey::new(&node.id, &port.id), port.data_type)
                        })
                        .collect();
                    (node.id.clone(), outputs)
                })
                .collect();
            self.resources.retain(|node_id, _| self.outputs.contains_key(node_id));
            for (node_id, outputs) in &self.outputs {
                let selected = outputs
                    .iter()
                    .find(|(_, data_type)| {
                        matches!(data_type, DataType::Roi | DataType::Mesh | DataType::Json)
                    })
                    .or_else(|| outputs.first());
                let Some((output, data_type)) = selected else {
                    continue;
                };
                let subscription_id = format!("native-output-{node_id}");
                session
                    .subscribe_output(OutputSubscription {
                        subscription_id: subscription_id.clone(),
                        output: output.clone(),
                        delivery: DeliveryPolicy::OnChange,
                        transport: if matches!(data_type, DataType::Sampler2d | DataType::SamplerCube) {
                            OutputTransport::Preview
                        } else {
                            OutputTransport::Value
                        },
                        max_width: None,
                        max_height: None,
                    })
                    .map_err(|error| error.to_json())?;
                self.event_subscriptions.insert(node_id.clone(), subscription_id);
            }
            Ok(())
        }

        /// The coordinator validates the node, removes its texture, and marks it dirty.
        pub(crate) fn load(
            &mut self,
            node_id: String,
            session: OnnxSession,
            config: NativeOnnxConfig,
        ) -> OnnxSessionInfo {
            let info = session.info().clone();
            self.resources.insert(
                node_id.clone(),
                NativeOnnxResource {
                    session: Arc::new(Mutex::new(session)),
                    config,
                    backend: info.backend.clone(),
                },
            );
            self.pending.remove(&node_id);
            self.output_events.retain(|event| event.node_id != node_id);
            info
        }

        pub(crate) fn unload(&mut self, node_id: &str) {
            self.resources.remove(node_id);
            self.pending.remove(node_id);
            self.output_events.retain(|event| event.node_id != node_id);
        }

        /// Workers retain their sessions until they finish; their results are now obsolete.
        pub(crate) fn cancel_pending(&mut self) {
            self.pending.clear();
            self.output_events.clear();
        }

        /// Call only after PlaybackSession::dispose succeeds and removes its subscriptions.
        /// Workers keep their own session references until completion or the blocking Drop join.
        pub(crate) fn clear(&mut self) {
            self.resources.clear();
            self.cancel_pending();
            self.outputs.clear();
            self.event_subscriptions.clear();
        }

        pub(crate) fn has_pending(&self) -> bool {
            !self.pending.is_empty()
        }

        pub(crate) fn schedule(
            &mut self,
            command: &ExecutionCommand,
            input_stamp: &FrameStamp,
            session: &PlaybackSession,
            read_source: impl FnOnce(&str) -> Result<Option<(Vec<u8>, u32, u32)>, String>,
        ) -> Result<(), String> {
            if self.pending.contains_key(&command.node_id) {
                return Ok(());
            }
            let Some(resource) = self.resources.get(&command.node_id) else {
                return Ok(());
            };
            let onnx_session = resource.session.clone();
            let config = resource.config.clone();
            let Some(source_id) = command.texture_inputs.values().next() else {
                return Ok(());
            };
            let Some((source_rgba, source_width, source_height)) = read_source(source_id)? else {
                return Ok(());
            };
            let sender = self.sender.clone();
            let node_id = command.node_id.clone();
            let revision = session.revision();
            let generation = session.node_generation(&node_id).map_err(|error| error.to_json())?;
            let input_stamp = input_stamp.clone();
            self.next_task = self.next_task.checked_add(1)
                .ok_or_else(|| "Native ONNX task ID exhausted".to_owned())?;
            let task_id = self.next_task;
            self.pending.insert(node_id.clone(), task_id);
            let worker = std::thread::Builder::new()
                .name(format!("open-quartz-onnx-{node_id}"))
                .spawn(move || {
                    let result = onnx_session
                        .lock()
                        .map_err(|_| "Native ONNX session lock is poisoned".to_owned())
                        .and_then(|mut session| {
                            crate::inference::run_native_image_task(
                                &mut session,
                                config.task,
                                &config.model_id,
                                &source_rgba,
                                source_width,
                                source_height,
                                config.target_size,
                                config.score_threshold,
                                config.iou_threshold,
                            )
                        });
                    let _ = sender.send(NativeOnnxCompletion {
                        node_id,
                        task_id,
                        revision,
                        generation,
                        input_stamp,
                        result,
                    });
                })
                .map_err(|error| {
                    self.pending.remove(&command.node_id);
                    format!("Cannot start native ONNX task: {error}")
                })?;
            self.workers.push(worker);
            Ok(())
        }

        pub(crate) fn next_completion(
            &mut self,
            session: &PlaybackSession,
        ) -> Result<Option<AcceptedInferenceCompletion>, String> {
            let mut running = Vec::new();
            for worker in self.workers.drain(..) {
                if worker.is_finished() {
                    let _ = worker.join();
                } else {
                    running.push(worker);
                }
            }
            self.workers = running;
            while let Ok(completion) = self.receiver.try_recv() {
                // Check every identity component before clearing pending state or exposing an image.
                // In particular, an old task must never clear a newer task for the same node.
                if self.pending.get(&completion.node_id) != Some(&completion.task_id)
                    || completion.revision != session.revision()
                    || session.node_generation(&completion.node_id).ok() != Some(completion.generation)
                {
                    continue;
                }
                self.pending.remove(&completion.node_id);
                let image = completion.result.map_err(|error| {
                    format!("ONNX {} inference failed: {error}", completion.node_id)
                })?;
                let backend = self.resources.get(&completion.node_id)
                    .map(|resource| resource.backend.clone())
                    .unwrap_or_else(|| "native".to_owned());
                let outputs = self.outputs.get(&completion.node_id)
                    .into_iter()
                    .flatten()
                    .filter_map(|(key, data_type)| {
                        let payload = match data_type {
                            DataType::Sampler2d | DataType::SamplerCube => OutputPayload::Resource {
                                handle: u64::from(completion.generation),
                            },
                            DataType::Roi | DataType::Mesh | DataType::Json | DataType::Auto => {
                                OutputPayload::Json(image.data.clone().unwrap_or(serde_json::Value::Null))
                            }
                            _ => return None,
                        };
                        Some((key.clone(), payload))
                    })
                    .collect();
                let envelope = AsyncCompletionEnvelope {
                    node_id: completion.node_id.clone(),
                    graph_revision: completion.revision,
                    node_generation: completion.generation,
                    input_stamp: completion.input_stamp.clone(),
                    content_stamp: ContentStamp {
                        epoch: completion.input_stamp.epoch,
                        timeline_ns: completion.input_stamp.timeline_ns,
                        media_pts_ns: None,
                    },
                    outputs,
                };
                let event = NativeOutputEvent {
                    node_id: completion.node_id.clone(),
                    width: image.width,
                    height: image.height,
                    backend,
                    data: image.data.clone(),
                };
                let subscription_id = self.event_subscriptions.get(&completion.node_id).cloned();
                return Ok(Some(AcceptedInferenceCompletion {
                    node_id: completion.node_id,
                    image,
                    envelope,
                    event,
                    subscription_id,
                }));
            }
            Ok(None)
        }

        /// Call after GPU upload and successful submission to PlaybackSession.
        pub(crate) fn record_delivery(
            &mut self,
            accepted: AcceptedInferenceCompletion,
            delivered_subscription_ids: impl IntoIterator<Item = String>,
        ) {
            if let Some(subscription_id) = accepted.subscription_id {
                if self.event_subscriptions.get(&accepted.node_id) == Some(&subscription_id)
                    && delivered_subscription_ids.into_iter().any(|id| id == subscription_id)
                {
                    self.output_events.push(accepted.event);
                }
            }
        }

        pub(crate) fn take_output_events(&mut self) -> Vec<NativeOutputEvent> {
            std::mem::take(&mut self.output_events)
        }
    }

    impl Drop for InferenceTasks {
        fn drop(&mut self) {
            for worker in self.workers.drain(..) {
                let _ = worker.join();
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::outputs::PlaybackCapabilities;

        fn graph() -> Graph {
            serde_json::from_value(serde_json::json!({"nodes":[{
                "id":"onnx","type":"onnx","position":{"x":0,"y":0},
                "data":{"type":"onnx","label":"Model","shaderCode":"","inputs":[],
                    "outputs":[{"id":"out","label":"output","dataType":"sampler2D","direction":"output"}],
                    "uniforms":{}}
            }],"edges":[]})).unwrap()
        }

        fn setup() -> (InferenceTasks, PlaybackSession) {
            let mut session = PlaybackSession::new_native(PlaybackCapabilities { data_paths: vec![] });
            let graph = graph();
            session.set_graph(&graph).unwrap();
            let mut tasks = InferenceTasks::new();
            tasks.sync_graph(&graph, &mut session).unwrap();
            (tasks, session)
        }

        fn completion(session: &PlaybackSession, task_id: u64) -> NativeOnnxCompletion {
            NativeOnnxCompletion {
                node_id: "onnx".into(),
                task_id,
                revision: session.revision(),
                generation: session.node_generation("onnx").unwrap(),
                input_stamp: FrameStamp { epoch: 1, frame: 1, timeline_ns: 0, deadline_ns: 0 },
                result: Ok(NativeOnnxImageOutput {
                    rgba: vec![255, 0, 0, 255], width: 1, height: 1, data: None,
                }),
            }
        }

        #[test]
        fn unloaded_and_superseded_completions_cannot_restore_textures_or_clear_current_work() {
            let (mut tasks, session) = setup();
            tasks.pending.insert("onnx".into(), 1);
            tasks.unload("onnx");
            tasks.sender.send(completion(&session, 1)).unwrap();
            // No accepted image means the coordinator cannot restore a texture.
            assert!(tasks.next_completion(&session).unwrap().is_none());
            assert!(!tasks.has_pending());
            assert!(tasks.take_output_events().is_empty());
            tasks.pending.insert("onnx".into(), 2);
            tasks.sender.send(completion(&session, 1)).unwrap();
            assert!(tasks.next_completion(&session).unwrap().is_none());
            assert_eq!(tasks.pending.get("onnx"), Some(&2));
            assert!(tasks.take_output_events().is_empty());
        }

        #[test]
        fn stale_revision_and_generation_never_clear_current_pending_or_surface_errors() {
            let (mut tasks, session) = setup();
            tasks.pending.insert("onnx".into(), 2);
            let mut stale_revision = completion(&session, 2);
            stale_revision.revision = stale_revision.revision.wrapping_add(1);
            stale_revision.result = Err("obsolete revision".into());
            tasks.sender.send(stale_revision).unwrap();
            let mut stale_generation = completion(&session, 2);
            stale_generation.generation = stale_generation.generation.wrapping_add(1);
            stale_generation.result = Err("obsolete generation".into());
            tasks.sender.send(stale_generation).unwrap();
            assert!(tasks.next_completion(&session).unwrap().is_none());
            assert_eq!(tasks.pending.get("onnx"), Some(&2));
            assert!(tasks.take_output_events().is_empty());
        }

        #[test]
        fn stopping_and_restarting_cannot_accept_old_work_or_clear_new_work() {
            let (mut tasks, mut session) = setup();
            tasks.pending.insert("onnx".into(), 1);
            let stopped = completion(&session, 1);
            session.stop().unwrap();
            tasks.cancel_pending();
            assert!(!tasks.has_pending());
            tasks.sender.send(stopped).unwrap();
            assert!(tasks.next_completion(&session).unwrap().is_none());
            assert!(tasks.take_output_events().is_empty());
            let graph = graph();
            session.set_graph(&graph).unwrap();
            tasks.sync_graph(&graph, &mut session).unwrap();
            session.play(0).unwrap();
            tasks.pending.insert("onnx".into(), 2);
            tasks.sender.send(completion(&session, 1)).unwrap();
            assert!(tasks.next_completion(&session).unwrap().is_none());
            assert_eq!(tasks.pending.get("onnx"), Some(&2));
            assert!(tasks.take_output_events().is_empty());
        }

        #[test]
        fn graph_sync_replaces_subscriptions_and_discards_pending_events_and_old_revision() {
            let (mut tasks, mut session) = setup();
            let old = completion(&session, 1);
            tasks.pending.insert("onnx".into(), 1);
            tasks.sender.send(completion(&session, 1)).unwrap();
            let accepted = tasks.next_completion(&session).unwrap().unwrap();
            tasks.record_delivery(accepted, ["native-output-onnx".to_owned()]);
            assert_eq!(tasks.output_events.len(), 1);
            tasks.pending.insert("onnx".into(), 2);
            let graph = graph();
            session.set_graph(&graph).unwrap();
            tasks.sync_graph(&graph, &mut session).unwrap();
            assert!(!tasks.has_pending());
            assert!(tasks.take_output_events().is_empty());
            assert_eq!(tasks.event_subscriptions.len(), 1);
            tasks.pending.insert("onnx".into(), 3);
            tasks.sender.send(old).unwrap();
            assert!(tasks.next_completion(&session).unwrap().is_none());
            assert_eq!(tasks.pending.get("onnx"), Some(&3));
            let empty: Graph = serde_json::from_value(serde_json::json!({"nodes":[],"edges":[]})).unwrap();
            session.set_graph(&empty).unwrap();
            tasks.sync_graph(&empty, &mut session).unwrap();
            assert!(tasks.outputs.is_empty());
            assert!(tasks.event_subscriptions.is_empty());
            assert!(!tasks.has_pending());
        }

        #[test]
        fn accepted_image_and_typed_envelope_only_emit_an_event_for_the_owner_subscription() {
            let (mut tasks, mut session) = setup();
            tasks.pending.insert("onnx".into(), 1);
            tasks.sender.send(completion(&session, 1)).unwrap();
            let accepted = tasks.next_completion(&session).unwrap().unwrap();
            assert!(!tasks.has_pending());
            assert_eq!(accepted.node_id, "onnx");
            assert_eq!(accepted.image.rgba, [255, 0, 0, 255]);
            assert_eq!((accepted.image.width, accepted.image.height), (1, 1));
            assert_eq!(accepted.envelope.graph_revision, session.revision());
            assert_eq!(accepted.envelope.node_generation, session.node_generation("onnx").unwrap());
            assert_eq!(accepted.envelope.outputs, vec![(
                OutputKey::new("onnx", "out"),
                OutputPayload::Resource { handle: u64::from(session.node_generation("onnx").unwrap()) },
            )]);
            assert!(tasks.take_output_events().is_empty());
            session.submit_completion(accepted.envelope.clone()).unwrap();
            let delivered = session.drain_deliveries().deliveries;
            tasks.record_delivery(accepted, delivered.into_iter().map(|delivery| delivery.subscription_id));
            let events = tasks.take_output_events();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].node_id, "onnx");
            assert_eq!(events[0].backend, "native");
            assert!(tasks.take_output_events().is_empty());
            tasks.pending.insert("onnx".into(), 2);
            tasks.sender.send(completion(&session, 2)).unwrap();
            let accepted = tasks.next_completion(&session).unwrap().unwrap();
            tasks.record_delivery(accepted, ["another-subscription".to_owned()]);
            assert!(tasks.take_output_events().is_empty());
        }

        #[test]
        fn current_failure_clears_only_its_pending_work_and_reports_the_node() {
            let (mut tasks, session) = setup();
            tasks.pending.insert("onnx".into(), 1);
            let mut failed = completion(&session, 1);
            failed.result = Err("model failed".into());
            tasks.sender.send(failed).unwrap();
            let error = tasks.next_completion(&session).err().unwrap();
            assert_eq!(error, "ONNX onnx inference failed: model failed");
            assert!(!tasks.has_pending());
            assert!(tasks.take_output_events().is_empty());
        }

        #[test]
        fn next_completion_preserves_queued_work_across_acceptance_and_failure() {
            let (mut tasks, mut session) = setup();
            let mut graph = graph();
            let mut other_node = graph.nodes[0].clone();
            other_node.id = "other".into();
            graph.nodes.push(other_node);
            session.set_graph(&graph).unwrap();
            tasks.sync_graph(&graph, &mut session).unwrap();
            let other_completion = |task_id| {
                let mut result = completion(&session, task_id);
                result.node_id = "other".into();
                result.generation = session.node_generation("other").unwrap();
                result
            };
            tasks.pending.insert("onnx".into(), 1);
            tasks.pending.insert("other".into(), 2);
            tasks.sender.send(completion(&session, 1)).unwrap();
            let mut failed = other_completion(2);
            failed.result = Err("other model failed".into());
            tasks.sender.send(failed).unwrap();
            let accepted = tasks.next_completion(&session).unwrap().unwrap();
            assert_eq!(accepted.node_id, "onnx");
            assert!(!tasks.pending.contains_key("onnx"));
            assert_eq!(tasks.pending.get("other"), Some(&2));
            // An upload or submit failure can stop the caller here without consuming other work.
            drop(accepted);
            assert_eq!(tasks.pending.get("other"), Some(&2));
            assert_eq!(
                tasks.next_completion(&session).err().unwrap(),
                "ONNX other inference failed: other model failed",
            );
            assert!(!tasks.has_pending());
            assert!(tasks.next_completion(&session).unwrap().is_none());
            tasks.pending.insert("onnx".into(), 3);
            tasks.pending.insert("other".into(), 4);
            let mut failed = completion(&session, 3);
            failed.result = Err("first model failed".into());
            tasks.sender.send(failed).unwrap();
            tasks.sender.send(other_completion(4)).unwrap();
            assert_eq!(
                tasks.next_completion(&session).err().unwrap(),
                "ONNX onnx inference failed: first model failed",
            );
            assert!(!tasks.pending.contains_key("onnx"));
            assert_eq!(tasks.pending.get("other"), Some(&4));
            let accepted = tasks.next_completion(&session).unwrap().unwrap();
            assert_eq!(accepted.node_id, "other");
            assert!(!tasks.has_pending());
            assert!(tasks.next_completion(&session).unwrap().is_none());
            assert!(tasks.take_output_events().is_empty());
        }

        #[test]
        fn clear_after_disposal_releases_metadata_and_retains_worker_and_channel_ownership() {
            let (mut tasks, mut session) = setup();
            tasks.next_task = 2;
            tasks.pending.insert("onnx".into(), 1);
            tasks.sender.send(completion(&session, 1)).unwrap();
            let accepted = tasks.next_completion(&session).unwrap().unwrap();
            tasks.record_delivery(accepted, ["native-output-onnx".to_owned()]);
            assert_eq!(tasks.output_events.len(), 1);
            assert!(!tasks.outputs.is_empty());
            assert!(!tasks.event_subscriptions.is_empty());
            tasks.pending.insert("onnx".into(), 2);
            let late = completion(&session, 2);
            tasks.workers.push(std::thread::spawn(|| {}));
            session.dispose().unwrap();
            tasks.clear();
            assert!(tasks.resources.is_empty());
            assert!(!tasks.has_pending());
            assert!(tasks.outputs.is_empty());
            assert!(tasks.event_subscriptions.is_empty());
            assert!(tasks.take_output_events().is_empty());
            assert_eq!(tasks.next_task, 2);
            assert_eq!(tasks.workers.len(), 1);
            // Late workers can still send safely, but disposed work can never be accepted.
            tasks.sender.send(late).unwrap();
            assert!(tasks.next_completion(&session).unwrap().is_none());
            assert!(tasks.take_output_events().is_empty());
        }

        #[test]
        fn drop_joins_owned_workers() {
            let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let worker_finished = finished.clone();
            let mut tasks = InferenceTasks::new();
            tasks.workers.push(std::thread::spawn(move || {
                worker_finished.store(true, std::sync::atomic::Ordering::SeqCst);
            }));
            drop(tasks);
            assert!(finished.load(std::sync::atomic::Ordering::SeqCst));
        }
    }
}
