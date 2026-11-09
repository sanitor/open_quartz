#[cfg(test)]
#[path = "inference_runtime_tests.rs"]
mod tests;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::thread::{self, JoinHandle};

use open_quartz::resources::plan::{
    plan_host_resource_intents, HostGraphSnapshot, HostResourceIntent, HostResourceIntentPlan,
    HostResourceIntentRequest, InferenceDownloadIntent,
};
use open_quartz::inference::{ExecutionPolicy, InferenceConfig, PreparedInferenceModel};
use open_quartz::resources::models::ModelStore;
use open_quartz::player::InferenceOutputEvent;
use open_quartz::{Graph, NativePlayer, InferenceTask};

use crate::render_bridge::Command;

#[derive(Clone, PartialEq)]
struct ModelSpec {
    key: String,
    model_id: String,
    task: InferenceTask,
    path: Option<String>,
    target_size: u32,
    score: f64,
    iou: f64,
    policy: ExecutionPolicy,
    download: Option<InferenceDownloadIntent>,
}

#[derive(Clone, Default)]
pub struct LiveInferenceState {
    pub status: String,
    pub backend: String,
    pub execution: String,
    pub artifact: String,
    pub inputs: String,
    pub outputs: String,
    pub interface: String,
    pub result: String,
    pub loading: bool,
    pub attached: bool,
    pub failed: bool,
}

pub enum ModelMessage {
    Progress {
        node_id: String,
        ticket: u64,
        received: u64,
        total: u64,
    },
    Ready {
        node_id: String,
        ticket: u64,
        result: Result<PreparedInferenceModel, String>,
    },
}

impl ModelMessage {
    pub fn node_id(&self) -> &str {
        match self {
            Self::Progress { node_id, .. } | Self::Ready { node_id, .. } => node_id,
        }
    }
}

#[derive(Default)]
pub struct InferenceRuntime {
    specs: BTreeMap<String, ModelSpec>,
    pub states: BTreeMap<String, LiveInferenceState>,
    requests: BTreeMap<String, (u64, Arc<AtomicBool>)>,
    next_ticket: u64,
    workers: Vec<(String, u64, JoinHandle<()>)>,
}

impl InferenceRuntime {
    pub fn clear(&mut self, host: &mut NativePlayer) {
        let ids: std::collections::BTreeSet<_> = self
            .states
            .keys()
            .chain(self.specs.keys())
            .cloned()
            .collect();
        for id in ids {
            self.cancel(&id);
            host.unload_inference(&id);
        }
        self.states.clear();
        self.specs.clear();
    }
    pub fn sync(
        &mut self,
        intents: &[HostResourceIntent],
        host: &mut NativePlayer,
        reset: bool,
    ) {
        let specs = model_specs(intents);
        let removed: Vec<_> = self
            .states
            .keys()
            .filter(|id| reset || self.specs.get(*id) != specs.get(*id))
            .cloned()
            .collect();
        for id in removed {
            self.cancel(&id);
            host.unload_inference(&id);
            self.states.remove(&id);
        }
        for (id, state) in &mut self.states {
            host.remove_texture(id);
            state.result.clear();
            if state.attached {
                state.status = "Loaded · waiting for inference output".into();
            }
        }
        self.specs = specs;
    }

