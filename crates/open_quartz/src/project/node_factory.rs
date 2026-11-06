use super::*;
use serde_json::Value;

impl Project {
    /// Build a node with Rust-owned executable defaults and add it atomically.
    pub fn create_graph_node(
        &mut self,
        request: NodeFactoryRequest,
        expected_revision: u32,
    ) -> Result<(ProjectNode, GraphChange), SdkError> {
        self.ensure_graph_revision(expected_revision)?;
        let node = self.build_graph_node(request)?;
        let change = self.apply_graph_command(
            GraphCommand::AddNode { node: node.clone() },
            expected_revision,
        )?;
        Ok((node, change))
    }
    fn build_graph_node(&self, request: NodeFactoryRequest) -> Result<ProjectNode, SdkError> {
        let (node_type, position, label_base) = match &request {
            NodeFactoryRequest::Shader {
                position, label, ..
            } => (NodeType::Shader, position, label.as_str()),
            NodeFactoryRequest::Input { position, .. } => (NodeType::Input, position, "input"),
            NodeFactoryRequest::System { position, source } => {
                (NodeType::Input, position, system_label(*source))
            }
            NodeFactoryRequest::Constant { position } => (NodeType::Constant, position, "constant"),
            NodeFactoryRequest::Math { position, op } => {
                (NodeType::Math, position, math_label(op).unwrap_or("math"))
            }
            NodeFactoryRequest::Renderer { position } => (NodeType::Renderer, position, "renderer"),
            NodeFactoryRequest::Onnx {
                position, label, ..
            } => (NodeType::Onnx, position, label.as_str()),
            NodeFactoryRequest::CustomOnnx { position } => {
                (NodeType::Onnx, position, "custom_onnx")
            }
        };
        let id = next_node_id(&self.graph, node_type, label_base);
        let position = position
            .as_ref()
            .copied()
            .unwrap_or_else(|| next_node_position(&self.graph));
        let instance_label = format!("{}_{}", normalize_label(label_base), numeric_suffix(&id));

        let mut data = NodeData {
            node_type,
            label: instance_label,
            ..NodeData::default()
        };
        data.shader_code = String::new();

        match request {
            NodeFactoryRequest::Shader {
                code,
                label,
                template_name,
                shader_template_id,
                ..
            } => {
                let parsed = crate::compiler::shader::parse_shader(&code);
                data.label = format!("{}_{}", normalize_label(&label), numeric_suffix(&id));
                data.template_name = template_name.or_else(|| Some(label));
                data.shader_template_id = shader_template_id;
                data.shader_code = code;
                data.inputs = remap_ports(&id, parsed.inputs);
                data.outputs = remap_ports(&id, parsed.outputs);
            }
            NodeFactoryRequest::Input {
                data_type,
                input_mode,
                ..
            } => {
                let code = input_shader_code(data_type);
                let parsed = crate::compiler::shader::parse_shader(&code);
                data.shader_code = code;
                data.input_data_type = Some(data_type);
                data.input_mode = Some(input_mode.unwrap_or(if data_type == DataType::Sampler2d {
                    InputMode::Image
                } else {
                    InputMode::System
                }));
                data.inputs = remap_ports(&id, parsed.inputs);
                data.outputs = remap_ports(&id, parsed.outputs)
                    .into_iter()
                    .map(|mut port| {
                        port.data_type = data_type;
                        port
                    })
                    .collect();
                if data_type != DataType::Sampler2d {
                    data.input_mode = input_mode;
                }
            }
            NodeFactoryRequest::System { source, .. } => {
                let data_type = system_data_type(source);
                let code = system_shader_code(source);
                let parsed = crate::compiler::shader::parse_shader(&code);
                data.shader_code = code;
                data.input_data_type = Some(data_type);
                data.input_mode = Some(InputMode::System);
                data.system_source = Some(source);
                data.inputs = remap_ports(&id, parsed.inputs);
                data.outputs = remap_ports(&id, parsed.outputs)
                    .into_iter()
                    .map(|mut port| {
                        port.data_type = data_type;
                        port
                    })
                    .collect();
            }
            NodeFactoryRequest::Constant { .. } => {
                let code = constant_shader_code();
                let parsed = crate::compiler::shader::parse_shader(&code);
                data.shader_code = code;
                data.inputs = remap_ports(&id, parsed.inputs);
                data.outputs = remap_ports(&id, parsed.outputs);
            }
            NodeFactoryRequest::Math { op, .. } => {
                let input_count = math_input_count(&op).ok_or_else(|| {
                    SdkError::new(SdkErrorCode::InvalidGraph, "Unknown math operation")
                        .with_details(op.clone())
                })?;
                data.label = format!(
                    "{}_{}",
                    normalize_label(math_label(&op).unwrap()),
                    numeric_suffix(&id)
                );
                data.template_name = math_label(&op).map(str::to_owned);
                data.math_op = Some(op);
                data.inputs = math_ports(&id, input_count, PortDirection::Input);
                data.outputs = math_ports(&id, 1, PortDirection::Output)
                    .into_iter()
                    .map(|mut port| {
                        port.label = "result".to_owned();
                        port.id = format!("{id}_result");
                        port
                    })
                    .collect();
            }
            NodeFactoryRequest::Renderer { .. } => {
                data.extra.insert("expanded".to_owned(), Value::Bool(true));
                data.inputs = vec![Port {
                    id: format!("{id}_inputTexture"),
                    label: "inputTexture".to_owned(),
                    data_type: DataType::Sampler2d,
                    direction: PortDirection::Input,
                    default_value: None,
                    description: None,
                }];
            }
            NodeFactoryRequest::Onnx {
                label,
                template_name,
                model_id,
                catalog_id,
                inputs,
                outputs,
                ..
            } => {
                data.label = format!("{}_{}", normalize_label(&label), numeric_suffix(&id));
                data.template_name = template_name.or_else(|| Some(label));
                data.onnx_model_id = model_id;
                data.onnx_catalog_id = catalog_id;
                data.onnx_source = Some(OnnxSource::Catalog);
                data.extra.insert(
                    "onnxStatus".to_owned(),
                    Value::String("not-downloaded".to_owned()),
                );
                data.inputs = remap_ports(&id, inputs);
                data.outputs = remap_ports(&id, outputs);
            }
            NodeFactoryRequest::CustomOnnx { .. } => {
                data.label = "Custom ONNX".to_owned();
                data.onnx_source = Some(OnnxSource::Custom);
                data.inputs = Vec::new();
                data.outputs = Vec::new();
            }
        }

        Ok(ProjectNode {
            id,
            node_type,
            position,
            data,
        })
    }
}

