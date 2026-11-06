use super::*;
use std::collections::BTreeMap;

#[test]
fn compiler_helpers_cover_all_bindings_types_defaults_and_diagnostics() {
    let types = [
        ("float", "f32"),
        ("int", "i32"),
        ("uint", "u32"),
        ("bool", "u32"),
        ("vec2", "vec2f"),
        ("vec3", "vec3f"),
        ("vec4", "vec4f"),
        ("ivec2", "vec2i"),
        ("ivec3", "vec3i"),
        ("ivec4", "vec4i"),
        ("uvec2", "vec2u"),
        ("uvec3", "vec3u"),
        ("uvec4", "vec4u"),
        ("mat2", "mat2x2f"),
        ("mat3", "mat3x3f"),
        ("mat4", "mat4x4f"),
        ("unknown", "f32"),
    ];
    for (source, target) in types {
        assert_eq!(data_type_to_wgsl(source), target);
    }
    assert!(is_sampler("sampler2D"));
    assert!(is_sampler("samplerCube"));
    assert!(!is_sampler("float"));
    assert_eq!(default_target_format(), "rgba8unorm");

    let mut preamble = String::new();
    let mut bindings = Vec::new();
    for (index, (kind, ty)) in [
        ("texture", Some("texture_2d<f32>")),
        ("externalTexture", None),
        ("uniform", Some("f32")),
        ("sampler", None),
        ("unknown", None),
    ]
    .into_iter()
    .enumerate()
    {
        push_binding(
            &mut preamble,
            &mut bindings,
            index as u32,
            kind,
            &format!("v{index}"),
            ty,
        );
    }
    assert_eq!(bindings.len(), 5);
    assert!(preamble.contains("texture_external"));
    assert_eq!(first_error_position("error at 12:7", 2), (10, 7));
    assert_eq!(first_error_position("error at bad:bad", 2), (1, 0));
    assert_eq!(first_error_position("no position", 2), (1, 0));
    let long = "x".repeat(500);
    assert!(compact_diagnostic(&long).ends_with('…'));
    assert_eq!(compact_diagnostic(" short   message "), "short message");
}

#[test]
fn compiler_covers_unconnected_video_unknown_bindings_and_default_request_fields() {
    let request: CompileRequest = serde_json::from_str(
        r#"{"userCode":"@fragment fn main() -> @location(0) vec4f { return vec4f(flag); }","inputPorts":[{"label":"flag","dataType":"bool"}],"upstreamMap":{},"videoInputs":[]}"#,
    )
    .unwrap();
    assert_eq!(request.target_format, "rgba8unorm");
    let compiled = compile_shader(&request);
    assert!(compiled.full_fragment_code.contains("flag: u32"));

    let stripped = strip_user_bindings(
        "@group(0) @binding(0) var value: texture_external;\n@group(0) @binding(1) var samp: sampler;\n@fragment fn main() -> @location(0) vec4f { return vec4f(1.0); }",
    );
    assert!(!stripped.contains("@binding"));

    let request = CompileRequest {
        user_code: "@fragment fn main() -> @location(0) vec4f { return vec4f(1.0); }".to_owned(),
        input_ports: vec![CompilePort {
            label: "image".to_owned(),
            data_type: "samplerCube".to_owned(),
        }],
        upstream_map: BTreeMap::from([("missing".to_owned(), "node".to_owned())]),
        video_inputs: vec!["missing".to_owned()],
        target_format: "rgba8unorm".to_owned(),
    };
    let compiled = compile_shader(&request);
    assert!(compiled.bindings.is_empty());
    let errors = validate_shader("@fragment fn main( ->", 0);
    assert!(!errors.is_empty());
}
