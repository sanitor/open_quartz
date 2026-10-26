use super::*;

#[test]
fn parser_maps_every_scalar_vector_matrix_and_fallback_type() {
    let cases = [
        ("i32", DataType::Int),
        ("u32", DataType::Uint),
        ("bool", DataType::Bool),
        ("f32", DataType::Float),
        ("vec2i", DataType::Ivec2),
        ("vec3i", DataType::Ivec3),
        ("vec4i", DataType::Ivec4),
        ("vec2u", DataType::Uvec2),
        ("vec3u", DataType::Uvec3),
        ("vec4u", DataType::Uvec4),
        ("vec2f", DataType::Vec2),
        ("vec3f", DataType::Vec3),
        ("vec4f", DataType::Vec4),
        ("mat2x2f", DataType::Mat2),
        ("mat3x3f", DataType::Mat3),
        ("mat4x4f", DataType::Float),
        ("unknown", DataType::Float),
    ];
    for (name, expected) in cases {
        assert_eq!(map_type_name(name), expected);
    }
}

#[test]
fn parser_handles_unnamed_builtin_duplicate_nonuniform_and_no_fragment_result() {
    let source = r#"
struct Values { x: f32 }
@group(0) @binding(0) var<storage, read> storageValue: Values;
@group(0) @binding(1) var image: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(2) var sampled: texture_2d<f32>;
@group(0) @binding(3) var sampledSampler: sampler;
@group(0) @binding(4) var<uniform> iValue: i32;
@group(0) @binding(5) var<uniform> uValue: u32;
@group(0) @binding(6) var<uniform> bValue: bool;
@group(0) @binding(7) var<uniform> vi2: vec2i;
@group(0) @binding(8) var<uniform> vu3: vec3u;
@group(0) @binding(9) var<uniform> matrix: mat3x3f;
@compute @workgroup_size(1) fn compute_main() {}
@fragment fn no_result() { }
"#;
    let parsed = parse_shader(source);
    assert!(parsed.inputs.iter().any(|port| port.label == "sampled"));
    assert!(parsed
        .inputs
        .iter()
        .any(|port| port.label == "iValue" && port.data_type == DataType::Int));
    assert!(parsed
        .inputs
        .iter()
        .any(|port| port.label == "uValue" && port.data_type == DataType::Uint));
    assert!(parsed
        .inputs
        .iter()
        .any(|port| port.label == "bValue" && port.data_type == DataType::Bool));
    assert!(parsed
        .inputs
        .iter()
        .any(|port| port.label == "vi2" && port.data_type == DataType::Ivec2));
    assert!(parsed
        .inputs
        .iter()
        .any(|port| port.label == "vu3" && port.data_type == DataType::Uvec3));
    assert!(parsed
        .inputs
        .iter()
        .any(|port| port.label == "matrix" && port.data_type == DataType::Mat3));
    assert!(parsed.outputs.is_empty());
}