fn next_node_id(graph: &Graph, node_type: NodeType, label_base: &str) -> String {
    let prefix = match node_type {
        NodeType::Shader => "shader",
        NodeType::Input => "input",
        NodeType::Constant => "constant",
        NodeType::Onnx => {
            if label_base == "custom_onnx" {
                "onnx"
            } else {
                "onnx"
            }
        }
        NodeType::Renderer => "renderer",
        NodeType::Math => "math",
    };
    let mut next = 1u32;
    for node in graph.nodes() {
        let Some((node_prefix, suffix)) = node.id.rsplit_once('_') else {
            continue;
        };
        if node_prefix == prefix {
            if let Ok(number) = suffix.parse::<u32>() {
                next = next.max(number.saturating_add(1));
            }
        }
    }
    format!("{prefix}_{next}")
}

fn numeric_suffix(id: &str) -> &str {
    id.rsplit_once('_').map(|(_, suffix)| suffix).unwrap_or("1")
}

fn normalize_label(value: &str) -> String {
    value
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("_")
}

fn next_node_position(graph: &Graph) -> Position {
    let offset = graph.nodes().len() as f64 * 28.0;
    Position {
        x: 100.0 + offset,
        y: 100.0 + offset,
    }
}

fn remap_ports(node_id: &str, ports: Vec<Port>) -> Vec<Port> {
    ports
        .into_iter()
        .map(|mut port| {
            port.id = format!("{node_id}_{}", port.label);
            port
        })
        .collect()
}

pub(super) fn remap_ports_preserving_ids(existing: &[Port], node_id: &str, ports: Vec<Port>) -> Vec<Port> {
    ports
        .into_iter()
        .map(|mut port| {
            if let Some(previous) = existing
                .iter()
                .find(|candidate| candidate.label == port.label)
            {
                port.id = previous.id.clone();
            } else {
                port.id = format!("{node_id}_{}", port.label);
            }
            port
        })
        .collect()
}

