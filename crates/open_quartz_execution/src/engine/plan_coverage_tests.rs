use super::*;
use serde_json::json;

fn node(id: &str, kind: &str) -> ProjectNode {
    serde_json::from_value(json!({
        "id": id, "type": kind, "position": {"x": 0.0, "y": 0.0},
        "data": {"type": kind, "label": id, "shaderCode": if kind == "shader" || kind == "constant" { "@fragment fn main() -> @location(0) vec4f { return vec4f(1.0); }" } else { "" }, "inputs": [], "outputs": [], "uniforms": {}}
    })).unwrap()
}

#[test]
fn plan_helpers_cover_defaults_targets_all_types_and_public_builder() {
    let types = [
        (DataType::Float, "float"),
        (DataType::Int, "int"),
        (DataType::Uint, "uint"),
        (DataType::Bool, "bool"),
        (DataType::Vec2, "vec2"),
        (DataType::Vec3, "vec3"),
        (DataType::Vec4, "vec4"),
        (DataType::Ivec2, "ivec2"),
        (DataType::Ivec3, "ivec3"),
        (DataType::Ivec4, "ivec4"),
        (DataType::Uvec2, "uvec2"),
        (DataType::Uvec3, "uvec3"),
        (DataType::Uvec4, "uvec4"),
        (DataType::Bvec2, "bvec2"),
        (DataType::Bvec3, "bvec3"),
        (DataType::Bvec4, "bvec4"),
        (DataType::Mat2, "mat2"),
        (DataType::Mat3, "mat3"),
        (DataType::Mat4, "mat4"),
        (DataType::Sampler2d, "sampler2D"),
        (DataType::SamplerCube, "samplerCube"),
        (DataType::Roi, "roi"),
        (DataType::Mesh, "mesh"),
        (DataType::Json, "json"),
        (DataType::Auto, "auto"),
    ];
    for (ty, name) in types {
        assert_eq!(data_type_name(ty), name);
    }

    assert_eq!(default_size(&[]), (512, 512));
    let mut framebuffer = node("fb", "input");
    framebuffer.data.input_mode = Some(InputMode::Framebuffer);
    framebuffer.data.fb_width = Some(320);
    let mut incomplete_framebuffer = node("incomplete-fb", "input");
    incomplete_framebuffer.data.input_mode = Some(InputMode::Framebuffer);
    incomplete_framebuffer.data.fb_width = Some(320);
    assert_eq!(default_size(&[incomplete_framebuffer]), (512, 512));
    framebuffer.data.fb_height = Some(200);
    assert_eq!(default_size(&[framebuffer]), (320, 200));
    let mut image = node("image", "input");
    image.data.image_width = Some(11);
    image.data.image_height = Some(12);
    assert_eq!(default_size(&[image]), (11, 12));

    let mut shader = node("shader", "shader");
    shader.data.auto_size = Some(false);
    shader.data.width = Some(7);
    shader.data.height = Some(8);
    shader.data.out_format = Some(open_quartz_schema::FramebufferFormat::Rgba32f);
    let target = target_for(&shader, 1, 1).unwrap();
    assert_eq!((target.width, target.height, target.float), (7, 8, true));
    shader.data.out_format = Some(open_quartz_schema::FramebufferFormat::Rg32f);
    assert!(target_for(&shader, 1, 1).unwrap().float);
    shader.data.out_format = Some(open_quartz_schema::FramebufferFormat::R32f);
    assert!(target_for(&shader, 1, 1).unwrap().float);
    assert!(target_for(&node("math", "math"), 1, 1).is_none());

    let plan = build_execution_plan(&[shader], &[]);
    assert_eq!(plan.output_nodes, ["shader"]);
}

#[test]
fn plan_covers_builtin_video_renderer_cycle_and_unmatched_edge_paths() {
    let mut video = node("video", "input");
    video.data.input_mode = Some(InputMode::Video);
    let mut shader = node("shader", "shader");
    shader.data.inputs = serde_json::from_value(json!([
        {"id":"time","label":"iTime","dataType":"float","direction":"input"},
        {"id":"image","label":"image","dataType":"sampler2D","direction":"input"}
    ]))
    .unwrap();
    shader.data.shader_code =
        "@fragment fn main() -> @location(0) vec4f { return vec4f(iTime); }".to_owned();
    let renderer = node("renderer", "renderer");
    let edges: Vec<Edge> = serde_json::from_value(json!([
        {"id":"video-shader","source":"video","sourceHandle":"out","target":"shader","targetHandle":"image"},
        {"id":"ignored","source":"video","sourceHandle":"out","target":"shader","targetHandle":"missing"},
        {"id":"shader-renderer","source":"shader","sourceHandle":"out","target":"renderer","targetHandle":"missing"}
    ])).unwrap();
    let plan = build_execution_plan_with_options(&[video, shader, renderer], &edges, true);
    let shader = plan.nodes.iter().find(|item| item.id == "shader").unwrap();
    assert_eq!(shader.builtin_ports, ["iTime"]);
    assert!(shader
        .shader
        .as_ref()
        .unwrap()
        .external_texture_bindings
        .contains_key("image"));
    assert_eq!(plan.output_nodes, ["renderer"]);

    let cycle_edges: Vec<Edge> = serde_json::from_value(json!([
        {"id":"ab","source":"a","sourceHandle":"out","target":"b","targetHandle":"in"},
        {"id":"ba","source":"b","sourceHandle":"out","target":"a","targetHandle":"in"}
    ]))
    .unwrap();
    assert!(build_execution_plan(&[node("a", "math"), node("b", "math")], &cycle_edges).cycle);

    let order = topological_sort(
        &[GraphNode {
            id: "known".to_owned(),
        }],
        &[GraphEdge {
            source: "known".to_owned(),
            target: "missing".to_owned(),
        }],
    );
    assert_eq!(order, ["known"]);
}
