use std::collections::BTreeMap;

use naga::front::wgsl;
use serde::{Deserialize, Serialize};

use crate::gpu::FULLSCREEN_VERT_WITH_UV;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompileRequest {
    pub user_code: String,
    pub input_ports: Vec<CompilePort>,
    #[serde(default)]
    pub upstream_map: BTreeMap<String, String>,
    #[serde(default)]
    pub video_inputs: Vec<String>,
    #[serde(default = "default_target_format")]
    pub target_format: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompilePort {
    pub label: String,
    pub data_type: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BindingDescriptor {
    pub binding: u32,
    pub kind: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wgsl_type: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompiledShader {
    pub full_fragment_code: String,
    pub preamble: String,
    pub preamble_lines: u32,
    pub bindings: Vec<BindingDescriptor>,
    pub upstream_samplers: BTreeMap<String, String>,
    pub texture_bindings: BTreeMap<String, u32>,
    pub external_texture_bindings: BTreeMap<String, u32>,
    pub uniform_bindings: BTreeMap<String, u32>,
    pub previous_frame_binding: Option<u32>,
    pub needs_feedback: bool,
    pub target_format: String,
    pub vertex_shader: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WgslCompilationError {
    pub message: String,
    pub line: u32,
    pub column: u32,
    pub offset: u32,
    pub length: u32,
}

pub fn compile_shader(request: &CompileRequest) -> CompiledShader {
    let mut binding = 0;
    let mut preamble = String::new();
    let mut bindings = Vec::new();
    let mut upstream_samplers = BTreeMap::new();
    let mut texture_bindings = BTreeMap::new();
    let mut external_texture_bindings = BTreeMap::new();
    let mut uniform_bindings = BTreeMap::new();
    let mut processed_code = strip_user_bindings(&request.user_code);
    let video_inputs = request
        .video_inputs
        .iter()
        .collect::<std::collections::BTreeSet<_>>();

    for (uniform_name, source_node_id) in &request.upstream_map {
        let Some(port) = request
            .input_ports
            .iter()
            .find(|port| port.label == *uniform_name)
        else {
            continue;
        };
        if is_sampler(&port.data_type) {
            upstream_samplers.insert(uniform_name.clone(), source_node_id.clone());
            if video_inputs.contains(uniform_name) {
                push_binding(
                    &mut preamble,
                    &mut bindings,
                    binding,
                    "externalTexture",
                    uniform_name,
                    None,
                );
                external_texture_bindings.insert(uniform_name.clone(), binding);
                binding += 1;
                let sampler_name = format!("{uniform_name}Sampler");
                push_binding(
                    &mut preamble,
                    &mut bindings,
                    binding,
                    "sampler",
                    &sampler_name,
                    None,
                );
                binding += 1;
                let sample = format!("textureSample({uniform_name},");
                let replacement = format!("textureSampleBaseClampToEdge({uniform_name},");
                processed_code = processed_code.replace(&sample, &replacement);
            } else {
                push_binding(
                    &mut preamble,
                    &mut bindings,
                    binding,
                    "texture",
                    uniform_name,
                    Some("texture_2d<f32>"),
                );
                texture_bindings.insert(uniform_name.clone(), binding);
                binding += 1;
                let sampler_name = format!("{uniform_name}Sampler");
                push_binding(
                    &mut preamble,
                    &mut bindings,
                    binding,
                    "sampler",
                    &sampler_name,
                    None,
                );
                binding += 1;
            }
        } else {
            let wgsl_type = data_type_to_wgsl(&port.data_type);
            push_binding(
                &mut preamble,
                &mut bindings,
                binding,
                "uniform",
                uniform_name,
                Some(&wgsl_type),
            );
            uniform_bindings.insert(uniform_name.clone(), binding);
            binding += 1;
        }
    }

    for port in &request.input_ports {
        if request.upstream_map.contains_key(&port.label) || is_sampler(&port.data_type) {
            continue;
        }
        let wgsl_type = data_type_to_wgsl(&port.data_type);
        push_binding(
            &mut preamble,
            &mut bindings,
            binding,
            "uniform",
            &port.label,
            Some(&wgsl_type),
        );
        uniform_bindings.insert(port.label.clone(), binding);
        binding += 1;
    }

    let needs_feedback = request
        .user_code
        .split_whitespace()
        .any(|word| word == "previousFrame" || word.contains("previousFrame"));
    let previous_frame_binding = if needs_feedback {
        push_binding(
            &mut preamble,
            &mut bindings,
            binding,
            "texture",
            "previousFrame",
            Some("texture_2d<f32>"),
        );
        let texture_binding = binding;
        binding += 1;
        push_binding(
            &mut preamble,
            &mut bindings,
            binding,
            "sampler",
            "previousFrameSampler",
            None,
        );
        Some(texture_binding)
    } else {
        None
    };

    let preamble_lines = preamble.lines().count() as u32;
    let full_fragment_code = format!("{preamble}{processed_code}");
    CompiledShader {
        full_fragment_code,
        preamble,
        preamble_lines,
        bindings,
        upstream_samplers,
        texture_bindings,
        external_texture_bindings,
        uniform_bindings,
        previous_frame_binding,
        needs_feedback,
        target_format: request.target_format.clone(),
        vertex_shader: FULLSCREEN_VERT_WITH_UV.to_owned(),
    }
}

pub fn validate_shader(code: &str, preamble_lines: u32) -> Vec<WgslCompilationError> {
    match wgsl::parse_str(code) {
        Ok(_) => Vec::new(),
        Err(error) => {
            let message = error.emit_to_string(code);
            let (line, column) = first_error_position(&message, preamble_lines);
            vec![WgslCompilationError {
                message,
                line,
                column,
                offset: 0,
                length: 0,
            }]
        }
    }
}

fn push_binding(
    preamble: &mut String,
    bindings: &mut Vec<BindingDescriptor>,
    binding: u32,
    kind: &str,
    name: &str,
    wgsl_type: Option<&str>,
) {
    let declaration = match (kind, wgsl_type) {
        ("texture", Some(ty)) => format!("@group(0) @binding({binding}) var {name}: {ty};\n"),
        ("externalTexture", _) => {
            format!("@group(0) @binding({binding}) var {name}: texture_external;\n")
        }
        ("uniform", Some(ty)) => {
            format!("@group(0) @binding({binding}) var<uniform> {name}: {ty};\n")
        }
        ("sampler", _) => format!("@group(0) @binding({binding}) var {name}: sampler;\n"),
        _ => String::new(),
    };
    preamble.push_str(&declaration);
    bindings.push(BindingDescriptor {
        binding,
        kind: kind.to_owned(),
        name: name.to_owned(),
        wgsl_type: wgsl_type.map(str::to_owned),
    });
}

fn strip_user_bindings(code: &str) -> String {
    let patterns = [
        r"@group\s*\(\s*\d+\s*\)\s*@binding\s*\(\s*\d+\s*\)\s*var\s+\w+\s*:\s*texture_2d\s*<\s*f32\s*>\s*;",
        r"@group\s*\(\s*\d+\s*\)\s*@binding\s*\(\s*\d+\s*\)\s*var\s+\w+\s*:\s*texture_external\s*;",
        r"@group\s*\(\s*\d+\s*\)\s*@binding\s*\(\s*\d+\s*\)\s*var\s+\w+\s*:\s*sampler\s*;",
        r"@group\s*\(\s*\d+\s*\)\s*@binding\s*\(\s*\d+\s*\)\s*var\s*<\s*uniform\s*>\s*\w+\s*:\s*[\w<>]+\s*;",
    ];
    patterns.iter().fold(code.to_owned(), |result, pattern| {
        regex::Regex::new(pattern)
            .expect("valid WGSL binding regex")
            .replace_all(&result, "")
            .into_owned()
    })
}

fn is_sampler(data_type: &str) -> bool {
    data_type == "sampler2D" || data_type == "samplerCube"
}

fn data_type_to_wgsl(data_type: &str) -> String {
    match data_type {
        "float" => "f32",
        "int" => "i32",
        "uint" => "u32",
        "bool" => "u32",
        "vec2" => "vec2f",
        "vec3" => "vec3f",
        "vec4" => "vec4f",
        "ivec2" => "vec2i",
        "ivec3" => "vec3i",
        "ivec4" => "vec4i",
        "uvec2" => "vec2u",
        "uvec3" => "vec3u",
        "uvec4" => "vec4u",
        "mat2" => "mat2x2f",
        "mat3" => "mat3x3f",
        "mat4" => "mat4x4f",
        _ => "f32",
    }
    .to_owned()
}

fn first_error_position(message: &str, preamble_lines: u32) -> (u32, u32) {
    let mut line = 1;
    let mut column = 0;
    for token in message.split_whitespace() {
        if let Some((line_text, column_text)) = token.trim_matches(['(', ')', ':']).split_once(':')
        {
            if let (Ok(raw_line), Ok(raw_column)) =
                (line_text.parse::<u32>(), column_text.parse::<u32>())
            {
                line = raw_line.saturating_sub(preamble_lines).max(1);
                column = raw_column;
                break;
            }
        }
    }
    (line, column)
}

fn default_target_format() -> String {
    "rgba8unorm".to_owned()
}
