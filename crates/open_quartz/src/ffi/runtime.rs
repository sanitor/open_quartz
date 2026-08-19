use crate::runtime::{
    AsyncCompletionEnvelope, OutputSubscription, PresentationSubscription, ResourceDescriptor,
    Runtime, RuntimeCapabilities, RuntimeFrameInput,
};
use crate::types::Graph;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = Runtime))]
pub struct RuntimeBinding {
    inner: Runtime,
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_class = Runtime))]
impl RuntimeBinding {
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(constructor))]
    pub fn new() -> Self {
        // Dedicated workers cannot own HTMLVideoElement. Browser video arrives
        // as transferred ImageBitmap frames and must compile as texture_2d.
        Self {
            inner: Runtime::new_native(RuntimeCapabilities { data_paths: vec![] }),
        }
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = setGraph))]
    pub fn set_graph_json(&mut self, graph_json: &str) -> Result<u32, String> {
        let graph: Graph = decode_json(graph_json, "graph")?;
        self.inner
            .set_graph(&graph)
            .map_err(|error| error.to_json())
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = setVideoNodes))]
    pub fn set_video_nodes_json(&mut self, node_ids_json: &str) -> Result<(), String> {
        let node_ids: Vec<String> = decode_json(node_ids_json, "video node IDs")?;
        self.inner
            .set_video_nodes(&node_ids)
            .map_err(|error| error.to_json())
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = registerResource))]
    pub fn register_resource_json(
        &mut self,
        descriptor_json: &str,
        handle: u64,
    ) -> Result<(), String> {
        let descriptor: ResourceDescriptor = decode_json(descriptor_json, "resource descriptor")?;
        self.inner
            .register_resource(descriptor, handle)
            .map_err(|error| error.to_json())
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = removeResource))]
    pub fn remove_resource(&mut self, resource_id: &str) -> Result<u64, String> {
        self.inner
            .remove_resource(resource_id)
            .map_err(|error| error.to_json())
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = play))]
    pub fn play(&mut self, now_ns: u64) -> Result<(), String> {
        self.inner.play(now_ns).map_err(|error| error.to_json())
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = advance))]
    pub fn advance_json(&mut self, input_json: &str) -> Result<String, String> {
        let input: RuntimeFrameInput = decode_json(input_json, "frame input")?;
        self.inner
            .advance(&input)
            .and_then(|clock| {
                serde_json::to_string(&clock).map_err(|error| {
                    crate::error::SdkError::new(
                        crate::error::SdkErrorCode::InvalidResource,
                        "Cannot serialize clock state",
                    )
                    .with_details(error.to_string())
                })
            })
            .map_err(|error| error.to_json())
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = subscribeOutput))]
    pub fn subscribe_output_json(&mut self, subscription_json: &str) -> Result<(), String> {
        let subscription: OutputSubscription =
            decode_json(subscription_json, "output subscription")?;
        self.inner
            .subscribe_output(subscription)
            .map_err(|error| error.to_json())
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = updateOutputSubscription))]
    pub fn update_output_subscription_json(
        &mut self,
        subscription_json: &str,
    ) -> Result<(), String> {
        let subscription: OutputSubscription =
            decode_json(subscription_json, "output subscription")?;
        self.inner
            .update_output_subscription(subscription)
            .map_err(|error| error.to_json())
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = unsubscribeOutput))]
    pub fn unsubscribe_output(&mut self, subscription_id: &str) -> Result<(), String> {
        self.inner
            .unsubscribe_output(subscription_id)
            .map(|_| ())
            .map_err(|error| error.to_json())
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = subscribePresentation))]
    pub fn subscribe_presentation_json(&mut self, subscription_json: &str) -> Result<(), String> {
        let subscription: PresentationSubscription =
            decode_json(subscription_json, "presentation subscription")?;
        self.inner
            .subscribe_presentation(subscription)
            .map_err(|error| error.to_json())
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = updatePresentation))]
    pub fn update_presentation_json(&mut self, subscription_json: &str) -> Result<(), String> {
        let subscription: PresentationSubscription =
            decode_json(subscription_json, "presentation subscription")?;
        self.inner
            .update_presentation_subscription(subscription)
            .map_err(|error| error.to_json())
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = unsubscribePresentation))]
    pub fn unsubscribe_presentation(&mut self, subscription_id: &str) -> bool {
        self.inner.unsubscribe_presentation(subscription_id)
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = submitCompletion))]
    pub fn submit_completion_json(&mut self, completion_json: &str) -> Result<(), String> {
        let completion: AsyncCompletionEnvelope = decode_json(completion_json, "async completion")?;
        self.inner
            .submit_completion(completion)
            .map_err(|error| error.to_json())
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = publishOutput))]
    pub fn publish_output_json(&mut self, state_json: &str) -> Result<(), String> {
        let state = decode_json(state_json, "output state")?;
        self.inner
            .publish_output(state)
            .map_err(|error| error.to_json())
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = executionPlan))]
    pub fn execution_plan_json(&self) -> Result<String, String> {
        let plan = self
            .inner
            .execution_plan()
            .ok_or_else(|| "Runtime must receive a graph before reading its execution plan")?;
        serde_json::to_string(plan)
            .map_err(|error| format!("Cannot serialize runtime execution plan: {error}"))
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = drainWork))]
    pub fn drain_work(&mut self) -> Result<String, String> {
        self.inner.drain_work().map_err(|error| error.to_json())
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = drainDeliveries))]
    pub fn drain_deliveries_json(&mut self) -> String {
        serde_json::to_string(&self.inner.drain_deliveries())
            .expect("Output delivery batch is always serializable")
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = drainEvents))]
    pub fn drain_events_json(&mut self) -> String {
        serde_json::to_string(&self.inner.drain_events())
            .expect("Runtime events are always serializable")
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = capabilities))]
    pub fn capabilities_json(&self) -> String {
        serde_json::to_string(self.inner.capabilities())
            .expect("Runtime capabilities are always serializable")
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = pause))]
    pub fn pause(&mut self, now_ns: u64) -> Result<(), String> {
        self.inner.pause(now_ns).map_err(|error| error.to_json())
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = resume))]
    pub fn resume(&mut self, now_ns: u64) -> Result<(), String> {
        self.inner.resume(now_ns).map_err(|error| error.to_json())
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = stop))]
    pub fn stop(&mut self) -> Result<(), String> {
        self.inner.stop().map_err(|error| error.to_json())
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = dispose))]
    pub fn dispose(&mut self) -> Result<(), String> {
        self.inner.dispose().map_err(|error| error.to_json())
    }
}

impl Default for RuntimeBinding {
    fn default() -> Self {
        Self::new()
    }
}

fn decode_json<T: serde::de::DeserializeOwned>(json: &str, name: &str) -> Result<T, String> {
    serde_json::from_str(json).map_err(|error| format!("Cannot decode {name}: {error}"))
}
