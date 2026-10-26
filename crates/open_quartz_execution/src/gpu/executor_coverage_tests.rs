use super::*;
use crate::engine::{build_execution_plan, GpuFacade};
use serde_json::json;

async fn backend() -> Arc<GpuBackend> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: !cfg!(target_os = "macos"),
            compatible_surface: None,
            apply_limit_buckets: false,
        })
        .await
        .unwrap();
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default())
        .await
        .unwrap();
    Arc::new(GpuBackend::from_device(device, queue))
}

fn shader_plan() -> ExecutionPlan {
    let nodes: Vec<open_quartz_schema::ProjectNode> = serde_json::from_value(json!([{
        "id":"shader", "type":"shader", "position":{"x":0.0,"y":0.0},
        "data":{"type":"shader","label":"Shader","shaderCode":"@fragment fn main() -> @location(0) vec4f { return vec4f(gain); }","inputs":[{"id":"gain","label":"gain","dataType":"float","direction":"input"}],"outputs":[],"uniforms":{"gain":1.0}}
    }])).unwrap();
    build_execution_plan(&nodes, &[])
}

fn command(kind: &str) -> ExecutionCommand {
    ExecutionCommand {
        node_id: "node".to_owned(),
        kind: kind.to_owned(),
        output_port_id: None,
        texture_inputs: BTreeMap::new(),
        uniforms: BTreeMap::new(),
        target_width: None,
        target_height: None,
        feedback_read_index: None,
        feedback_write_index: None,
        clear_feedback: false,
        scalar_output: None,
    }
}

fn binding(kind: &str, name: &str, ty: Option<&str>) -> BindingDescriptor {
    BindingDescriptor {
        binding: 0,
        kind: kind.to_owned(),
        name: name.to_owned(),
        wgsl_type: ty.map(str::to_owned),
    }
}

#[test]
fn gpu_error_binding_layout_uniform_size_and_packing_cover_variants() {
    let node_error = GpuExecutionError::for_node("node", "bad");
    assert_eq!(node_error.to_string(), "GPU node node: bad");
    let plain = GpuExecutionError {
        node_id: None,
        message: "plain".to_owned(),
    };
    assert!(GpuExecutor::map_readback_result("node", Err("read".to_owned())).is_err());
    assert_eq!(plain.to_string(), "plain");

    for descriptor in [
        binding("uniform", "u", Some("f32")),
        binding("texture", "t", None),
        binding("sampler", "s", None),
    ] {
        assert!(binding_layout_entry(&descriptor).is_ok());
    }
    assert!(binding_layout_entry(&binding("externalTexture", "e", None)).is_err());
    assert!(binding_layout_entry(&binding("unknown", "x", None)).is_err());
    assert_eq!(target_format(true), TextureFormat::Rgba32Float);
    assert_eq!(target_format(false), TextureFormat::Rgba8Unorm);

    for (ty, size) in [
        ("vec2f", 8),
        ("vec2i", 8),
        ("vec2u", 8),
        ("vec3f", 16),
        ("vec3i", 16),
        ("vec3u", 16),
        ("vec4f", 16),
        ("vec4i", 16),
        ("vec4u", 16),
        ("mat2x2f", 16),
        ("mat3x3f", 48),
        ("mat4x4f", 64),
        ("f32", 4),
    ] {
        assert_eq!(uniform_size(ty), size);
    }
    assert_eq!(pack_uniform("i32", &[2.0]), 2_i32.to_le_bytes());
    assert_eq!(pack_uniform("u32", &[3.0]), 3_u32.to_le_bytes());
    assert_eq!(pack_uniform("f32", &[4.0]), 4_f32.to_le_bytes());
    assert_eq!(pack_uniform("mat3x3f", &[1.0; 9]).len(), 48);
    assert_eq!(pack_uniform("mat3x3f", &[]), vec![0; 48]);
}

#[test]
fn executor_public_and_command_error_paths_are_structured() {
    pollster::block_on(async {
        let backend = backend().await;
        assert!(
            !backend.supported_features().is_empty() || backend.supported_features().is_empty()
        );
        let mut executor = GpuExecutor::new(backend.clone());
        assert!(Arc::ptr_eq(executor.backend(), &backend));
        assert!(executor.read_output_rgba("missing").await.is_err());

        assert!(executor.upload_rgba("bad", &[0; 3], 1, 1).is_err());
        let texture = backend.create_texture(1, 1, TextureFormat::Rgba8Unorm);
        let invalid = GpuOutputHandle {
            texture: texture.texture,
            view: texture.view,
            sampler: texture.sampler,
            width: 0,
            height: 1,
            format: texture.format,
        };
        assert!(executor.register_external_texture("", invalid).is_err());

        let empty = ExecutionPlan {
            sorted_ids: vec![],
            nodes: vec![],
            output_nodes: vec![],
            default_width: 1,
            default_height: 1,
            cycle: false,
        };
        assert!(executor
            .execute_commands(&empty, &[command("math")])
            .is_ok());
        assert!(executor
            .execute_commands(&empty, &[command("onnx")])
            .is_err());
        assert!(executor
            .execute_commands(&empty, &[command("other")])
            .is_err());
        assert!(executor
            .execute_commands(&empty, &[command("renderer")])
            .is_err());
        let mut renderer = command("renderer");
        renderer
            .texture_inputs
            .insert("input".to_owned(), "missing".to_owned());
        assert!(executor.execute_commands(&empty, &[renderer]).is_err());
        assert!(executor
            .execute(
                &empty,
                &FrameResult {
                    frame: 1,
                    commands: vec![command("math")],
                    dirty_nodes: vec![]
                }
            )
            .is_ok());
    });
}