    pub fn load(
        &mut self,
        node_id: String,
        path_override: String,
        sender: mpsc::Sender<Command>,
    ) -> Result<(), String> {
        let Some(spec) = self.specs.get(&node_id).cloned() else {
            let error = "No model ID is configured for this Inference node".to_owned();
            self.fail(&node_id, error.clone());
            return Err(error);
        };
        self.cancel(&node_id);
        self.next_ticket += 1;
        let ticket = self.next_ticket;
        let cancelled = Arc::new(AtomicBool::new(false));
        self.requests
            .insert(node_id.clone(), (ticket, cancelled.clone()));
        self.states.insert(
            node_id.clone(),
            LiveInferenceState {
                status: "Loading model…".into(),
                loading: true,
                ..Default::default()
            },
        );
        let worker_id = node_id.clone();
        let worker = thread::Builder::new()
            .name("open-quartz-model".into())
            .spawn(move || {
                let result = (|| {
                    check_cancelled(&cancelled)?;
                    let config = InferenceConfig {
                        model_id: spec.model_id.clone(),
                        task: spec.task,
                        target_size: spec.target_size,
                        score_threshold: spec.score as f32,
                        iou_threshold: spec.iou as f32,
                        policy: spec.policy,
                    };
                    config.policy.validate().map_err(|error| error.to_string())?;
                    if !path_override.trim().is_empty() {
                        PreparedInferenceModel::open_with_policy(&PathBuf::from(path_override.trim()), config)
                    } else if let Some(path) = &spec.path {
                        PreparedInferenceModel::open_with_policy(&PathBuf::from(path), config)
                    } else {
                        let base = directories::BaseDirs::new()
                            .ok_or("Cannot resolve application data directory")?;
                        let store =
                            ModelStore::new(base.data_dir().join("com.openquartz.app/models"));
                        if let Some(download) = &spec.download {
                            let mut last_progress = 0;
                            store.download(
                                &spec.model_id,
                                &download.url,
                                download.expected_size,
                                &download.sha256,
                                |received, total| {
                                    check_cancelled(&cancelled)?;
                                    if received == 0
                                        || (total > 0 && received == total)
                                        || received.saturating_sub(last_progress) >= 102_400
                                    {
                                        last_progress = received;
                                        sender
                                            .send(Command::Model(ModelMessage::Progress {
                                                node_id: worker_id.clone(),
                                                ticket,
                                                received,
                                                total,
                                            }))
                                            .map_err(|_| {
                                                "Native render thread closed".to_owned()
                                            })?;
                                    }
                                    Ok(())
                                },
                            )?;
                        }
                        check_cancelled(&cancelled)?;
                        store.open_cached(&spec.model_id, config)
                    }
                })();
                let _ = sender.send(Command::Model(ModelMessage::Ready {
                    node_id: worker_id,
                    ticket,
                    result,
                }));
            })
            .map_err(|error| format!("Cannot start model worker: {error}"));
        match worker {
            Ok(worker) => {
                self.workers.push((node_id, ticket, worker));
                Ok(())
            }
            Err(error) => {
                self.fail(&node_id, error.clone());
                Err(error)
            }
        }
    }

    pub fn is_loading(&self) -> bool {
        !self.requests.is_empty()
    }

    pub fn reap_workers(&mut self) -> bool {
        let mut changed = false;
        let mut index = 0;
        while index < self.workers.len() {
            if !self.workers[index].2.is_finished() {
                index += 1;
                continue;
            }
            let (id, ticket, worker) = self.workers.swap_remove(index);
            if let Err(error) = worker.join() {
                if self.requests.get(&id).map(|request| request.0) == Some(ticket) {
                    let message = error
                        .downcast_ref::<String>()
                        .map(String::as_str)
                        .or_else(|| error.downcast_ref::<&str>().copied())
                        .unwrap_or("unknown native worker panic");
                    self.fail(&id, format!("Native model worker failed: {message}"));
                    changed = true;
                }
            }
        }
        changed
    }

    pub fn receive(&mut self, message: ModelMessage, host: &mut NativePlayer) -> bool {
        let (node_id, ticket) = match &message {
            ModelMessage::Progress {
                node_id, ticket, ..
            }
            | ModelMessage::Ready {
                node_id, ticket, ..
            } => (node_id, ticket),
        };
        if self.requests.get(node_id).map(|request| request.0) != Some(*ticket) {
            return false;
        }
        match message {
            ModelMessage::Progress {
                node_id,
                received,
                total,
                ..
            } => {
                if let Some(state) = self.states.get_mut(&node_id) {
                    state.status = if total > 0 {
                        format!("Downloading · {received} / {total} bytes")
                    } else {
                        format!("Downloading · {received} bytes (total unknown)")
                    };
                }
            }
            ModelMessage::Ready {
                node_id, result, ..
            } => {
                self.requests.remove(&node_id);
                let prepared = result.and_then(|prepared| {
                    let contract = prepared.task_contract()?;
                    let interface = serde_json::to_string_pretty(&serde_json::json!({
                        "model": prepared.info(), "taskContract": contract,
                    })).map_err(|error| format!("Cannot describe model interface: {error}"))?;
                    prepared.attach(host, node_id.clone()).map(|info| (info, interface))
                });
                match prepared {
                    Ok((info, interface)) => {
                        self.states.insert(
                            node_id,
                            LiveInferenceState {
                                status: "Loaded · waiting for inference output".into(),
                                backend: info.backend,
                                execution: format!(
                                    "Configured {:?}; provider placement {:?}; resident buffer transfer path {:?}; per-frame host pixel/tensor bytes: 0",
                                    info.plan.configured_backend,
                                    info.plan.provider_placement,
                                    info.plan.transfer_path,
                                ),
                                artifact: format!("{} · {} bytes · SHA-256 {}", info.artifact.format, info.artifact.byte_length, info.artifact.sha256),
                                inputs: info.inputs.iter().map(|tensor| format!("{} · {:?} · {:?}", tensor.name, tensor.element_type, tensor.dimensions)).collect::<Vec<_>>().join("\n"),
                                outputs: info.outputs.iter().map(|tensor| format!("{} · {:?} · {:?}", tensor.name, tensor.element_type, tensor.dimensions)).collect::<Vec<_>>().join("\n"),
                                interface,
                                attached: true,
                                ..Default::default()
                            },
                        );
                        return true;
                    }
                    Err(error) => self.fail(&node_id, error),
                }
            }
        }
        false
    }

