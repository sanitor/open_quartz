use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::Value;

use crate::graph::{DirtySet, GraphEdge};
use crate::types::{DataType, Edge, NodeType, ProjectNode, SystemSource};

use super::frame::{ExecutionCommand, FrameInputs, FrameResult};
use super::plan::{build_execution_plan_with_options, ExecutionPlan, NodeExecutionPlan};

pub struct ExecutionEngine {
    nodes: Vec<ProjectNode>,
    plan: ExecutionPlan,
    dirty: DirtySet,
    feedback_read_index: HashMap<String, u8>,
    feedback_first_frame: HashSet<String>,
    math_values: HashMap<String, f64>,
    external_video_textures: bool,
}

impl ExecutionEngine {
    pub fn prepare(nodes: Vec<ProjectNode>, edges: Vec<Edge>) -> Self {
        Self::prepare_with_options(nodes, edges, true)
    }

    pub fn prepare_with_options(
        nodes: Vec<ProjectNode>,
        edges: Vec<Edge>,
        external_video_textures: bool,
    ) -> Self {
        let plan = build_execution_plan_with_options(&nodes, &edges, external_video_textures);
        let graph_edges = edges
            .iter()
            .map(|edge| GraphEdge {
                source: edge.source.clone(),
                target: edge.target.clone(),
            })
            .collect::<Vec<_>>();
        let mut dirty = DirtySet::new(&graph_edges);
        dirty.mark_all(plan.sorted_ids.clone());
        let feedback_first_frame = plan
            .nodes
            .iter()
            .filter(|node| node.feedback)
            .map(|node| node.id.clone())
            .collect();
        Self {
            nodes,
            plan,
            dirty,
            feedback_read_index: HashMap::new(),
            feedback_first_frame,
            math_values: HashMap::new(),
            external_video_textures,
        }
    }

    pub fn plan(&self) -> &ExecutionPlan {
        &self.plan
    }

    pub fn mark_dirty(&mut self, node_id: &str) {
        self.dirty.mark_dirty(node_id);
    }

    pub fn replace_graph(&mut self, nodes: Vec<ProjectNode>, edges: Vec<Edge>) {
        *self = Self::prepare_with_options(nodes, edges, self.external_video_textures);
    }

    pub fn replace_graph_preserving_state(
        &mut self,
        nodes: Vec<ProjectNode>,
        edges: Vec<Edge>,
        changed_nodes: &HashSet<String>,
    ) {
        let mut next = Self::prepare_with_options(nodes, edges, self.external_video_textures);
        next.dirty.take_in_order(&next.plan.sorted_ids);
        for node_id in changed_nodes {
            next.dirty.mark_dirty(node_id);
        }

        for node in &next.plan.nodes {
            if changed_nodes.contains(&node.id) {
                continue;
            }
            if let Some(read_index) = self.feedback_read_index.get(&node.id) {
                next.feedback_read_index
                    .insert(node.id.clone(), *read_index);
            }
            if !self.feedback_first_frame.contains(&node.id) {
                next.feedback_first_frame.remove(&node.id);
            }
            if let Some(value) = self.math_values.get(&node.id) {
                next.math_values.insert(node.id.clone(), *value);
            }
        }

        *self = next;
    }

