use open_quartz_execution::wgsl::{compile_shader, validate_shader, CompilePort, CompileRequest};
use std::collections::BTreeMap;

fn request(user_code: &str, ports: &[(&str, &str)]) -> CompileRequest {
    CompileRequest {
        user_code: user_code.to_owned(),
        input_ports: ports
            .iter()
            .map(|(label, data_type)| CompilePort {
                label: (*label).to_owned(),
                data_type: (*data_type).to_owned(),
            })
            .collect(),
        upstream_map: BTreeMap::new(),
        video_inputs: Vec::new(),
        target_format: "rgba8unorm".to_owned(),
    }
}

#[test]
fn injects_uniform_and_texture_bindings_in_upstream_order() {
    let mut request = request(
        "@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f { return vec4f(intensity); }",
        &[("inputImage", "sampler2D"), ("intensity", "float")],
    );
    request
        .upstream_map
        .insert("inputImage".to_owned(), "image_1".to_owned());
    request
        .upstream_map
        .insert("intensity".to_owned(), "uniform_1".to_owned());

    let compiled = compile_shader(&request);
    assert_eq!(compiled.texture_bindings.get("inputImage"), Some(&0));
    assert_eq!(compiled.uniform_bindings.get("intensity"), Some(&2));
    assert_eq!(compiled.upstream_samplers["inputImage"], "image_1");
    assert!(compiled
        .full_fragment_code
        .contains("var inputImage: texture_2d<f32>"));
    assert!(compiled
        .full_fragment_code
        .contains("var<uniform> intensity: f32"));
    assert_eq!(compiled.preamble_lines, 3);
}

#[test]
fn injects_disconnected_uniforms_but_not_disconnected_samplers() {
    let request = request(
        "@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f { return vec4f(gain); }",
        &[("inputImage", "sampler2D"), ("gain", "vec3")],
    );
    let compiled = compile_shader(&request);
    assert!(!compiled
        .full_fragment_code
        .contains("var inputImage: texture_2d"));
    assert_eq!(compiled.uniform_bindings.get("gain"), Some(&0));
    assert!(compiled
        .full_fragment_code
        .contains("var<uniform> gain: vec3f"));
}

#[test]
fn video_inputs_use_external_texture_and_rewrite_sampling() {
    let mut request = request(
        "@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f { return textureSample(inputImage, inputImageSampler, v_uv); }",
        &[("inputImage", "sampler2D")],
    );
    request
        .upstream_map
        .insert("inputImage".to_owned(), "video_1".to_owned());
    request.video_inputs.push("inputImage".to_owned());

    let compiled = compile_shader(&request);
    assert_eq!(
        compiled.external_texture_bindings.get("inputImage"),
        Some(&0)
    );
    assert!(compiled.texture_bindings.is_empty());
    assert!(compiled.full_fragment_code.contains("texture_external"));
    assert!(compiled
        .full_fragment_code
        .contains("textureSampleBaseClampToEdge(inputImage,"));
}

#[test]
fn feedback_adds_previous_frame_texture_and_sampler() {
    let request = request(
        "@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f { return textureSample(previousFrame, previousFrameSampler, v_uv); }",
        &[],
    );
    let compiled = compile_shader(&request);
    assert!(compiled.needs_feedback);
    assert_eq!(compiled.previous_frame_binding, Some(0));
    assert!(compiled
        .full_fragment_code
        .contains("var previousFrame: texture_2d<f32>"));
    assert!(compiled
        .full_fragment_code
        .contains("var previousFrameSampler: sampler"));
}

#[test]
fn strips_user_binding_declarations_before_injection() {
    let mut request = request(
        "@group(0) @binding(9) var inputImage: texture_2d<f32>;\n@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f { return textureSample(inputImage, inputImageSampler, v_uv); }",
        &[("inputImage", "sampler2D")],
    );
    request
        .upstream_map
        .insert("inputImage".to_owned(), "image_1".to_owned());
    let compiled = compile_shader(&request);
    assert!(!compiled.full_fragment_code.contains("@binding(9)"));
    assert!(compiled
        .full_fragment_code
        .contains("@binding(0) var inputImage"));
}

#[test]
fn validation_returns_no_errors_for_valid_source_and_error_for_invalid_source() {
    let valid = "@fragment fn main() -> @location(0) vec4f { return vec4f(1.0); }";
    assert!(validate_shader(valid, 0).is_empty());

    let invalid = validate_shader("@fragment fn main(broken {{{", 0);
    assert_eq!(invalid.len(), 1);
    assert!(!invalid[0].message.is_empty());
    assert!(invalid[0].line >= 1);
}
