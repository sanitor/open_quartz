use open_quartz_execution::wgsl::parse_shader;

#[test]
fn extracts_texture_uniform_and_fragment_output() {
    let code = r#"
@group(0) @binding(0) var inputImage: texture_2d<f32>;
@group(0) @binding(1) var inputImageSampler: sampler;
@group(0) @binding(2) var<uniform> intensity: f32;
@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
    return vec4f(textureSample(inputImage, inputImageSampler, v_uv).rgb * intensity, 1.0);
}
"#;

    let parsed = parse_shader(code);
    assert!(parsed.parse_error.is_none());
    assert_eq!(parsed.inputs.len(), 2);
    assert_eq!(parsed.inputs[0].label, "inputImage");
    assert_eq!(
        parsed.inputs[0].data_type,
        open_quartz_schema::DataType::Sampler2d
    );
    assert_eq!(parsed.inputs[1].label, "intensity");
    assert_eq!(
        parsed.inputs[1].data_type,
        open_quartz_schema::DataType::Float
    );
    assert_eq!(parsed.outputs.len(), 1);
    assert_eq!(parsed.outputs[0].label, "fragColor");
    assert_eq!(
        parsed.outputs[0].data_type,
        open_quartz_schema::DataType::Vec4
    );
}

#[test]
fn extracts_fallback_texture_and_uniform_without_bindings() {
    let parsed = parse_shader(
        r#"@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
            let color = textureSample(inputImage, inputImageSampler, v_uv);
            return vec4f(color.rgb * intensity, color.a);
        }"#,
    );

    assert!(parsed.parse_error.is_none());
    assert_eq!(parsed.inputs.len(), 2);
    assert_eq!(parsed.inputs[0].label, "inputImage");
    assert_eq!(parsed.inputs[1].label, "intensity");
    assert_eq!(
        parsed.outputs[0].data_type,
        open_quartz_schema::DataType::Vec4
    );
}

#[test]
fn filters_builtin_feedback_texture_and_attributes() {
    let parsed = parse_shader(
        r#"@doraemon fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
            let color = textureSample(previousFrame, previousFrameSampler, v_uv);
            return color;
        }"#,
    );

    assert!(parsed.parse_error.is_none());
    assert!(parsed.inputs.is_empty());
}

#[test]
fn fallback_preserves_declared_bindings_when_builtin_injection_is_missing() {
    let parsed = parse_shader(
        r#"@group(0) @binding(0) var<uniform> d_a: f32;
@group(0) @binding(1) var<uniform> d_b: f32;
@fragment fn main(@location(0) uv: vec2f) -> @location(0) vec4f {
    let size = textureDimensions(previousFrame);
    return vec4f(d_a + d_b + f32(size.x) + uv.x);
}"#,
    );

    assert!(parsed.parse_error.is_none());
    assert_eq!(
        parsed
            .inputs
            .iter()
            .map(|port| port.label.as_str())
            .collect::<Vec<_>>(),
        ["d_a", "d_b"]
    );
    assert!(parsed
        .inputs
        .iter()
        .all(|port| port.data_type == open_quartz_schema::DataType::Float));
}

#[test]
fn returns_parse_error_for_invalid_wgsl() {
    let parsed = parse_shader("@fragment fn main(broken {{{");
    assert!(parsed
        .parse_error
        .as_deref()
        .is_some_and(|error| !error.is_empty()));
}

#[test]
fn extracts_shader_and_port_comments() {
    let parsed = parse_shader(
        r#"// Adjusts brightness.
@group(0) @binding(0) var<uniform> brightness: f32; // Additive offset. -1 to 1
@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
    return vec4f(brightness);
}"#,
    );

    assert_eq!(parsed.description.as_deref(), Some("Adjusts brightness."));
    assert_eq!(
        parsed.inputs[0].description.as_deref(),
        Some("Additive offset. -1 to 1")
    );
}

#[test]
fn serializes_typescript_wire_names() {
    let parsed = parse_shader(
        "@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f { return vec4f(1.0); }",
    );
    let json = serde_json::to_value(parsed).unwrap();
    assert_eq!(json["outputs"][0]["dataType"], "vec4");
    assert_eq!(json["outputs"][0]["direction"], "output");
    assert!(json["raw"].as_str().unwrap().contains("@fragment"));
}