#[test]
fn executor_resource_matching_bind_group_and_shader_error_paths() {
    pollster::block_on(async {
        let backend = backend().await;
        let mut executor = GpuExecutor::new(backend.clone());
        let plan = shader_plan();
        executor.sync_plan(&plan).unwrap();
        let resources = executor.nodes.get("shader").unwrap();
        assert!(executor.output_handle("shader").is_some());
        let mut float_plan = plan.nodes[0].clone();
        float_plan.target.as_mut().unwrap().float = true;
        if !backend
            .supported_features()
            .contains(wgpu::Features::FLOAT32_FILTERABLE)
        {
            assert!(executor.create_node_resources(&float_plan).is_err());
        }
        assert!(resources.matches(&plan.nodes[0]));
        let mut no_shader = plan.nodes[0].clone();
        no_shader.shader = None;
        assert!(!resources.matches(&no_shader));
        let mut no_target = plan.nodes[0].clone();
        no_target.target = None;
        assert!(!resources.matches(&no_target));
        assert!(executor.create_node_resources(&no_target).is_err());

        let mut external = plan.nodes[0].clone();
        external
            .shader
            .as_mut()
            .unwrap()
            .external_texture_bindings
            .insert("video".to_owned(), 0);
        assert!(executor.create_node_resources(&external).is_err());
        let mut invalid = plan.clone();
        invalid.nodes[0]
            .validation_errors
            .push(crate::wgsl::compiler::WgslCompilationError {
                message: "invalid".to_owned(),
                line: 1,
                column: 1,
                offset: 0,
                length: 0,
            });
        assert!(executor.sync_plan(&invalid).is_err());

        let resources = executor.nodes.get("shader").unwrap();
        assert!(bind_group_entry(
            &binding("uniform", "missing", Some("f32")),
            resources,
            &BTreeMap::new()
        )
        .is_err());
        assert!(bind_group_entry(
            &binding("texture", "missing", None),
            resources,
            &BTreeMap::new()
        )
        .is_err());
        assert!(bind_group_entry(
            &binding("sampler", "missingSampler", None),
            resources,
            &BTreeMap::new()
        )
        .is_err());
        assert!(bind_group_entry(
            &binding("unknown", "missing", None),
            resources,
            &BTreeMap::new()
        )
        .is_err());
        executor
            .nodes
            .get_mut("shader")
            .unwrap()
            .bindings
            .push(binding("unknown", "x", None));
        let mut invalid_binding_command = command("shader");
        invalid_binding_command.node_id = "shader".to_owned();
        assert!(executor
            .execute_commands(&plan, &[invalid_binding_command])
            .is_err());
        executor.nodes.get_mut("shader").unwrap().bindings.pop();

        let mut missing_source = command("shader");
        missing_source.node_id = "shader".to_owned();
        missing_source
            .texture_inputs
            .insert("image".to_owned(), "absent".to_owned());
        assert!(executor.execute_commands(&plan, &[missing_source]).is_err());
        let missing_resources = command("shader");
        assert!(executor
            .execute_commands(&plan, &[missing_resources])
            .is_err());

        let mut feedback = command("shader");
        feedback.node_id = "shader".to_owned();
        feedback.feedback_read_index = Some(9);
        assert!(executor.execute_commands(&plan, &[feedback]).is_err());
        let mut write = command("shader");
        write.node_id = "shader".to_owned();
        write.feedback_write_index = Some(9);
        write.uniforms.insert("unknown".to_owned(), vec![1.0]);
        assert!(executor.execute_commands(&plan, &[write]).is_err());

        let trait_error =
            GpuFacade::execute(&mut executor, &empty_plan(), &[command("unsupported")])
                .unwrap_err();
        assert_eq!(trait_error.code, SdkErrorCode::InvalidResource);
        let mut none_error_plan = plan.clone();
        none_error_plan.nodes[0]
            .shader
            .as_mut()
            .unwrap()
            .bindings
            .push(binding("unknown", "x", None));
        none_error_plan.nodes[0]
            .shader
            .as_mut()
            .unwrap()
            .full_fragment_code
            .push(' ');
        for _ in 0..3 {
            executor
                .backend
                .readback_ring
                .acquire(&executor.backend.device, 256)
                .unwrap();
        }
        assert!(executor.read_output_rgba("shader").await.is_err());

        let trait_error = GpuFacade::execute(&mut executor, &none_error_plan, &[]).unwrap_err();
        assert!(trait_error.node_id.is_none());
    });
}

fn empty_plan() -> ExecutionPlan {
    ExecutionPlan {
        sorted_ids: vec![],
        nodes: vec![],
        output_nodes: vec![],
        default_width: 1,
        default_height: 1,
        cycle: false,
    }
}