    pub fn run_frame(&mut self, inputs: &FrameInputs) -> FrameResult {
        self.mark_dynamic_nodes(inputs);
        let dirty_nodes = self.dirty.take_in_order(&self.plan.sorted_ids);
        let mut commands = Vec::new();

        for node_id in &dirty_nodes {
            let Some(node_plan) = self
                .plan
                .nodes
                .iter()
                .find(|plan| plan.id == *node_id)
                .cloned()
            else {
                continue;
            };
            let Some(node) = self.nodes.iter().find(|node| node.id == *node_id).cloned() else {
                continue;
            };
            match node_plan.node_type {
                NodeType::Input => {}
                NodeType::Math => {
                    let value = self.evaluate_math(&node_plan, &node);
                    self.math_values.insert(node_id.clone(), value);
                    commands.push(command_for(
                        &node_plan,
                        "math",
                        node.data
                            .outputs
                            .iter()
                            .find(|port| port.data_type == DataType::Float)
                            .map(|port| port.id.clone()),
                        BTreeMap::new(),
                        None,
                        false,
                        Some(value),
                    ));
                }
                NodeType::Shader | NodeType::Constant => {
                    let uniforms = self.resolve_uniforms(&node_plan, &node, inputs);
                    let (read_index, write_index, clear_feedback) = if node_plan.feedback {
                        let read = self.feedback_read_index.get(node_id).copied().unwrap_or(0);
                        let write = 1 - read;
                        let clear = self.feedback_first_frame.remove(node_id);
                        self.feedback_read_index.insert(node_id.clone(), write);
                        (Some(read), Some(write), clear)
                    } else {
                        (None, None, false)
                    };
                    commands.push(command_for(
                        &node_plan,
                        if node_plan.node_type == NodeType::Shader {
                            "shader"
                        } else {
                            "constant"
                        },
                        None,
                        uniforms,
                        read_index.zip(write_index),
                        clear_feedback,
                        None,
                    ));
                }
                NodeType::Onnx => commands.push(command_for(
                    &node_plan,
                    "onnx",
                    None,
                    BTreeMap::new(),
                    None,
                    false,
                    None,
                )),
                NodeType::Renderer => commands.push(command_for(
                    &node_plan,
                    "renderer",
                    None,
                    BTreeMap::new(),
                    None,
                    false,
                    None,
                )),
            }
        }

        FrameResult {
            frame: inputs.frame,
            commands,
            dirty_nodes,
        }
    }

    fn mark_dynamic_nodes(&mut self, inputs: &FrameInputs) {
        let mut dynamic = Vec::new();
        for node in &self.nodes {
            let is_dynamic_system = matches!(
                node.data.system_source,
                Some(
                    SystemSource::Time
                        | SystemSource::TimeDelta
                        | SystemSource::Frame
                        | SystemSource::Mouse
                )
            );
            let dynamic_shader =
                matches!(node.data.node_type, NodeType::Shader | NodeType::Constant)
                    && ["iTime", "iTimeDelta", "iFrame", "iMouse", "previousFrame"]
                        .iter()
                        .any(|name| node.data.shader_code.contains(name));
            if is_dynamic_system || dynamic_shader || inputs.video_nodes.contains(&node.id) {
                dynamic.push(node.id.clone());
            }
        }
        for node_id in dynamic {
            self.dirty.mark_dirty(&node_id);
        }
    }

    fn resolve_uniforms(
        &self,
        node_plan: &NodeExecutionPlan,
        node: &ProjectNode,
        inputs: &FrameInputs,
    ) -> BTreeMap<String, Vec<f32>> {
        let mut uniforms = BTreeMap::new();
        let Some(shader) = &node_plan.shader else {
            return uniforms;
        };
        for name in shader.uniform_bindings.keys() {
            let value = if node_plan.builtin_ports.contains(name) {
                builtin_value(name, node_plan, inputs)
            } else if let Some(source_id) = node_plan.upstream.get(name) {
                self.upstream_scalar(source_id).unwrap_or_else(|| vec![0.0])
            } else {
                node.data
                    .uniforms
                    .get(name)
                    .map(json_value_to_f32)
                    .unwrap_or_else(|| vec![0.0])
            };
            uniforms.insert(name.clone(), value);
        }
        uniforms
    }

    fn upstream_scalar(&self, source_id: &str) -> Option<Vec<f32>> {
        if let Some(value) = self.math_values.get(source_id) {
            return Some(vec![*value as f32]);
        }
        let source = self.nodes.iter().find(|node| node.id == source_id)?;
        let label = source.data.inputs.first()?.label.as_str();
        source.data.uniforms.get(label).map(json_value_to_f32)
    }

    fn evaluate_math(&self, plan: &NodeExecutionPlan, node: &ProjectNode) -> f64 {
        let values = ["a", "b", "c"]
            .iter()
            .map(|label| {
                plan.upstream
                    .get(*label)
                    .and_then(|source| self.upstream_scalar(source))
                    .and_then(|value| value.first().copied())
                    .map(f64::from)
                    .or_else(|| node.data.uniforms.get(*label).and_then(Value::as_f64))
                    .unwrap_or(0.0)
            })
            .collect::<Vec<_>>();
        math_compute(node.data.math_op.as_deref().unwrap_or("add"), &values)
    }
}

