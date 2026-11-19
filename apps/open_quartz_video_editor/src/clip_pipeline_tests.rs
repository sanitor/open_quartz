use super::*;

pub(super) const INVERT: &str = "@fragment fn main(@location(0) uv: vec2f) -> @location(0) vec4f { let color = textureSample(image, imageSampler, uv); return vec4f(vec3f(1.0) - color.rgb, color.a); }";

fn assert_rejected(pipeline: &mut ClipPipeline, graph: Graph) {
    let before = pipeline.clone();
    assert!(pipeline.replace_graph(graph).is_err());
    assert_eq!(*pipeline, before);
}

#[test]
fn default_is_a_real_video_to_image_graph_and_roundtrips() {
    let pipeline = ClipPipeline::default();
    pipeline.validate().unwrap();
    assert_eq!(pipeline.graph().nodes.len(), 2);
    assert_eq!(pipeline.graph().edges.len(), 1);
    let source = &pipeline.graph().nodes[0];
    assert_eq!(source.id, SOURCE_ID);
    assert_eq!(source.data.input_mode, Some(InputMode::Video));
    assert_eq!(source.data.outputs[0].data_type, DataType::Sampler2d);
    let output = &pipeline.graph().nodes[1];
    assert_eq!(output.id, OUTPUT_ID);
    assert_eq!(output.data.inputs[0].label, "image");
    assert_eq!(output.data.inputs[0].data_type, DataType::Sampler2d);
    assert_eq!(output.data.outputs[0].data_type, DataType::Vec4);
    let restored: ClipPipeline = serde_json::from_slice(&serde_json::to_vec(&pipeline).unwrap()).unwrap();
    assert_eq!(restored, pipeline);
    restored.validate().unwrap();
}

#[test]
fn serial_factory_ids_are_deterministic_and_removal_bridges_selected_chains() {
    let mut pipeline = ClipPipeline::default();
    let first = pipeline.add_serial(SOURCE_ID, INVERT, "Invert").unwrap();
    let second = pipeline.add_serial(OUTPUT_ID, INVERT, "Invert").unwrap();
    assert_eq!(first, "shader_1");
    assert_eq!(second, "shader_2");
    assert!(pipeline.graph().edges.iter().any(|edge| edge.source == first && edge.target == second));
    let mut same = ClipPipeline::default();
    assert_eq!(same.add_serial(SOURCE_ID, INVERT, "Invert").unwrap(), first);
    assert_eq!(same.add_serial(OUTPUT_ID, INVERT, "Invert").unwrap(), second);
    assert_eq!(same, pipeline);
    pipeline.remove_nodes(&[second, first]).unwrap();
    assert_eq!(pipeline, ClipPipeline::default());
}

#[test]
fn protected_nodes_allow_layout_only_and_failed_commands_are_atomic() {
    let mut pipeline = ClipPipeline::default();
    for id in [SOURCE_ID, OUTPUT_ID] {
        let before = pipeline.clone();
        assert!(ClipPipeline::is_protected(id));
        assert!(pipeline.apply_command(GraphCommand::RemoveNode { node_id: NodeId::new(id) }).is_err());
        assert_eq!(pipeline, before);
        let mut changed = pipeline.graph().nodes.iter().find(|node| node.id == id).unwrap().clone();
        changed.data.label = "Changed".to_owned();
        assert!(pipeline.apply_command(GraphCommand::UpdateNode { node: changed }).is_err());
        assert_eq!(pipeline, before);
        assert!(pipeline.apply_command(GraphCommand::UpdateShaderCode {
            node_id: NodeId::new(id), shader_code: INVERT.to_owned(),
        }).is_err());
        assert_eq!(pipeline, before);
        pipeline.apply_command(GraphCommand::SetNodePosition {
            node_id: NodeId::new(id), position: Position { x: 12.0, y: 34.0 },
        }).unwrap();
        assert_eq!(pipeline.graph().nodes.iter().find(|node| node.id == id).unwrap().position,
            Position { x: 12.0, y: 34.0 });
    }
    let before = pipeline.clone();
    let edge_id = pipeline.graph().edges[0].id.clone();
    assert!(pipeline.apply_command(GraphCommand::Disconnect { edge_id }).is_err());
    assert_eq!(pipeline, before);
}

