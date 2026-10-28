#[cfg(test)]
#[path = "onnx_runtime_tests.rs"]
mod tests;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::thread::{self, JoinHandle};

use open_quartz_host_api::{
    plan_host_resource_intents, HostGraphSnapshot, HostResourceIntent, HostResourceIntentRequest,
    HostResourceTarget, OnnxDownloadIntent,
};
use open_quartz_native_host::{
    models::{ModelStore, PreparedOnnxModel},
    NativeHost, NativeOnnxConfig, NativeOutputEvent,
};
use open_quartz_sdk::{Graph, OnnxTask};

use crate::render_bridge::Command;

#[derive(Clone, PartialEq)]
struct ModelSpec {
    key: String,
    model_id: String,
    task: OnnxTask,
    path: Option<String>,
    target_size: u32,
    score: f64,
    iou: f64,
    download: Option<OnnxDownloadIntent>,
}

#[derive(Clone, Default)]
pub struct LiveOnnxState {
    pub status: String,
    pub backend: String,
    pub inputs: String,
    pub outputs: String,
    pub result: String,
    pub loading: bool,
    pub attached: bool,
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
        result: Result<PreparedOnnxModel, String>,
    },
}

#[derive(Default)]
pub struct OnnxRuntime {
    specs: BTreeMap<String, ModelSpec>,
    pub states: BTreeMap<String, LiveOnnxState>,
    requests: BTreeMap<String, (u64, Arc<AtomicBool>)>,
    next_ticket: u64,
    workers: Vec<(String, u64, JoinHandle<()>)>,
}

impl OnnxRuntime {
    pub fn clear(&mut self, host: &mut NativeHost) {
        let ids: std::collections::BTreeSet<_> = self
            .states
            .keys()
            .chain(self.specs.keys())
            .cloned()
            .collect();
        for id in ids {
            self.cancel(&id);
            host.unload_onnx(&id);
        }
        self.states.clear();
        self.specs.clear();
    }
    pub fn sync(
        &mut self,
        graph: &Graph,
        host: &mut NativeHost,
        reset: bool,
    ) -> Result<(), String> {
        let specs = model_specs(graph)?;
        let removed: Vec<_> = self
            .states
            .keys()
            .filter(|id| reset || self.specs.get(*id) != specs.get(*id))
            .cloned()
            .collect();
        for id in removed {
            self.cancel(&id);
            host.unload_onnx(&id);
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
        Ok(())
    }

    pub fn load(
        &mut self,
        node_id: String,
        path_override: String,
        sender: mpsc::Sender<Command>,
    ) -> Result<(), String> {
        let Some(spec) = self.specs.get(&node_id).cloned() else {
            let error = "No model ID is configured for this ONNX node".to_owned();
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
            LiveOnnxState {
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
                    require_ort_library()?;
                    let config = NativeOnnxConfig {
                        model_id: spec.model_id.clone(),
                        task: spec.task,
                        target_size: spec.target_size,
                        score_threshold: spec.score as f32,
                        iou_threshold: spec.iou as f32,
                    };
                    if !path_override.trim().is_empty() {
                        PreparedOnnxModel::open(&PathBuf::from(path_override.trim()), config, true)
                    } else if let Some(path) = &spec.path {
                        PreparedOnnxModel::open(&PathBuf::from(path), config, true)
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
                        store.open_cached(&spec.model_id, config, true)
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

    pub fn receive(&mut self, message: ModelMessage, host: &mut NativeHost) -> bool {
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
                match result.and_then(|prepared| prepared.attach(host, node_id.clone())) {
                    Ok(info) => {
                        self.states.insert(
                            node_id,
                            LiveOnnxState {
                                status: "Loaded · waiting for inference output".into(),
                                backend: info.backend,
                                inputs: info.input_names.join(", "),
                                outputs: info.output_names.join(", "),
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

    pub fn unload(&mut self, node_id: &str, host: &mut NativeHost) {
        self.cancel(node_id);
        host.unload_onnx(node_id);
        self.states.insert(
            node_id.into(),
            LiveOnnxState {
                status: "Unloaded · no session attached".into(),
                ..Default::default()
            },
        );
    }

    pub fn output(&mut self, event: NativeOutputEvent) {
        let Some(state) = self
            .states
            .get_mut(&event.node_id)
            .filter(|state| state.attached)
        else {
            return;
        };
        state.status = format!("Inference output · {}×{}", event.width, event.height);
        state.backend = event.backend;
        state.result = event
            .data
            .map(|data| serde_json::to_string_pretty(&data).expect("JSON value serializes"))
            .unwrap_or_else(|| "Texture output · no structured result for this task".into());
    }

    pub fn render_failed(&mut self, error: &str) {
        for state in self.states.values_mut().filter(|state| state.attached) {
            state.status = format!("Runtime stopped: {error}");
            state.result.clear();
        }
    }

    fn fail(&mut self, node_id: &str, error: String) {
        self.cancel(node_id);
        self.states.insert(
            node_id.into(),
            LiveOnnxState {
                status: format!("Model load failed: {error}"),
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

impl Drop for OnnxRuntime {
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

fn require_ort_library() -> Result<(), String> {
    let path = std::env::var_os("ORT_DYLIB_PATH").map(PathBuf::from)
        .ok_or("ONNX Runtime is not configured. Set ORT_DYLIB_PATH to the native runtime library before launching Slint.")?;
    if !path.is_file() {
        return Err(format!(
            "ONNX Runtime library does not exist: {}",
            path.display()
        ));
    }
    Ok(())
}

fn model_specs(graph: &Graph) -> Result<BTreeMap<String, ModelSpec>, String> {
    let graph: HostGraphSnapshot =
        serde_json::from_value(serde_json::to_value(graph).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let plan = plan_host_resource_intents(HostResourceIntentRequest {
        host: HostResourceTarget::Native,
        previous_graph: None,
        graph,
    })?;
    Ok(plan
        .intents
        .into_iter()
        .filter_map(|intent| match intent {
            HostResourceIntent::LoadOnnx {
                node_id,
                key,
                model_id,
                task,
                model_path,
                target_size,
                score_threshold,
                iou_threshold,
                download,
            } => Some((
                node_id,
                ModelSpec {
                    key,
                    model_id,
                    task,
                    path: model_path,
                    target_size,
                    score: score_threshold,
                    iou: iou_threshold,
                    download,
                },
            )),
            _ => None,
        })
        .collect())
}