fn command_for(
    plan: &NodeExecutionPlan,
    kind: &str,
    output_port_id: Option<String>,
    uniforms: BTreeMap<String, Vec<f32>>,
    feedback: Option<(u8, u8)>,
    clear_feedback: bool,
    scalar_output: Option<f64>,
) -> ExecutionCommand {
    let texture_inputs = plan
        .shader
        .as_ref()
        .map(|shader| {
            plan.upstream
                .iter()
                .filter(|(name, _)| {
                    shader.texture_bindings.contains_key(*name)
                        || shader.external_texture_bindings.contains_key(*name)
                })
                .map(|(name, source)| (name.clone(), source.clone()))
                .collect()
        })
        .unwrap_or_else(|| plan.upstream.clone());
    ExecutionCommand {
        node_id: plan.id.clone(),
        kind: kind.to_owned(),
        output_port_id,
        texture_inputs,
        uniforms,
        target_width: plan.target.as_ref().map(|target| target.width),
        target_height: plan.target.as_ref().map(|target| target.height),
        feedback_read_index: feedback.map(|value| value.0),
        feedback_write_index: feedback.map(|value| value.1),
        clear_feedback,
        scalar_output,
    }
}

fn builtin_value(name: &str, plan: &NodeExecutionPlan, inputs: &FrameInputs) -> Vec<f32> {
    match name {
        "iTime" => vec![inputs.time as f32],
        "iTimeDelta" => vec![inputs.delta as f32],
        "iFrame" => vec![inputs.frame as f32],
        "iDate" => inputs.date.to_vec(),
        "iMouse" => inputs.mouse.to_vec(),
        "iResolution" => plan
            .target
            .as_ref()
            .map(|target| vec![target.width as f32, target.height as f32, 1.0])
            .unwrap_or_else(|| inputs.resolution.to_vec()),
        _ => vec![0.0],
    }
}

fn json_value_to_f32(value: &Value) -> Vec<f32> {
    match value {
        Value::Array(values) => values
            .iter()
            .map(|item| item.as_f64().unwrap_or(0.0) as f32)
            .collect(),
        Value::Bool(value) => vec![u8::from(*value) as f32],
        Value::Number(value) => vec![value.as_f64().unwrap_or(0.0) as f32],
        _ => vec![0.0],
    }
}

fn math_compute(operation: &str, input: &[f64]) -> f64 {
    let a = input.first().copied().unwrap_or(0.0);
    let b = input.get(1).copied().unwrap_or(0.0);
    let c = input.get(2).copied().unwrap_or(0.0);
    match operation {
        "add" => a + b,
        "subtract" => a - b,
        "multiply" => a * b,
        "divide" => safe_divide(a, b),
        "negate" => -a,
        "modulo" => safe_modulo(a, b),
        "min" => a.min(b),
        "max" => a.max(b),
        "clamp" => a.max(b).min(c),
        "saturate" => a.clamp(0.0, 1.0),
        "step" => f64::from(b >= a),
        "smoothstep" => {
            let t = ((c - a) / (b - a)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        }
        "abs" => a.abs(),
        "sign" => a.signum(),
        "sin" => a.sin(),
        "cos" => a.cos(),
        "tan" => a.tan(),
        "asin" => a.asin(),
        "acos" => a.acos(),
        "atan" => a.atan(),
        "pow" => a.powf(b),
        "sqrt" => a.sqrt(),
        "exp" => a.exp(),
        "log" => a.ln(),
        "mix" => a * (1.0 - c) + b * c,
        "floor" => a.floor(),
        "ceil" => a.ceil(),
        "round" => a.round(),
        "fract" => a.fract(),
        _ => 0.0,
    }
}

fn safe_divide(a: f64, b: f64) -> f64 {
    if b == 0.0 {
        0.0
    } else {
        a / b
    }
}

fn safe_modulo(a: f64, b: f64) -> f64 {
    if b == 0.0 {
        0.0
    } else {
        a % b
    }
}