pub(super) fn input_shader_code(data_type: DataType) -> String {
    if data_type == DataType::Sampler2d {
        "@group(0) @binding(0) var value: texture_2d<f32>;\n\
         @group(0) @binding(1) var valueSampler: sampler;\n\
         @fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f { \
           return textureSample(value, valueSampler, v_uv); \
         }"
        .replace("         ", "")
    } else {
        "@group(0) @binding(0) var<uniform> value: f32;\n\
         @fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f { \
           return vec4f(value, 0.0, 0.0, 1.0); \
         }"
        .replace("         ", "")
    }
}

fn constant_shader_code() -> String {
    "@group(0) @binding(0) var<uniform> color: vec4f;\n\
     @fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f { return color; }"
        .replace("     ", "")
}

fn system_label(source: SystemSource) -> &'static str {
    match source {
        SystemSource::Time => "time",
        SystemSource::TimeDelta => "time_delta",
        SystemSource::Frame => "frame",
        SystemSource::Mouse => "mouse",
        SystemSource::Resolution => "resolution",
    }
}

fn system_data_type(source: SystemSource) -> DataType {
    match source {
        SystemSource::Time | SystemSource::TimeDelta => DataType::Float,
        SystemSource::Frame => DataType::Int,
        SystemSource::Mouse => DataType::Vec4,
        SystemSource::Resolution => DataType::Vec3,
    }
}

fn system_shader_code(source: SystemSource) -> String {
    let (wgsl_type, expression) = match source {
        SystemSource::Time | SystemSource::TimeDelta => ("f32", "vec4f(value, 0.0, 0.0, 1.0)"),
        SystemSource::Frame => ("i32", "vec4f(f32(value), 0.0, 0.0, 1.0)"),
        SystemSource::Mouse => ("vec4f", "value"),
        SystemSource::Resolution => ("vec3f", "vec4f(value, 1.0)"),
    };
    format!(
        "@group(0) @binding(0) var<uniform> value: {wgsl_type};\n\
         @fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {{ return {expression}; }}"
    )
}

fn math_label(op: &str) -> Option<&'static str> {
    Some(match op {
        "add" => "Add",
        "subtract" => "Subtract",
        "multiply" => "Multiply",
        "divide" => "Divide",
        "negate" => "Negate",
        "modulo" => "Modulo",
        "min" => "Min",
        "max" => "Max",
        "clamp" => "Clamp",
        "saturate" => "Saturate",
        "step" => "Step",
        "smoothstep" => "Smoothstep",
        "abs" => "Abs",
        "sign" => "Sign",
        "sin" => "Sin",
        "cos" => "Cos",
        "tan" => "Tan",
        "asin" => "Asin",
        "acos" => "Acos",
        "atan" => "Atan",
        "pow" => "Pow",
        "sqrt" => "Sqrt",
        "exp" => "Exp",
        "log" => "Log",
        "mix" => "Mix",
        "floor" => "Floor",
        "ceil" => "Ceil",
        "round" => "Round",
        "fract" => "Fract",
        _ => return None,
    })
}

fn math_input_count(op: &str) -> Option<usize> {
    Some(match op {
        "negate" | "saturate" | "abs" | "sign" | "sin" | "cos" | "tan" | "asin" | "acos"
        | "atan" | "sqrt" | "exp" | "log" | "floor" | "ceil" | "round" | "fract" => 1,
        "add" | "subtract" | "multiply" | "divide" | "modulo" | "min" | "max" | "step" | "pow" => 2,
        "clamp" | "smoothstep" | "mix" => 3,
        _ => return None,
    })
}

fn math_ports(node_id: &str, count: usize, direction: PortDirection) -> Vec<Port> {
    let labels = ["a", "b", "c"];
    (0..count)
        .map(|index| Port {
            id: format!(
                "{node_id}_{}",
                labels.get(index).copied().unwrap_or("value")
            ),
            label: labels.get(index).copied().unwrap_or("value").to_owned(),
            data_type: DataType::Auto,
            direction,
            default_value: (direction == PortDirection::Input).then_some(Value::from(0)),
            description: None,
        })
        .collect()
}

#[cfg(test)]
#[path = "node_factory_coverage_tests.rs"]
mod coverage_tests;