#[test]
fn fallback_comments_locals_and_keywords_cover_skip_paths() {
    let source = r#"
// Shader description
// second line
@group(0) @binding(0) var image: texture_2d<f32>; // image description
@group(0) @binding(1) var imageSampler: sampler;
@group(0) @binding(2) var<uniform> vec: vec4f;
@fragment fn main() -> @location(0) vec4f {
  let localValue = vec.x;
  return textureSample(image, imageSampler, vec2f(localValue));
}
"#;
    let parsed = parse_shader(source);
    assert!(parsed
        .description
        .as_deref()
        .unwrap()
        .contains("Shader description"));
    assert!(parsed.inputs.iter().any(|port| port.label == "image"));
    let (description, comments) = extract_comments(source);
    assert!(description.is_some());
    assert!(!comments.is_empty());
    assert!(!strip_comments(source).contains("description"));
    assert!(local_names(source).contains("localValue"));
    assert!(keyword_names().contains("textureSample"));

    let invalid = r#"
@group(0) @binding(0) var fallbackTex: texture_2d<f32>;
@group(0) @binding(1) var<uniform> fallbackI: i32;
@fragment fn broken( { return vec4f(1.0); }
"#;
    let fallback = parse_shader(invalid);
    assert!(fallback.parse_error.is_some());
    assert!(fallback
        .inputs
        .iter()
        .any(|port| port.label == "fallbackTex"));
    assert!(fallback.inputs.iter().any(|port| port.label == "fallbackI"));
}

#[test]
fn naga_type_mapping_covers_every_vector_matrix_and_default_variant() {
    use naga::{Scalar, ScalarKind, VectorSize};
    let scalar = |kind| Scalar { kind, width: 4 };
    for (size, kind, expected) in [
        (VectorSize::Bi, ScalarKind::Sint, DataType::Ivec2),
        (VectorSize::Tri, ScalarKind::Sint, DataType::Ivec3),
        (VectorSize::Quad, ScalarKind::Sint, DataType::Ivec4),
        (VectorSize::Bi, ScalarKind::Uint, DataType::Uvec2),
        (VectorSize::Tri, ScalarKind::Uint, DataType::Uvec3),
        (VectorSize::Quad, ScalarKind::Uint, DataType::Uvec4),
        (VectorSize::Bi, ScalarKind::Bool, DataType::Bvec2),
        (VectorSize::Tri, ScalarKind::Bool, DataType::Bvec3),
        (VectorSize::Quad, ScalarKind::Bool, DataType::Bvec4),
        (VectorSize::Bi, ScalarKind::Float, DataType::Vec2),
        (VectorSize::Tri, ScalarKind::Float, DataType::Vec3),
        (VectorSize::Quad, ScalarKind::Float, DataType::Vec4),
    ] {
        assert_eq!(
            type_inner_to_data_type(&TypeInner::Vector {
                size,
                scalar: scalar(kind)
            }),
            expected
        );
    }
    for (columns, rows, expected) in [
        (VectorSize::Bi, VectorSize::Bi, DataType::Mat2),
        (VectorSize::Tri, VectorSize::Tri, DataType::Mat3),
        (VectorSize::Quad, VectorSize::Quad, DataType::Mat4),
    ] {
        assert_eq!(
            type_inner_to_data_type(&TypeInner::Matrix {
                columns,
                rows,
                scalar: scalar(ScalarKind::Float)
            }),
            expected
        );
    }
    assert_eq!(
        type_inner_to_data_type(&TypeInner::Atomic(scalar(ScalarKind::Uint))),
        DataType::Float
    );
}

#[test]
fn naga_and_fallback_extractors_skip_unnamed_builtin_duplicate_and_compute_only_entries() {
    let mut module = naga::front::wgsl::parse_str(
        "@group(0) @binding(0) var<uniform> first: f32;\n@group(0) @binding(1) var<uniform> second: f32;\n@compute @workgroup_size(1) fn main() {}",
    ).unwrap();
    let handles = module
        .global_variables
        .iter()
        .map(|(handle, _)| handle)
        .collect::<Vec<_>>();
    module.global_variables[handles[0]].name = None;
    module.global_variables[handles[1]].name = Some("iTime".to_owned());
    let mut result = parse_shader("");
    extract_naga_bindings(&module, &HashMap::new(), &mut result);
    extract_naga_outputs(&module, &mut result);
    assert!(result.inputs.is_empty());
    assert!(result.outputs.is_empty());

    fallback_extract(
        "@group(0) @binding(0) var repeated: f32;\n@group(0) @binding(1) var repeated: f32;\n@group(0) @binding(2) var iTime: f32;",
        &HashMap::new(),
        &mut result,
    );
    assert_eq!(
        result
            .inputs
            .iter()
            .filter(|port| port.label == "repeated")
            .count(),
        1
    );
    assert!(!result.inputs.iter().any(|port| port.label == "iTime"));
    let descriptions = HashMap::from([("described".to_owned(), "port docs".to_owned())]);
    fallback_extract(
        "@group(0) @binding(0) var described: f32;",
        &descriptions,
        &mut result,
    );
    assert_eq!(
        result
            .inputs
            .iter()
            .find(|port| port.label == "described")
            .unwrap()
            .description
            .as_deref(),
        Some("port docs")
    );
    assert_eq!(
        extract_comments("// description\n\ncode").0.as_deref(),
        Some("description")
    );
}
