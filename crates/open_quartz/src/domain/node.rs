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
    Onnx,
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
pub enum OnnxSource {
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
pub enum OnnxParamValue {
    Number(f64),
    Boolean(bool),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onnx_model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onnx_score_threshold: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onnx_iou_threshold: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onnx_target_size: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onnx_source: Option<OnnxSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onnx_catalog_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onnx_custom_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onnx_params: Option<BTreeMap<String, OnnxParamValue>>,
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
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
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
            onnx_model_id: None,
            onnx_score_threshold: None,
            onnx_iou_threshold: None,
            onnx_target_size: None,
            onnx_source: None,
            onnx_catalog_id: None,
            onnx_custom_path: None,
            onnx_params: None,
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
