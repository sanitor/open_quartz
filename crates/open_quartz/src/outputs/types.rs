use serde::{Deserialize, Serialize};


#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameStamp {
    pub epoch: u64,
    pub frame: u64,
    pub timeline_ns: u64,
    pub deadline_ns: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentStamp {
    pub epoch: u64,
    pub timeline_ns: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_pts_ns: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputKey {
    pub node_id: String,
    pub port_id: String,
}

impl OutputKey {
    pub fn new(node_id: impl Into<String>, port_id: impl Into<String>) -> Self {
        Self {
            node_id: node_id.into(),
            port_id: port_id.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeliveryPolicy {
    OnChange,
    Latest,
    Every,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutputTransport {
    Value,
    Preview,
    Capture,
    NativePresent,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "kebab-case")]
pub enum OutputPayload {
    Bool(bool),
    Int(i64),
    Uint(u64),
    Float(f64),
    FloatArray(Vec<f64>),
    Json(serde_json::Value),
    Resource {
        handle: u64,
    },
    Tensor {
        handle: u64,
        dtype: String,
        shape: Vec<u64>,
    },
    Bytes(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputState {
    pub output: OutputKey,
    pub graph_revision: u32,
    pub output_generation: u64,
    pub evaluation_stamp: FrameStamp,
    pub content_stamp: ContentStamp,
    pub payload: OutputPayload,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputSubscription {
    pub subscription_id: String,
    pub output: OutputKey,
    pub delivery: DeliveryPolicy,
    pub transport: OutputTransport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_height: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputDelivery {
    pub subscription_id: String,
    pub state: OutputState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionInvalidation {
    pub subscription_id: String,
    pub output: OutputKey,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputDeliveryBatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame_stamp: Option<FrameStamp>,
    pub deliveries: Vec<OutputDelivery>,
    pub invalidations: Vec<SubscriptionInvalidation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PresentationFit {
    Contain,
    Cover,
    Stretch,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationItem {
    pub output: OutputKey,
    pub resource_handle: u64,
    pub viewport: Viewport,
    pub fit: PresentationFit,
    pub z_index: i32,
    pub evaluation_stamp: FrameStamp,
    pub content_stamp: ContentStamp,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationSet {
    pub group_id: String,
    pub frame_stamp: FrameStamp,
    pub items: Vec<PresentationItem>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AsyncCompletionEnvelope {
    pub node_id: String,
    pub graph_revision: u32,
    pub node_generation: u32,
    pub input_stamp: FrameStamp,
    pub content_stamp: ContentStamp,
    pub outputs: Vec<(OutputKey, OutputPayload)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataPathMode {
    #[serde(rename = "cpu-copy")]
    CpuCopy,
    #[serde(rename = "shared-gpu")]
    SharedGpu,
    #[serde(rename = "native-present")]
    NativePresent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackCapabilities {
    pub data_paths: Vec<DataPathMode>,
}

/// The graph catalog does not impose a universal color or alpha convention.
/// Unknown must remain explicit rather than silently labelling arbitrary shader values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FrameColorSpace { Unknown, Srgb, LinearBt709 }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FrameAlphaMode { Unknown, Straight, Premultiplied, Opaque }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameImageDescription {
    pub width: u32,
    pub height: u32,
    pub color_space: FrameColorSpace,
    pub alpha_mode: FrameAlphaMode,
}

/// Provenance supplied by the caller, not inferred from texture arrival time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameSnapshotIdentity {
    pub output_node_id: String,
    pub graph_revision: u32,
    pub evaluation_stamp: Option<FrameStamp>,
    pub content_stamp: Option<ContentStamp>,
    pub color_space: FrameColorSpace,
    pub alpha_mode: FrameAlphaMode,
}
