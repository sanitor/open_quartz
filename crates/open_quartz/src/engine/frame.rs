use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameInputs {
    pub time: f64,
    pub delta: f64,
    pub frame: u64,
    #[serde(default)]
    pub date: [f32; 4],
    #[serde(default)]
    pub mouse: [f32; 4],
    #[serde(default = "default_resolution")]
    pub resolution: [f32; 3],
    #[serde(default)]
    pub video_nodes: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionCommand {
    pub node_id: String,
    pub kind: String,
    pub texture_inputs: BTreeMap<String, String>,
    pub uniforms: BTreeMap<String, Vec<f32>>,
    pub target_width: Option<u32>,
    pub target_height: Option<u32>,
    pub feedback_read_index: Option<u8>,
    pub feedback_write_index: Option<u8>,
    pub clear_feedback: bool,
    pub scalar_output: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameResult {
    pub frame: u64,
    pub commands: Vec<ExecutionCommand>,
    pub dirty_nodes: Vec<String>,
}

fn default_resolution() -> [f32; 3] {
    [512.0, 512.0, 1.0]
}