    pub fn unload(&mut self, node_id: &str, host: &mut NativePlayer) {
        self.cancel(node_id);
        host.unload_inference(node_id);
        self.states.insert(
            node_id.into(),
            LiveInferenceState {
                status: "Unloaded · no session attached".into(),
                ..Default::default()
            },
        );
    }

    pub fn output(&mut self, event: InferenceOutputEvent) {
        let Some(state) = self
            .states
            .get_mut(&event.node_id)
            .filter(|state| state.attached)
        else {
            return;
        };
        state.status = format!("Inference output · {}×{}", event.width, event.height);
        state.failed = false;
        state.backend = event.backend;
        state.result = "Resident GPU texture output · no host pixels or tensors".into();
    }

    pub fn render_failed(&mut self, error: &str) {
        let failed_node = serde_json::from_str::<open_quartz::SdkError>(error)
            .ok()
            .and_then(|error| error.node_id);
        for (node_id, state) in self.states.iter_mut().filter(|(_, state)| state.attached) {
            state.status = format!("Runtime stopped: {error}");
            state.failed |= failed_node.as_deref() == Some(node_id.as_str());
            state.result.clear();
        }
    }

    fn fail(&mut self, node_id: &str, error: String) {
        self.cancel(node_id);
        self.states.insert(
            node_id.into(),
            LiveInferenceState {
                status: format!("Model load failed: {error}"),
                failed: true,
                ..Default::default()
            },
        );
    }

    fn cancel(&mut self, node_id: &str) {
        if let Some((_, cancelled)) = self.requests.remove(node_id) {
            cancelled.store(true, Ordering::Release);
        }
    }
}

impl Drop for InferenceRuntime {
    fn drop(&mut self) {
        for (_, cancelled) in self.requests.values() {
            cancelled.store(true, Ordering::Release);
        }
        // Session construction cannot be interrupted. Workers own their resources and
        // late sends fail after shutdown; never block the UI waiting for a network read.
    }
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::Acquire) {
        Err("Model load cancelled".into())
    } else {
        Ok(())
    }
}

pub(crate) fn host_resource_plan(graph: &Graph) -> Result<HostResourceIntentPlan, String> {
    let graph: HostGraphSnapshot =
        serde_json::from_value(serde_json::to_value(graph).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    plan_host_resource_intents(HostResourceIntentRequest {
        previous_graph: None,
        graph,
    })
}

fn model_specs(intents: &[HostResourceIntent]) -> BTreeMap<String, ModelSpec> {
    intents
        .iter()
        .filter_map(|intent| match intent {
            HostResourceIntent::LoadInference {
                node_id,
                key,
                model_id,
                task,
                model_path,
                target_size,
                score_threshold,
                iou_threshold,
                policy,
                download,
            } => Some((
                node_id.clone(),
                ModelSpec {
                    key: key.clone(),
                    model_id: model_id.clone(),
                    task: *task,
                    path: model_path.clone(),
                    target_size: *target_size,
                    score: *score_threshold,
                    iou: *iou_threshold,
                    policy: policy.clone(),
                    download: download.clone(),
                },
            )),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod error_tests {
    use super::{LiveInferenceState, InferenceRuntime};
    use open_quartz::{SdkError, SdkErrorCode};

    #[test]
    fn render_error_only_marks_the_attributed_inference_node_failed() {
        let mut runtime = InferenceRuntime::default();
        for id in ["model_a", "model_b"] {
            runtime.states.insert(id.into(), LiveInferenceState {
                attached: true,
                ..Default::default()
            });
        }
        runtime.render_failed(&SdkError::new(SdkErrorCode::InvalidResource, "Bad shader")
            .for_node("shader_1").to_json());
        assert!(runtime.states.values().all(|state| !state.failed));
        runtime.render_failed(&SdkError::new(SdkErrorCode::InvalidResource, "Inference failed")
            .for_node("model_a").to_json());
        assert!(runtime.states["model_a"].failed);
        assert!(!runtime.states["model_b"].failed);
    }
}
