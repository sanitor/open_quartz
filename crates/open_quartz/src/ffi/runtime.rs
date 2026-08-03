use crate::runtime::{
    OutputState, OutputSubscription, Runtime, RuntimeCapabilities, RuntimeFrameInput,
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
        Self {
            inner: Runtime::new(RuntimeCapabilities { data_paths: vec![] }),
        }
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = setGraph))]
    pub fn set_graph_json(&mut self, graph_json: &str) -> Result<u32, String> {
        let graph: Graph = serde_json::from_str(graph_json)
            .map_err(|error| format!("Cannot decode graph: {error}"))?;
        self.inner
            .set_graph(&graph)
            .map_err(|error| error.to_json())
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = advance))]
    pub fn advance_json(&mut self, input_json: &str) -> Result<(), String> {
        let input: RuntimeFrameInput = serde_json::from_str(input_json)
            .map_err(|error| format!("Cannot decode frame input: {error}"))?;
        self.inner.advance(&input).map_err(|error| error.to_json())
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = subscribeOutput))]
    pub fn subscribe_output_json(&mut self, subscription_json: &str) -> Result<(), String> {
        let subscription: OutputSubscription = serde_json::from_str(subscription_json)
            .map_err(|error| format!("Cannot decode output subscription: {error}"))?;
        self.inner
            .subscribe_output(subscription)
            .map_err(|error| error.to_json())
    }

    #[cfg_attr(
        target_arch = "wasm32",
        wasm_bindgen(js_name = updateOutputSubscription)
    )]
    pub fn update_output_subscription_json(
        &mut self,
        subscription_json: &str,
    ) -> Result<(), String> {
        let subscription: OutputSubscription = serde_json::from_str(subscription_json)
            .map_err(|error| format!("Cannot decode output subscription: {error}"))?;
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

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = publishOutput))]
    pub fn publish_output_json(&mut self, state_json: &str) -> Result<(), String> {
        let state: OutputState = serde_json::from_str(state_json)
            .map_err(|error| format!("Cannot decode output state: {error}"))?;
        self.inner
            .publish_output(state)
            .map_err(|error| error.to_json())
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = drainDeliveries))]
    pub fn drain_deliveries_json(&mut self) -> String {
        serde_json::to_string(&self.inner.drain_deliveries())
            .expect("Output delivery batch is always serializable")
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = pause))]
    pub fn pause(&mut self) -> Result<(), String> {
        self.inner.pause().map_err(|error| error.to_json())
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = resume))]
    pub fn resume(&mut self) -> Result<(), String> {
        self.inner.resume().map_err(|error| error.to_json())
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = stop))]
    pub fn stop(&mut self) -> Result<(), String> {
        self.inner.stop().map_err(|error| error.to_json())
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = dispose))]
    pub fn dispose(&mut self) {
        self.inner.dispose();
    }
}

impl Default for RuntimeBinding {
    fn default() -> Self {
        Self::new()
    }
}
