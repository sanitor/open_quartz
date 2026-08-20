#![cfg(target_arch = "wasm32")]

use serde::Serialize;
use wasm_bindgen::prelude::*;

use open_quartz_execution::engine::ExecutionCommand;
use open_quartz_execution::host::PlayerHost;
use open_quartz_execution::runtime::{
    AsyncCompletionEnvelope, ContentStamp, DataPathMode, FrameStamp, OutputKey, OutputPayload,
    OutputState, OutputSubscription, RuntimeCapabilities, RuntimeFrameInput,
};
use open_quartz_schema::Graph;
use open_quartz_execution::wasm_environment::{BrowserFrame, BrowserGpuEnvironment};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BrowserInferenceTask {
    graph_revision: u32,
    node_generation: u32,
    input_stamp: FrameStamp,
    #[serde(flatten)]
    command: ExecutionCommand,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BrowserFrameResult {
    clock: open_quartz_execution::runtime::ClockState,
    inference_tasks: Vec<BrowserInferenceTask>,
}

#[wasm_bindgen(js_name = BrowserPlayer)]
pub struct BrowserPlayerBinding {
    host: PlayerHost,
    gpu: BrowserGpuEnvironment,
    output_node_id: Option<String>,
}

#[wasm_bindgen(js_class = BrowserPlayer)]
impl BrowserPlayerBinding {
    #[wasm_bindgen(js_name = create)]
    pub async fn create(canvas: web_sys::OffscreenCanvas) -> Result<BrowserPlayerBinding, JsValue> {
        let gpu = BrowserGpuEnvironment::from_offscreen_canvas(canvas)
            .await
            .map_err(|error| JsValue::from_str(&error.to_json()))?;
        Ok(Self {
            host: PlayerHost::new_native(RuntimeCapabilities {
                data_paths: vec![DataPathMode::ExternalFrameNoCpuReadback],
            }),
            gpu,
            output_node_id: None,
        })
    }

    #[wasm_bindgen(js_name = setGraph)]
    pub fn set_graph(&mut self, graph_json: &str) -> Result<u32, JsValue> {
        let graph: Graph = serde_json::from_str(graph_json)
            .map_err(|error| JsValue::from_str(&format!("Cannot decode graph: {error}")))?;
        let revision = self
            .host
            .set_graph(&graph)
            .map_err(|error| JsValue::from_str(&error.to_json()))?;
        let plan = self
            .host
            .execution_plan()
            .ok_or_else(|| JsValue::from_str("Graph did not produce an execution plan"))?;
        self.output_node_id = plan.output_nodes.first().cloned();
        self.gpu
            .executor
            .sync_plan(plan)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        Ok(revision)
    }

    pub fn play(&mut self, now_ns: u64) -> Result<(), JsValue> {
        self.host
            .play(now_ns)
            .map_err(|error| JsValue::from_str(&error.to_json()))
    }

    pub fn pause(&mut self, now_ns: u64) -> Result<(), JsValue> {
        self.host
            .pause(now_ns)
            .map_err(|error| JsValue::from_str(&error.to_json()))
    }

    pub fn resume(&mut self, now_ns: u64) -> Result<(), JsValue> {
        self.host
            .resume(now_ns)
            .map_err(|error| JsValue::from_str(&error.to_json()))
    }

    pub fn stop(&mut self) -> Result<(), JsValue> {
        self.host
            .stop()
            .map_err(|error| JsValue::from_str(&error.to_json()))
    }

    #[wasm_bindgen(js_name = uploadFrame)]
    pub fn upload_frame(
        &mut self,
        node_id: &str,
        bitmap: web_sys::ImageBitmap,
        timestamp_ns: u64,
    ) -> Result<(), JsValue> {
        let frame = BrowserFrame::new(bitmap, timestamp_ns);
        self.gpu
            .upload_frame(node_id, &frame)
            .map_err(|error| JsValue::from_str(&error.to_json()))?;
        self.host
            .mark_dirty(node_id)
            .map_err(|error| JsValue::from_str(&error.to_json()))
    }

    pub fn frame(&mut self, input_json: &str) -> Result<String, JsValue> {
        let input: RuntimeFrameInput = serde_json::from_str(input_json)
            .map_err(|error| JsValue::from_str(&format!("Cannot decode frame input: {error}")))?;
        let clock = self
            .host
            .advance(&input)
            .map_err(|error| JsValue::from_str(&error.to_json()))?;
        let stamp = FrameStamp {
            epoch: clock.epoch,
            frame: clock.frame,
            timeline_ns: clock.timeline_ns,
            deadline_ns: clock.next_deadline_ns,
        };
        let commands = self.host.drain_commands();
        let mut gpu_commands = Vec::new();
        let mut executed_commands = Vec::new();
        let mut inference_tasks = Vec::new();
        for command in commands {
            if command.kind == "onnx" {
                if !gpu_commands.is_empty() {
                    self.host
                        .execute_gpu(&mut self.gpu.executor, &gpu_commands)
                        .map_err(|error| JsValue::from_str(&error.to_json()))?;
                    executed_commands.extend(gpu_commands.drain(..));
                }
                let node_generation = self
                    .host
                    .node_generation(&command.node_id)
                    .map_err(|error| JsValue::from_str(&error.to_json()))?;
                inference_tasks.push(BrowserInferenceTask {
                    graph_revision: self.host.revision(),
                    node_generation,
                    input_stamp: stamp.clone(),
                    command,
                });
                continue;
            }
            let inputs_ready = command
                .texture_inputs
                .values()
                .all(|source| self.gpu.executor.output_texture(source).is_some());
            if inputs_ready {
                gpu_commands.push(command);
            }
        }
        if !gpu_commands.is_empty() {
            self.host
                .execute_gpu(&mut self.gpu.executor, &gpu_commands)
                .map_err(|error| JsValue::from_str(&error.to_json()))?;
            executed_commands.extend(gpu_commands);
        }

        for command in &executed_commands {
            let Some(port_id) = command.output_port_id.as_deref() else {
                continue;
            };
            if self.gpu.executor.output_texture(&command.node_id).is_none() {
                continue;
            }
            let generation = self
                .host
                .node_generation(&command.node_id)
                .map_err(|error| JsValue::from_str(&error.to_json()))?;
            self.host
                .publish_output(OutputState {
                    output: OutputKey::new(&command.node_id, port_id),
                    graph_revision: self.host.revision(),
                    output_generation: u64::from(generation),
                    evaluation_stamp: stamp.clone(),
                    content_stamp: ContentStamp {
                        epoch: stamp.epoch,
                        timeline_ns: stamp.timeline_ns,
                        media_pts_ns: None,
                    },
                    payload: OutputPayload::Resource {
                        handle: u64::from(generation),
                    },
                })
                .map_err(|error| JsValue::from_str(&error.to_json()))?;
        }
        if let Some(output_node_id) = self.output_node_id.as_deref() {
            if let Some(output) = self.gpu.executor.output_handle(output_node_id) {
                self.gpu
                    .present(&output)
                    .map_err(|error| JsValue::from_str(&error.to_json()))?;
            }
        }
        serde_json::to_string(&BrowserFrameResult {
            clock,
            inference_tasks,
        })
        .map_err(|error| JsValue::from_str(&format!("Cannot encode frame result: {error}")))
    }

    #[wasm_bindgen(js_name = uploadRgba)]
    pub fn upload_rgba(
        &mut self,
        node_id: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Result<(), JsValue> {
        self.gpu
            .executor
            .upload_rgba(node_id, rgba, width, height)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;

        self.host
            .mark_dirty(node_id)
            .map_err(|error| JsValue::from_str(&error.to_json()))
    }

    #[wasm_bindgen(js_name = outputInfo)]
    pub fn output_info(&self, node_id: &str) -> Result<String, JsValue> {
        let output = self
            .gpu
            .executor
            .output_handle(node_id)
            .ok_or_else(|| JsValue::from_str("GPU output is not available"))?;
        Ok(serde_json::json!({ "width": output.width, "height": output.height }).to_string())
    }

    #[wasm_bindgen(js_name = readOutputRgba)]
    pub async fn read_output_rgba(&self, node_id: &str) -> Result<Vec<u8>, JsValue> {
        self.gpu
            .executor
            .read_output_rgba(node_id)
            .await
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }

    #[wasm_bindgen(js_name = subscribeOutput)]
    pub fn subscribe_output(&mut self, subscription_json: &str) -> Result<(), JsValue> {
        let subscription: OutputSubscription = serde_json::from_str(subscription_json)
            .map_err(|error| JsValue::from_str(&format!("Cannot decode subscription: {error}")))?;
        self.host
            .subscribe_output(subscription)
            .map_err(|error| JsValue::from_str(&error.to_json()))
    }

    #[wasm_bindgen(js_name = unsubscribeOutput)]
    pub fn unsubscribe_output(&mut self, subscription_id: &str) -> Result<(), JsValue> {
        self.host
            .unsubscribe_output(subscription_id)
            .map(|_| ())
            .map_err(|error| JsValue::from_str(&error.to_json()))
    }

    #[wasm_bindgen(js_name = submitCompletion)]
    pub fn submit_completion(&mut self, completion_json: &str) -> Result<(), JsValue> {
        let completion: AsyncCompletionEnvelope = serde_json::from_str(completion_json)
            .map_err(|error| JsValue::from_str(&format!("Cannot decode completion: {error}")))?;
        self.host
            .submit_completion(completion)
            .map_err(|error| JsValue::from_str(&error.to_json()))
    }

    #[wasm_bindgen(js_name = drainDeliveries)]
    pub fn drain_deliveries(&mut self) -> String {
        serde_json::to_string(&self.host.drain_deliveries())
            .expect("Output deliveries are serializable")
    }

    pub fn close(&mut self) -> Result<(), JsValue> {
        self.host
            .dispose()
            .map_err(|error| JsValue::from_str(&error.to_json()))
    }
}
