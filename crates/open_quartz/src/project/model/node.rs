use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::Port;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeType {
    Shader,
    Input,
    Constant,
    #[serde(alias = "onnx")]
    Inference,
    Renderer,
    Math,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputMode {
    Image,
    Framebuffer,
    Video,
    System,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FramebufferFormat {
    Rgba8,
    Rgba32f,
    Rg8,
    Rg32f,
    R8,
    R32f,
    Nv12,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextureFilter {
    Linear,
    Nearest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextureWrap {
    Clamp,
    Repeat,
    Mirror,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InferenceSource {
    Catalog,
    Custom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoSourceType {
    Camera,
    File,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SystemSource {
    Time,
    TimeDelta,
    Frame,
    Mouse,
    Resolution,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InferenceParamValue {
    Number(f64),
    Boolean(bool),
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeData {
    #[serde(rename = "type")]
    pub node_type: NodeType,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shader_template_id: Option<String>,
    pub shader_code: String,
    pub inputs: Vec<Port>,
    pub outputs: Vec<Port>,
    pub uniforms: Map<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_data_type: Option<super::DataType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_mode: Option<InputMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_data_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fb_format: Option<FramebufferFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fb_width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fb_height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fb_stride: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_data_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tex_filter: Option<TextureFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tex_wrap: Option<TextureWrap>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_size: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out_format: Option<FramebufferFormat>,
    #[serde(alias = "onnxModelId", skip_serializing_if = "Option::is_none")]
    pub inference_model_id: Option<String>,
    #[serde(alias = "onnxScoreThreshold", skip_serializing_if = "Option::is_none")]
    pub inference_score_threshold: Option<f64>,
    #[serde(alias = "onnxIouThreshold", skip_serializing_if = "Option::is_none")]
    pub inference_iou_threshold: Option<f64>,
    #[serde(alias = "onnxTargetSize", skip_serializing_if = "Option::is_none")]
    pub inference_target_size: Option<u32>,
    #[serde(alias = "onnxSource", skip_serializing_if = "Option::is_none")]
    pub inference_source: Option<InferenceSource>,
    #[serde(alias = "onnxCatalogId", skip_serializing_if = "Option::is_none")]
    pub inference_catalog_id: Option<String>,
    #[serde(
        alias = "onnxCustomPath",
        alias = "onnxModelPath",
        alias = "inferenceCustomPath",
        alias = "customPath",
        skip_serializing_if = "Option::is_none"
    )]
    pub inference_model_path: Option<String>,
    #[serde(alias = "onnxParams", skip_serializing_if = "Option::is_none")]
    pub inference_params: Option<BTreeMap<String, InferenceParamValue>>,
    #[serde(alias = "onnxTask", skip_serializing_if = "Option::is_none")]
    pub inference_task: Option<super::InferenceTask>,
    #[serde(alias = "onnxPolicy", skip_serializing_if = "Option::is_none")]
    pub inference_policy: Option<crate::inference::ExecutionPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_source_type: Option<VideoSourceType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_file_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_loop: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_playback_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_source: Option<SystemSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub math_op: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feedback_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feedback_clear_color: Option<[f64; 4]>,
    #[serde(flatten, default, serialize_with = "serialize_inference_extras")]
    pub extra: Map<String, Value>,
}

// Decode normalized keys into the public data model without changing its API.
#[derive(Deserialize)]
#[serde(remote = "NodeData", rename_all = "camelCase")]
struct NodeDataWire {
    #[serde(rename = "type")]
    node_type: NodeType,
    label: String,
    template_name: Option<String>,
    shader_template_id: Option<String>,
    shader_code: String,
    inputs: Vec<Port>,
    outputs: Vec<Port>,
    uniforms: Map<String, Value>,
    input_data_type: Option<super::DataType>,
    input_mode: Option<InputMode>,
    image_data_url: Option<String>,
    image_width: Option<u32>,
    image_height: Option<u32>,
    fb_format: Option<FramebufferFormat>,
    fb_width: Option<u32>,
    fb_height: Option<u32>,
    fb_stride: Option<u32>,
    raw_data_url: Option<String>,
    tex_filter: Option<TextureFilter>,
    tex_wrap: Option<TextureWrap>,
    width: Option<u32>,
    height: Option<u32>,
    auto_size: Option<bool>,
    out_format: Option<FramebufferFormat>,
    inference_model_id: Option<String>,
    inference_score_threshold: Option<f64>,
    inference_iou_threshold: Option<f64>,
    inference_target_size: Option<u32>,
    inference_source: Option<InferenceSource>,
    inference_catalog_id: Option<String>,
    inference_model_path: Option<String>,
    inference_params: Option<BTreeMap<String, InferenceParamValue>>,
    inference_task: Option<super::InferenceTask>,
    inference_policy: Option<crate::inference::ExecutionPolicy>,
    video_source_type: Option<VideoSourceType>,
    video_url: Option<String>,
    video_file_path: Option<String>,
    video_device_id: Option<String>,
    video_loop: Option<bool>,
    video_playback_rate: Option<f64>,
    system_source: Option<SystemSource>,
    math_op: Option<String>,
    feedback_enabled: Option<bool>,
    feedback_clear_color: Option<[f64; 4]>,
    #[serde(flatten, default)]
    extra: Map<String, Value>,
}

impl<'de> Deserialize<'de> for NodeData {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut data = Map::<String, Value>::deserialize(deserializer)?;
        // The first present spelling wins, including an explicit canonical null.
        let mut model_path = None;
        for key in [
            "inferenceModelPath",
            "inferenceCustomPath",
            "onnxModelPath",
            "onnxCustomPath",
            "customPath",
        ] {
            if let Some(value) = data.remove(key) {
                if model_path.is_none() {
                    model_path = Some(value);
                }
            }
        }
        if let Some(value) = model_path {
            data.insert("inferenceModelPath".to_owned(), value);
        }
        let inference = matches!(
            data.get("type").and_then(Value::as_str),
            Some("inference" | "onnx")
        );
        let mut params = None;
        for key in ["params", "onnxParams", "inferenceParams"] {
            if key == "params" && !inference {
                continue;
            }
            if let Some(mut value) = data.remove(key) {
                if key != "inferenceParams" {
                    if let Some(legacy) = value.as_object_mut() {
                        for (param, canonical, old) in [
                            ("targetSize", "inferenceTargetSize", "onnxTargetSize"),
                            (
                                "scoreThreshold",
                                "inferenceScoreThreshold",
                                "onnxScoreThreshold",
                            ),
                            ("iouThreshold", "inferenceIouThreshold", "onnxIouThreshold"),
                        ] {
                            if data.contains_key(canonical) || data.contains_key(old) {
                                legacy.remove(param);
                            }
                        }
                        if [
                            "inferencePreferDirectMl",
                            "preferDirectMl",
                            "onnxPreferDirectMl",
                        ]
                        .iter()
                        .any(|field| data.contains_key(*field))
                        {
                            legacy.remove("preferDirectMl");
                        }
                    }
                }
                match (&mut params, value) {
                    (Some(Value::Object(previous)), Value::Object(current)) => {
                        previous.extend(current)
                    }
                    (params, value) => *params = Some(value),
                }
            }
        }
        if let Some(params) = params {
            data.insert("inferenceParams".to_owned(), params);
        }
        NodeDataWire::deserialize(Value::Object(canonical_inference_extras(data)))
            .map_err(serde::de::Error::custom)
    }
}

impl Default for NodeData {
    fn default() -> Self {
        Self {
            node_type: NodeType::Shader,
            label: String::new(),
            template_name: None,
            shader_template_id: None,
            shader_code: String::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            uniforms: Map::new(),
            input_data_type: None,
            input_mode: None,
            image_data_url: None,
            image_width: None,
            image_height: None,
            fb_format: None,
            fb_width: None,
            fb_height: None,
            fb_stride: None,
            raw_data_url: None,
            tex_filter: None,
            tex_wrap: None,
            width: None,
            height: None,
            auto_size: None,
            out_format: None,
            inference_model_id: None,
            inference_score_threshold: None,
            inference_iou_threshold: None,
            inference_target_size: None,
            inference_source: None,
            inference_catalog_id: None,
            inference_model_path: None,
            inference_params: None,
            inference_task: None,
            inference_policy: None,
            video_source_type: None,
            video_url: None,
            video_file_path: None,
            video_device_id: None,
            video_loop: None,
            video_playback_rate: None,
            system_source: None,
            math_op: None,
            feedback_enabled: None,
            feedback_clear_color: None,
            extra: Map::new(),
        }
    }
}

fn canonical_inference_extras(extra: Map<String, Value>) -> Map<String, Value> {
    let mut canonical = Map::new();
    for (key, value) in &extra {
        if let Some(suffix) = key.strip_prefix("onnx") {
            canonical.insert(format!("inference{suffix}"), value.clone());
        }
    }
    for (key, value) in extra {
        if !key.starts_with("onnx") {
            canonical.insert(key, value);
        }
    }
    canonical
}

fn serialize_inference_extras<S>(
    extra: &Map<String, Value>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    canonical_inference_extras(extra.clone()).serialize(serializer)
}