#[test]
fn rejects_invalid_graph_structure_ports_resources_and_shader_code_without_mutation() {
    let mut pipeline = ClipPipeline::default();
    let id = pipeline.add_serial(SOURCE_ID, INVERT, "Invert").unwrap();
    let mut invalid = pipeline.graph().clone();
    invalid.nodes.push(invalid.nodes[0].clone());
    assert_rejected(&mut pipeline, invalid);
    let mut invalid = pipeline.graph().clone();
    invalid.edges[0].source_handle = "missing-port".into();
    assert_rejected(&mut pipeline, invalid);
    let mut invalid = pipeline.graph().clone();
    invalid.edges.push(open_quartz::project::Edge {
        id: "cycle".into(), source: id.clone(), source_handle: format!("{id}_fragColor"),
        target: id.clone(), target_handle: format!("{id}_image"),
    });
    assert_rejected(&mut pipeline, invalid);
    let mut invalid = pipeline.graph().clone();
    let node = invalid.nodes.iter_mut().find(|node| node.id == id).unwrap();
    node.data.inputs[0].data_type = DataType::Float;
    assert_rejected(&mut pipeline, invalid);
    let mut invalid = pipeline.graph().clone();
    invalid.nodes.iter_mut().find(|node| node.id == id).unwrap().data.image_data_url = Some("file:///hidden.png".into());
    assert_rejected(&mut pipeline, invalid);
    let mut invalid = pipeline.graph().clone();
    invalid.nodes.iter_mut().find(|node| node.id == id).unwrap().data.feedback_enabled = Some(true);
    assert_rejected(&mut pipeline, invalid);
    let before = pipeline.clone();
    assert!(pipeline.apply_command(GraphCommand::UpdateShaderCode {
        node_id: NodeId::new(&id), shader_code: "@fragment fn main( ->".into(),
    }).is_err());
    assert_eq!(pipeline, before);
    assert!(pipeline.remove_nodes(&["missing".into()]).is_err());
    assert_eq!(pipeline, before);
}

#[test]
fn temporal_and_interactive_shaders_are_not_accepted_as_static_effects() {
    let mut pipeline = ClipPipeline::default();
    for builtin in ["iTime", "iTimeDelta", "iFrame", "iDate.x", "iMouse.x"] {
        let code = format!("@fragment fn main(@location(0) uv: vec2f) -> @location(0) vec4f {{ return textureSample(image, imageSampler, uv) * {builtin}; }}");
        let before = pipeline.clone();
        assert!(pipeline.add_serial(SOURCE_ID, &code, "Not static").is_err(), "{builtin}");
        assert_eq!(pipeline, before);
    }
    let feedback = "@fragment fn main(@location(0) uv: vec2f) -> @location(0) vec4f { return textureSample(image, imageSampler, uv) + textureSample(previousFrame, previousFrameSampler, uv); }";
    assert!(pipeline.add_serial(SOURCE_ID, feedback, "Feedback").is_err());
    assert_eq!(pipeline, ClipPipeline::default());
}

#[test]
fn multi_image_merge_cannot_be_serially_inserted_or_ambiguously_removed() {
    let code = "@fragment fn main(@location(0) uv: vec2f) -> @location(0) vec4f { return textureSample(image, imageSampler, uv) + textureSample(other, otherSampler, uv); }";
    let mut pipeline = ClipPipeline::default();
    assert!(pipeline.add_serial(SOURCE_ID, code, "Merge").is_err());
    assert_eq!(pipeline, ClipPipeline::default());
    let mut factory = project(pipeline.graph().clone());
    let (merge, _) = factory.create_graph_node(NodeFactoryRequest::Shader {
        position: None, code: code.to_owned(), label: "Merge".to_owned(),
        template_name: None, shader_template_id: None,
    }, 0).unwrap();
    let mut graph = factory.to_file().graph;
    for input in &merge.data.inputs {
        graph.apply_command(GraphCommand::Connect {
            source: key(SOURCE_ID, "video_output"), target: key(&merge.id, &input.id),
        }).unwrap();
    }
    graph.apply_command(GraphCommand::Connect {
        source: key(&merge.id, &merge.data.outputs[0].id), target: key(OUTPUT_ID, "output_image"),
    }).unwrap();
    pipeline.replace_graph(graph).unwrap();
    let before = pipeline.clone();
    assert!(pipeline.remove_nodes(&[merge.id]).is_err());
    assert_eq!(pipeline, before);
}

