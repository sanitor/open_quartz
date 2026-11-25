use super::*;
use crate::animation_key::{self, Keyframe};
use open_quartz::project::{DataType, NodeType};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EffectKeys {
    pub node: String,
    pub parameter: String,
    pub keys: Vec<Keyframe<i64, f32>>,
}

pub(super) fn effect_uniform(pipeline: &ClipPipeline, node: &str, parameter: &str) -> Option<f32> {
    let graph = pipeline.graph();
    let node = graph.nodes.iter().find(|item| item.id == node)?;
    if !matches!(node.data.node_type, NodeType::Shader | NodeType::Constant) {
        return None;
    }
    let port = node.data.inputs.iter().find(|port| port.label == parameter && port.data_type == DataType::Float)?;
    if graph.edges.iter().any(|edge| edge.target == node.id && edge.target_handle == port.id) {
        return None;
    }
    let value = node.data.uniforms.get(parameter).or(port.default_value.as_ref())?.as_f64()? as f32;
    value.is_finite().then_some(value)
}

impl Clip {
    pub fn effect_channels(&self) -> Vec<(String, String, String)> {
        self.pipeline.graph().nodes.iter().flat_map(|node| {
            node.data.inputs.iter().filter_map(|port| {
                effect_uniform(&self.pipeline, &node.id, &port.label)?;
                Some((node.id.clone(), port.label.clone(), format!("{} · {}", node.data.label, port.label)))
            })
        }).collect()
    }

    pub fn effect_value_at(&self, node: &str, parameter: &str, frame: u64) -> Option<f32> {
        let base = effect_uniform(&self.pipeline, node, parameter)?;
        let offset = i128::from(frame) - i128::from(self.timeline_start);
        let animation = self.effect_keys.iter().find(|entry| entry.node == node && entry.parameter == parameter);
        Some(animation.map_or(base, |entry|
            animation_key::sample(&entry.keys, offset, base, animation_key::mix_scalar)))
    }

    pub(super) fn validate_effect_animation(&self) -> Result<(), String> {
        validate_effect_keys(&self.pipeline, &self.effect_keys)
    }
}

pub(super) fn validate_effect_keys(pipeline: &ClipPipeline, keys: &[EffectKeys]) -> Result<(), String> {
    let mut unique = std::collections::BTreeSet::new();
    for entry in keys {
        if !unique.insert((&entry.node, &entry.parameter))
            || effect_uniform(pipeline, &entry.node, &entry.parameter).is_none()
            || entry.keys.is_empty() || !animation_key::validate(&entry.keys, |value| value.is_finite())
        {
            return Err("Invalid animated effect parameter".into());
        }
    }
    Ok(())
}
