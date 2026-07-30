use std::collections::{HashMap, HashSet};

use naga::{AddressSpace, ImageClass, ShaderStage, TypeInner};
use regex::Regex;
use serde::Serialize;

use crate::types::{DataType, Port, PortDirection};

const BUILTIN_UNIFORMS: &[&str] = &[
    "iTime",
    "iTimeDelta",
    "iFrame",
    "iDate",
    "iMouse",
    "iResolution",
    "previousFrame",
];

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedShader {
    pub inputs: Vec<Port>,
    pub outputs: Vec<Port>,
    pub raw: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

pub fn parse_shader(code: &str) -> ParsedShader {
    let (description, port_descriptions) = extract_comments(code);
    let mut result = ParsedShader {
        inputs: Vec::new(),
        outputs: Vec::new(),
        raw: code.to_owned(),
        parse_error: None,
        description,
    };

    match naga::front::wgsl::parse_str(code) {
        Ok(module) => {
            extract_naga_bindings(&module, &port_descriptions, &mut result);
            extract_naga_outputs(&module, &mut result);
            if result.inputs.is_empty() {
                fallback_extract(code, &port_descriptions, &mut result);
            }
        }
        Err(error) => {
            fallback_extract(code, &port_descriptions, &mut result);
            let has_fragment_output = code.contains("@location") && code.contains("->");
            if !has_fragment_output {
                result.parse_error = Some(error.emit_to_string(code));
            }
        }
    }

    result
}

fn extract_naga_bindings(
    module: &naga::Module,
    descriptions: &HashMap<String, String>,
    result: &mut ParsedShader,
) {
    let mut seen = HashSet::new();
    for (_, global) in module.global_variables.iter() {
        let Some(name) = global.name.as_deref() else {
            continue;
        };
        if BUILTIN_UNIFORMS.contains(&name) || !seen.insert(name.to_owned()) {
            continue;
        }

        let ty = &module.types[global.ty].inner;
        let data_type = match ty {
            TypeInner::Image {
                class: ImageClass::Sampled { .. },
                ..
            } => Some(DataType::Sampler2d),
            TypeInner::Image { .. } => Some(DataType::Sampler2d),
            TypeInner::Sampler { .. } => None,
            _ if matches!(global.space, AddressSpace::Uniform) => Some(type_inner_to_data_type(ty)),
            _ => None,
        };
        let Some(data_type) = data_type else { continue };
        result.inputs.push(port(
            name,
            data_type,
            PortDirection::Input,
            descriptions.get(name).cloned(),
        ));
    }
}

fn extract_naga_outputs(module: &naga::Module, result: &mut ParsedShader) {
    let Some(entry) = module
        .entry_points
        .iter()
        .find(|entry| entry.stage == ShaderStage::Fragment)
    else {
        return;
    };
    let Some(function_result) = entry.function.result.as_ref() else {
        return;
    };
    let data_type = type_handle_to_data_type(module, function_result.ty);
    result
        .outputs
        .push(port("fragColor", data_type, PortDirection::Output, None));
}

fn type_handle_to_data_type(module: &naga::Module, handle: naga::Handle<naga::Type>) -> DataType {
    type_inner_to_data_type(&module.types[handle].inner)
}

fn type_inner_to_data_type(inner: &TypeInner) -> DataType {
    match inner {
        TypeInner::Scalar(scalar) if scalar.kind == naga::ScalarKind::Sint => DataType::Int,
        TypeInner::Scalar(scalar) if scalar.kind == naga::ScalarKind::Uint => DataType::Uint,
        TypeInner::Scalar(scalar) if scalar.kind == naga::ScalarKind::Bool => DataType::Bool,
        TypeInner::Scalar(_) => DataType::Float,
        TypeInner::Vector { size, scalar } => match (size, scalar.kind) {
            (naga::VectorSize::Bi, naga::ScalarKind::Sint) => DataType::Ivec2,
            (naga::VectorSize::Tri, naga::ScalarKind::Sint) => DataType::Ivec3,
            (naga::VectorSize::Quad, naga::ScalarKind::Sint) => DataType::Ivec4,
            (naga::VectorSize::Bi, naga::ScalarKind::Uint) => DataType::Uvec2,
            (naga::VectorSize::Tri, naga::ScalarKind::Uint) => DataType::Uvec3,
            (naga::VectorSize::Quad, naga::ScalarKind::Uint) => DataType::Uvec4,
            (naga::VectorSize::Bi, naga::ScalarKind::Bool) => DataType::Bvec2,
            (naga::VectorSize::Tri, naga::ScalarKind::Bool) => DataType::Bvec3,
            (naga::VectorSize::Quad, naga::ScalarKind::Bool) => DataType::Bvec4,
            (naga::VectorSize::Bi, _) => DataType::Vec2,
            (naga::VectorSize::Tri, _) => DataType::Vec3,
            (naga::VectorSize::Quad, _) => DataType::Vec4,
        },
        TypeInner::Matrix { columns, rows, .. } => match (columns, rows) {
            (naga::VectorSize::Bi, naga::VectorSize::Bi) => DataType::Mat2,
            (naga::VectorSize::Tri, naga::VectorSize::Tri) => DataType::Mat3,
            _ => DataType::Mat4,
        },
        _ => DataType::Float,
    }
}

fn fallback_extract(code: &str, descriptions: &HashMap<String, String>, result: &mut ParsedShader) {
    let clean = strip_comments(code);
    let mut seen = result
        .inputs
        .iter()
        .map(|port| port.label.clone())
        .collect::<HashSet<_>>();

    let binding_re = Regex::new(
        r"@group\s*\(\s*\d+\s*\)\s*@binding\s*\(\s*\d+\s*\)\s*var(?:<\w+>)?\s+(\w+)\s*:\s*([\w<>]+)",
    )
    .expect("valid binding regex");
    for captures in binding_re.captures_iter(&clean) {
        let name = &captures[1];
        if BUILTIN_UNIFORMS.contains(&name) || !seen.insert(name.to_owned()) {
            continue;
        }
        let type_name = &captures[2];
        let data_type = if type_name.starts_with("texture_") {
            DataType::Sampler2d
        } else {
            map_type_name(type_name)
        };
        result.inputs.push(port(
            name,
            data_type,
            PortDirection::Input,
            descriptions.get(name).cloned(),
        ));
    }

    let texture_re = Regex::new(r"textureSample\w*\s*\(\s*(\w+)\s*,").expect("valid texture regex");
    let dimensions_re =
        Regex::new(r"textureDimensions\s*\(\s*(\w+)\s*\)").expect("valid dimensions regex");
    for captures in texture_re
        .captures_iter(&clean)
        .chain(dimensions_re.captures_iter(&clean))
    {
        let name = &captures[1];
        if !BUILTIN_UNIFORMS.contains(&name) && seen.insert(name.to_owned()) {
            result
                .inputs
                .push(port(name, DataType::Sampler2d, PortDirection::Input, None));
        }
    }

    let locals = local_names(&clean);
    let keywords = keyword_names();
    let identifier_re = Regex::new(r"\b([A-Za-z_]\w*)\b").expect("valid identifier regex");
    for captures in identifier_re.captures_iter(&clean) {
        let name = &captures[1];
        let start = captures.get(1).expect("identifier capture").start();
        if start > 0 && (clean.as_bytes()[start - 1] == b'.' || clean.as_bytes()[start - 1] == b'@')
        {
            continue;
        }
        if keywords.contains(name)
            || BUILTIN_UNIFORMS.contains(&name)
            || locals.contains(name)
            || name.ends_with("Sampler")
            || name.ends_with("_sampler")
            || name.chars().next().is_some_and(char::is_uppercase)
            || seen.contains(name)
        {
            continue;
        }
        seen.insert(name.to_owned());
        result
            .inputs
            .push(port(name, DataType::Float, PortDirection::Input, None));
    }

    if result.outputs.is_empty() {
        let output_re =
            Regex::new(r"->\s*@location\s*\(\s*\d+\s*\)\s*([\w<>]+)").expect("valid output regex");
        if let Some(captures) = output_re.captures(&clean) {
            result.outputs.push(port(
                "fragColor",
                map_type_name(&captures[1]),
                PortDirection::Output,
                None,
            ));
        }
    }

    for input in &mut result.inputs {
        if input.description.is_none() {
            input.description = descriptions.get(&input.label).cloned();
        }
    }
}

fn port(
    label: &str,
    data_type: DataType,
    direction: PortDirection,
    description: Option<String>,
) -> Port {
    Port {
        id: format!("port_{}", label),
        label: label.to_owned(),
        data_type,
        direction,
        default_value: None,
        description,
    }
}

fn map_type_name(name: &str) -> DataType {
    match name.trim() {
        "f32" | "f16" => DataType::Float,
        "i32" => DataType::Int,
        "u32" => DataType::Uint,
        "bool" => DataType::Bool,
        "vec2f" | "vec2" => DataType::Vec2,
        "vec3f" | "vec3" => DataType::Vec3,
        "vec4f" | "vec4" => DataType::Vec4,
        "vec2i" => DataType::Ivec2,
        "vec3i" => DataType::Ivec3,
        "vec4i" => DataType::Ivec4,
        "vec2u" => DataType::Uvec2,
        "vec3u" => DataType::Uvec3,
        "vec4u" => DataType::Uvec4,
        "mat2x2f" | "mat2x2" => DataType::Mat2,
        "mat3x3f" | "mat3x3" => DataType::Mat3,
        _ => DataType::Float,
    }
}

fn extract_comments(code: &str) -> (Option<String>, HashMap<String, String>) {
    let mut description_lines = Vec::new();
    for line in code.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !description_lines.is_empty() {
                break;
            }
            continue;
        }
        if let Some(comment) = trimmed.strip_prefix("//") {
            if description_lines.is_empty() || !description_lines.is_empty() {
                description_lines.push(comment.trim().to_owned());
            }
        } else {
            break;
        }
    }

    let mut descriptions = HashMap::new();
    let var_re = Regex::new(r"var(?:<\w+>)?\s+(\w+)\s*:[^\n]*//\s*(.+)")
        .expect("valid variable comment regex");
    for captures in var_re.captures_iter(code) {
        descriptions.insert(captures[1].to_owned(), captures[2].trim().to_owned());
    }
    let description = (!description_lines.is_empty()).then(|| description_lines.join(" "));
    (description, descriptions)
}