#[test]
fn insertion_before_output_does_not_rewrite_other_fanout_branches() {
    let mut pipeline = ClipPipeline::default();
    let first = pipeline.add_serial(SOURCE_ID, INVERT, "Invert").unwrap();
    let branch = pipeline.add_serial(OUTPUT_ID, INVERT, "Branch").unwrap();
    let mut graph = pipeline.graph().clone();
    graph.apply_command(GraphCommand::Connect {
        source: key(SOURCE_ID, "video_output"), target: key(OUTPUT_ID, "output_image"),
    }).unwrap();
    pipeline.replace_graph(graph).unwrap();
    let inserted = pipeline.add_serial(OUTPUT_ID, INVERT, "Output only").unwrap();
    assert!(pipeline.graph().edges.iter().any(|edge| edge.source == SOURCE_ID && edge.target == first));
    assert!(pipeline.graph().edges.iter().any(|edge| edge.source == first && edge.target == branch));
    assert!(pipeline.graph().edges.iter().any(|edge| edge.source == SOURCE_ID && edge.target == inserted));
    assert!(pipeline.graph().edges.iter().any(|edge| edge.source == inserted && edge.target == OUTPUT_ID));
}

#[test]
fn free_bindings_invalid_wgsl_types_and_extra_shader_stages_are_rejected_atomically() {
    let mut pipeline = ClipPipeline::default();
    let free_resource = format!("@group(1) @binding(0) var<storage, read> hidden: array<f32>;\n{INVERT}");
    let invalid_type = INVERT.replace("vec3f(1.0) - color.rgb", "vec3f(1.0) - true");
    let extra_stage = format!("{INVERT}\n@compute @workgroup_size(1) fn hidden_compute() {{}}");
    for code in [free_resource, invalid_type, extra_stage] {
        let before = pipeline.clone();
        assert!(pipeline.add_serial(SOURCE_ID, &code, "Invalid").is_err());
        assert_eq!(pipeline, before);
    }
}

#[test]
fn only_clip_source_can_supply_an_external_resource_and_missing_texture_bindings_fail() {
    let mut pipeline = ClipPipeline::default();
    let mut factory = project(pipeline.graph().clone());
    let (input, _) = factory.create_graph_node(NodeFactoryRequest::Input {
        position: None, data_type: DataType::Sampler2d, input_mode: Some(InputMode::Image),
    }, 0).unwrap();
    assert!(pipeline.apply_command(GraphCommand::AddNode { node: input }).is_err());
    let mut source = pipeline.graph().nodes[0].clone();
    source.data.video_file_path = Some("/untracked/video.mp4".into());
    assert!(pipeline.apply_command(GraphCommand::UpdateNode { node: source }).is_err());
    let shader = pipeline.add_serial(SOURCE_ID, INVERT, "Invert").unwrap();
    let mut unbound = pipeline.graph().clone();
    unbound.edges.retain(|edge| edge.target != shader);
    assert_rejected(&mut pipeline, unbound);
    let mut duplicate = pipeline.graph().clone();
    let mut edge = duplicate.edges.iter().find(|edge| edge.target == shader).unwrap().clone();
    edge.id = "duplicate-binding".into();
    duplicate.edges.push(edge);
    assert_rejected(&mut pipeline, duplicate);
}

#[test]
fn constants_and_math_nodes_use_existing_factory_defaults_and_can_be_removed() {
    let mut pipeline = ClipPipeline::default();
    let mut factory = project(pipeline.graph().clone());
    let (constant, _) = factory.create_graph_node(NodeFactoryRequest::Constant { position: None }, 0).unwrap();
    pipeline.apply_command(GraphCommand::AddNode { node: constant.clone() }).unwrap();
    let mut factory = project(pipeline.graph().clone());
    let (math, _) = factory.create_graph_node(NodeFactoryRequest::Math { position: None, op: "multiply".into() }, 0).unwrap();
    pipeline.apply_command(GraphCommand::AddNode { node: math.clone() }).unwrap();
    pipeline.apply_command(GraphCommand::Connect {
        source: key(&constant.id, &constant.data.outputs[0].id), target: key(&math.id, &math.data.inputs[0].id),
    }).unwrap();
    pipeline.remove_nodes(&[math.id, constant.id]).unwrap();
    assert_eq!(pipeline, ClipPipeline::default());
}
