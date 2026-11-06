use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DataType {
    #[serde(rename = "float")]
    Float,
    #[serde(rename = "int")]
    Int,
    #[serde(rename = "uint")]
    Uint,
    #[serde(rename = "bool")]
    Bool,
    #[serde(rename = "vec2")]
    Vec2,
    #[serde(rename = "vec3")]
    Vec3,
    #[serde(rename = "vec4")]
    Vec4,
    #[serde(rename = "ivec2")]
    Ivec2,
    #[serde(rename = "ivec3")]
    Ivec3,
    #[serde(rename = "ivec4")]
    Ivec4,
    #[serde(rename = "uvec2")]
    Uvec2,
    #[serde(rename = "uvec3")]
    Uvec3,
    #[serde(rename = "uvec4")]
    Uvec4,
    #[serde(rename = "bvec2")]
    Bvec2,
    #[serde(rename = "bvec3")]
    Bvec3,
    #[serde(rename = "bvec4")]
    Bvec4,
    #[serde(rename = "mat2")]
    Mat2,
    #[serde(rename = "mat3")]
    Mat3,
    #[serde(rename = "mat4")]
    Mat4,
    #[serde(rename = "sampler2D")]
    Sampler2d,
    #[serde(rename = "samplerCube")]
    SamplerCube,
    #[serde(rename = "roi")]
    Roi,
    #[serde(rename = "mesh")]
    Mesh,
    #[serde(rename = "json")]
    Json,
    #[serde(rename = "auto")]
    Auto,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PortDirection {
    Input,
    Output,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Port {
    pub id: String,
    pub label: String,
    pub data_type: DataType,
    pub direction: PortDirection,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}