fn strip_comments(code: &str) -> String {
    let line_re = Regex::new(r"//[^\n]*").expect("valid line comment regex");
    let block_re = Regex::new(r"/\*[\s\S]*?\*/").expect("valid block comment regex");
    block_re
        .replace_all(&line_re.replace_all(code, ""), "")
        .into_owned()
}

fn local_names(code: &str) -> HashSet<String> {
    let mut names = HashSet::new();
    let local_re = Regex::new(r"(?:let|var|const|fn)\s+(\w+)").expect("valid local regex");
    for captures in local_re.captures_iter(code) {
        names.insert(captures[1].to_owned());
    }
    let parameter_re =
        Regex::new(r"(?:@\w+(?:\(\d+\))?\s+)*(\w+)\s*:\s*[\w<>]+").expect("valid parameter regex");
    for captures in parameter_re.captures_iter(code) {
        names.insert(captures[1].to_owned());
    }
    names
}

fn keyword_names() -> HashSet<&'static str> {
    [
        "fn",
        "let",
        "var",
        "return",
        "if",
        "else",
        "for",
        "while",
        "loop",
        "break",
        "continue",
        "switch",
        "case",
        "default",
        "struct",
        "true",
        "false",
        "discard",
        "main",
        "const",
        "override",
        "enable",
        "diagnostic",
        "alias",
        "continuing",
        "fallthrough",
        "fragment",
        "vertex",
        "compute",
        "location",
        "group",
        "binding",
        "builtin",
        "workgroup_size",
        "align",
        "size",
        "interpolate",
        "invariant",
        "id",
        "must_use",
        "textureSample",
        "textureDimensions",
        "uniform",
        "f32",
        "i32",
        "u32",
        "bool",
        "vec2f",
        "vec3f",
        "vec4f",
        "vec2i",
        "vec3i",
        "vec4i",
        "vec2u",
        "vec3u",
        "vec4u",
        "abs",
        "clamp",
        "cos",
        "dot",
        "floor",
        "fract",
        "length",
        "max",
        "min",
        "mix",
        "normalize",
        "pow",
        "sin",
        "smoothstep",
        "sqrt",
        "step",
        "textureLoad",
        "textureStore",
    ]
    .into_iter()
    .collect()
}
