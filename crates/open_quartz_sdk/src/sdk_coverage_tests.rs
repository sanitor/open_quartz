use super::*;
use std::thread;

fn recover_lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    match value.lock() {
        Ok(guard) => guard,
        Err(error) => error.into_inner(),
    }
}

fn port(id: &str, data_type: DataType, direction: PortDirection) -> Port {
    Port {
        id: id.to_owned(),
        label: id.to_owned(),
        data_type,
        direction,
        default_value: None,
        description: None,
    }
}

fn node(id: &str, node_type: NodeType, inputs: Vec<Port>, outputs: Vec<Port>) -> ProjectNode {
    ProjectNode {
        id: id.to_owned(),
        node_type,
        position: Position { x: 0.0, y: 0.0 },
        data: NodeData {
            node_type,
            label: id.to_owned(),
            inputs,
            outputs,
            ..NodeData::default()
        },
    }
}

fn renderer_graph() -> Graph {
    Graph {
        nodes: vec![
            node(
                "source",
                NodeType::Shader,
                vec![],
                vec![port("out", DataType::Sampler2d, PortDirection::Output)],
            ),
            node(
                "renderer",
                NodeType::Renderer,
                vec![port("in", DataType::Sampler2d, PortDirection::Input)],
                vec![],
            ),
        ],
        edges: vec![Edge {
            id: "source-renderer".to_owned(),
            source: "source".to_owned(),
            source_handle: "out".to_owned(),
            target: "renderer".to_owned(),
            target_handle: "in".to_owned(),
        }],
    }
}

fn project_file(graph: Graph) -> ProjectFile {
    ProjectFile {
        version: PROJECT_FILE_VERSION.to_owned(),
        name: "Coverage".to_owned(),
        created_at: "created".to_owned(),
        updated_at: "updated".to_owned(),
        graph,
    }
}

#[test]
fn json_project_layout_and_screen_saver_contracts_cover_failures() {
    let sdk = OpenQuartz::new(Environment::headless());
    let invalid_json = sdk.open_project_json("{").unwrap_err();
    assert_eq!(invalid_json.code, SdkErrorCode::InvalidState);
    assert_eq!(
        sdk.normalize_project_json("{").unwrap_err().code,
        SdkErrorCode::InvalidState
    );
    assert_eq!(
        sdk.screen_saver_export_project_json("{", "renderer")
            .unwrap_err()
            .code,
        SdkErrorCode::InvalidState
    );

    let graph = renderer_graph();
    let json = serde_json::to_string(&project_file(graph.clone())).unwrap();
    let normalized = sdk.normalize_project_json(&json).unwrap();
    assert_eq!(
        sdk.open_project_json(&normalized).unwrap().name(),
        "Coverage"
    );
    let exported = sdk
        .screen_saver_export_project_json(&json, "renderer")
        .unwrap();
    assert_eq!(
        sdk.open_project_json(&exported)
            .unwrap()
            .graph()
            .nodes()
            .len(),
        2
    );
    assert_eq!(
        sdk.screen_saver_export_project_json(&json, "missing")
            .unwrap_err()
            .code,
        SdkErrorCode::UnknownNode
    );
    let invalid_file = project_file(Graph {
        nodes: vec![
            node("duplicate", NodeType::Input, vec![], vec![]),
            node("duplicate", NodeType::Input, vec![], vec![]),
        ],
        edges: vec![],
    });
    assert_eq!(
        Project::try_from_file(invalid_file).unwrap_err().code,
        SdkErrorCode::InvalidGraph
    );
    let missing_renderer = Project::try_from_file(project_file(renderer_graph())).unwrap();
    assert_eq!(
        missing_renderer
            .screen_saver_graph("missing", 1, 1)
            .unwrap_err()
            .code,
        SdkErrorCode::UnknownNode
    );

    let mut project = sdk.create_project("Initial");
    assert_eq!(project.name(), "Initial");
    project.set_name("Renamed");
    assert_eq!(project.name(), "Renamed");
    assert!(project.layout().position(&NodeId::new("missing")).is_none());
    assert_eq!(
        project
            .layout_mut()
            .set_position(NodeId::new("missing"), Position { x: 1.0, y: 2.0 })
            .unwrap_err()
            .code,
        SdkErrorCode::UnknownNode
    );
    project.initialize_graph(graph.clone()).unwrap();
    project
        .graph_mut()
        .nodes
        .push(node("without-layout", NodeType::Input, vec![], vec![]));
    assert_eq!(
        project
            .to_file()
            .graph
            .nodes
            .iter()
            .find(|node| node.id == "without-layout")
            .unwrap()
            .position,
        Position { x: 0.0, y: 0.0 }
    );
    assert_eq!(
        project
            .replace_graph(project.graph_snapshot(), 99)
            .unwrap_err()
            .code,
        SdkErrorCode::StaleRevision
    );
    assert!(project.layout().position(&NodeId::new("source")).is_some());
    assert_eq!(
        project.initialize_graph(graph.clone()).unwrap_err().code,
        SdkErrorCode::InvalidState
    );

    let mut stale_history = sdk.create_project("Stale history");
    stale_history
        .create_graph_node(NodeFactoryRequest::Constant { position: None }, 0)
        .unwrap();
    assert_eq!(
        stale_history.rollback_graph(99).unwrap_err().code,
        SdkErrorCode::StaleRevision
    );
    stale_history.rollback_graph(1).unwrap();
    assert_eq!(
        stale_history.redo_graph(99).unwrap_err().code,
        SdkErrorCode::StaleRevision
    );
    let mut empty = sdk.create_project("History");
    assert_eq!(
        empty.rollback_graph(0).unwrap_err().code,
        SdkErrorCode::InvalidState
    );
    assert_eq!(
        empty.redo_graph(0).unwrap_err().code,
        SdkErrorCode::InvalidState
    );

    let shader_only = Graph {
        nodes: vec![node("source", NodeType::Shader, vec![], vec![])],
        edges: vec![],
    };
    let shader_project = Project::try_from_file(project_file(shader_only)).unwrap();
    assert_eq!(
        shader_project
            .screen_saver_graph("source", 1, 1)
            .unwrap_err()
            .code,
        SdkErrorCode::UnknownNode
    );

    let renderer_without_input = Graph {
        nodes: vec![node("renderer", NodeType::Renderer, vec![], vec![])],
        edges: vec![],
    };
    let no_input = Project::try_from_file(project_file(renderer_without_input)).unwrap();
    assert_eq!(
        no_input
            .screen_saver_graph("renderer", 1, 1)
            .unwrap_err()
            .code,
        SdkErrorCode::InvalidGraph
    );

    let reserved_source = Graph {
        nodes: vec![
            node(
                SCREEN_SAVER_RESAMPLE_NODE_ID,
                NodeType::Shader,
                vec![],
                vec![port("out", DataType::Sampler2d, PortDirection::Output)],
            ),
            node(
                "renderer",
                NodeType::Renderer,
                vec![port("in", DataType::Sampler2d, PortDirection::Input)],
                vec![],
            ),
        ],
        edges: vec![Edge {
            id: "edge".to_owned(),
            source: SCREEN_SAVER_RESAMPLE_NODE_ID.to_owned(),
            source_handle: "out".to_owned(),
            target: "renderer".to_owned(),
            target_handle: "in".to_owned(),
        }],
    };
    let reserved = Project::try_from_file(project_file(reserved_source)).unwrap();
    assert_eq!(
        reserved
            .screen_saver_graph("renderer", 1, 1)
            .unwrap_err()
            .code,
        SdkErrorCode::InvalidGraph
    );
}

#[test]
fn node_factory_covers_every_executable_variant_and_math_descriptor() {
    let sdk = OpenQuartz::new(Environment::headless());
    let mut project = sdk.create_project("Factories");
    project
        .graph_mut()
        .nodes
        .push(node("legacy", NodeType::Shader, vec![], vec![]));
    project
        .graph_mut()
        .nodes
        .push(node("shader_bad", NodeType::Shader, vec![], vec![]));
    project
        .graph_mut()
        .nodes
        .push(node("shader_9", NodeType::Shader, vec![], vec![]));
    project
        .create_graph_node(
            NodeFactoryRequest::Shader {
                position: None,
                code: "@fragment fn main() -> @location(0) vec4f { return vec4f(1.0); }".to_owned(),
                label: "Factory Shader".to_owned(),
                template_name: None,
                shader_template_id: None,
            },
            0,
        )
        .unwrap();

    let (sampler, _) = project
        .create_graph_node(
            NodeFactoryRequest::Input {
                position: None,
                data_type: DataType::Sampler2d,
                input_mode: None,
            },
            project.graph_revision(),
        )
        .unwrap();
    assert_eq!(sampler.data.input_mode, Some(InputMode::Image));
    let (scalar, _) = project
        .create_graph_node(
            NodeFactoryRequest::Input {
                position: None,
                data_type: DataType::Float,
                input_mode: None,
            },
            project.graph_revision(),
        )
        .unwrap();
    assert_eq!(scalar.data.input_mode, None);

    for source in [
        SystemSource::Time,
        SystemSource::TimeDelta,
        SystemSource::Frame,
        SystemSource::Mouse,
        SystemSource::Resolution,
    ] {
        let (system, _) = project
            .create_graph_node(
                NodeFactoryRequest::System {
                    position: None,
                    source,
                },
                project.graph_revision(),
            )
            .unwrap();
        assert_eq!(system.data.system_source, Some(source));
    }

    project
        .create_graph_node(
            NodeFactoryRequest::Constant { position: None },
            project.graph_revision(),
        )
        .unwrap();
    project
        .create_graph_node(
            NodeFactoryRequest::Onnx {
                position: None,
                label: "Detector".to_owned(),
                template_name: None,
                model_id: Some("model".to_owned()),
                catalog_id: Some("catalog".to_owned()),
                inputs: vec![port("image", DataType::Sampler2d, PortDirection::Input)],
                outputs: vec![port("boxes", DataType::Roi, PortDirection::Output)],
            },
            project.graph_revision(),
        )
        .unwrap();
    project
        .create_graph_node(
            NodeFactoryRequest::CustomOnnx { position: None },
            project.graph_revision(),
        )
        .unwrap();

    let math_ops = [
        "add",
        "subtract",
        "multiply",
        "divide",
        "negate",
        "modulo",
        "min",
        "max",
        "clamp",
        "saturate",
        "step",
        "smoothstep",
        "abs",
        "sign",
        "sin",
        "cos",
        "tan",
        "asin",
        "acos",
        "atan",
        "pow",
        "sqrt",
        "exp",
        "log",
        "mix",
        "floor",
        "ceil",
        "round",
        "fract",
    ];
    for op in math_ops {
        let (math, _) = project
            .create_graph_node(
                NodeFactoryRequest::Math {
                    position: None,
                    op: op.to_owned(),
                },
                project.graph_revision(),
            )
            .unwrap();
        assert_eq!(math.data.math_op.as_deref(), Some(op));
    }
    assert_eq!(
        project
            .create_graph_node(
                NodeFactoryRequest::Math {
                    position: None,
                    op: "unknown".to_owned()
                },
                project.graph_revision(),
            )
            .unwrap_err()
            .code,
        SdkErrorCode::InvalidGraph
    );

    let mut invalid_project = sdk.create_project("Invalid internal graph");
    invalid_project.graph_mut().nodes.extend([
        node("duplicate", NodeType::Input, vec![], vec![]),
        node("duplicate", NodeType::Input, vec![], vec![]),
    ]);
    assert_eq!(
        invalid_project
            .create_graph_node(NodeFactoryRequest::Constant { position: None }, 0)
            .unwrap_err()
            .code,
        SdkErrorCode::InvalidGraph
    );

    assert_eq!(numeric_suffix("legacy"), "1");
    assert_eq!(normalize_label(" Two Words "), "two_words");
    let extra_ports = math_ports("math", 4, PortDirection::Input);
    assert_eq!(extra_ports[3].label, "value");
}

#[test]
fn graph_updates_cover_input_shader_parse_and_edge_change_contracts() {
    let sdk = OpenQuartz::new(Environment::headless());
    let mut project = sdk.create_project("Commands");
    assert_eq!(
        project
            .create_graph_node(NodeFactoryRequest::Constant { position: None }, 99)
            .unwrap_err()
            .code,
        SdkErrorCode::StaleRevision
    );
    assert_eq!(
        project
            .apply_graph_command(
                GraphCommand::RemoveNode {
                    node_id: NodeId::new("missing")
                },
                99,
            )
            .unwrap_err()
            .code,
        SdkErrorCode::StaleRevision
    );
    let mut invalid_initialization = sdk.create_project("Invalid initialization");
    assert_eq!(
        invalid_initialization
            .initialize_graph(Graph {
                nodes: vec![
                    node("same", NodeType::Input, vec![], vec![]),
                    node("same", NodeType::Input, vec![], vec![]),
                ],
                edges: vec![],
            })
            .unwrap_err()
            .code,
        SdkErrorCode::InvalidGraph
    );
    let (input, _) = project
        .create_graph_node(
            NodeFactoryRequest::Input {
                position: None,
                data_type: DataType::Float,
                input_mode: None,
            },
            0,
        )
        .unwrap();
    project
        .apply_graph_command(
            GraphCommand::UpdateInputType {
                node_id: NodeId::new(&input.id),
                data_type: DataType::Sampler2d,
                input_mode: None,
            },
            project.graph_revision(),
        )
        .unwrap();
    assert_eq!(
        project.graph().nodes()[0].data.input_mode,
        Some(InputMode::Image)
    );
    project
        .apply_graph_command(
            GraphCommand::UpdateInputType {
                node_id: NodeId::new(&input.id),
                data_type: DataType::Float,
                input_mode: Some(InputMode::System),
            },
            project.graph_revision(),
        )
        .unwrap();

    assert_eq!(
        project
            .apply_graph_command(
                GraphCommand::UpdateInputType {
                    node_id: NodeId::new("missing"),
                    data_type: DataType::Float,
                    input_mode: None,
                },
                project.graph_revision(),
            )
            .unwrap_err()
            .code,
        SdkErrorCode::UnknownNode
    );

    let (shader, _) = project
        .create_graph_node(
            NodeFactoryRequest::Shader {
                position: None,
                code: "@fragment fn main() -> @location(0) vec4f { return vec4f(1.0); }".to_owned(),
                label: "Shader".to_owned(),
                template_name: Some("Shader".to_owned()),
                shader_template_id: None,
            },
            project.graph_revision(),
        )
        .unwrap();
    assert_eq!(
        project
            .apply_graph_command(
                GraphCommand::UpdateInputType {
                    node_id: NodeId::new(&shader.id),
                    data_type: DataType::Float,
                    input_mode: None,
                },
                project.graph_revision(),
            )
            .unwrap_err()
            .code,
        SdkErrorCode::InvalidGraph
    );
    assert_eq!(
        project
            .apply_graph_command(
                GraphCommand::UpdateShaderCode {
                    node_id: NodeId::new("missing"),
                    shader_code: String::new(),
                },
                project.graph_revision(),
            )
            .unwrap_err()
            .code,
        SdkErrorCode::UnknownNode
    );
    assert!(changed_node_ids(&Graph::default(), &Graph::default()).is_empty());
    let unchanged = renderer_graph();
    assert!(changed_node_ids(&unchanged, &unchanged).is_empty());

    let shader_source = node(
        "shader",
        NodeType::Shader,
        vec![],
        vec![port("out", DataType::Sampler2d, PortDirection::Output)],
    );
    let renderer_target = node(
        "renderer",
        NodeType::Renderer,
        vec![port("in", DataType::Sampler2d, PortDirection::Input)],
        vec![],
    );
    let mut invalid_shader_update = Project::try_from_file(project_file(Graph {
        nodes: vec![shader_source, renderer_target],
        edges: vec![Edge {
            id: "shader-renderer".to_owned(),
            source: "shader".to_owned(),
            source_handle: "out".to_owned(),
            target: "renderer".to_owned(),
            target_handle: "in".to_owned(),
        }],
    }))
    .unwrap();
    assert_eq!(
        invalid_shader_update
            .apply_graph_command(
                GraphCommand::UpdateShaderCode {
                    node_id: NodeId::new("shader"),
                    shader_code: "not wgsl".to_owned(),
                },
                0,
            )
            .unwrap_err()
            .code,
        SdkErrorCode::InvalidGraph
    );

    let input_source = node(
        "input",
        NodeType::Input,
        vec![],
        vec![port("out", DataType::Float, PortDirection::Output)],
    );
    let scalar_target = node(
        "target",
        NodeType::Math,
        vec![port("in", DataType::Float, PortDirection::Input)],
        vec![],
    );
    let mut invalid_input_update = Project::try_from_file(project_file(Graph {
        nodes: vec![input_source, scalar_target],
        edges: vec![Edge {
            id: "input-target".to_owned(),
            source: "input".to_owned(),
            source_handle: "out".to_owned(),
            target: "target".to_owned(),
            target_handle: "in".to_owned(),
        }],
    }))
    .unwrap();
    assert_eq!(
        invalid_input_update
            .apply_graph_command(
                GraphCommand::UpdateInputType {
                    node_id: NodeId::new("input"),
                    data_type: DataType::Sampler2d,
                    input_mode: None,
                },
                0,
            )
            .unwrap_err()
            .code,
        SdkErrorCode::InvalidGraph
    );
    project
        .apply_graph_command(
            GraphCommand::UpdateShaderCode {
                node_id: NodeId::new(&shader.id),
                shader_code: "not wgsl".to_owned(),
            },
            project.graph_revision(),
        )
        .unwrap();
    assert!(project
        .graph()
        .nodes()
        .iter()
        .find(|node| node.id == shader.id)
        .unwrap()
        .data
        .extra
        .contains_key("parseError"));

    let source = node(
        "a",
        NodeType::Input,
        vec![],
        vec![port("out", DataType::Float, PortDirection::Output)],
    );
    let target = node(
        "b",
        NodeType::Input,
        vec![port("in", DataType::Float, PortDirection::Input)],
        vec![],
    );
    let graph_without_edge = Graph {
        nodes: vec![source.clone(), target.clone()],
        edges: vec![],
    };
    let graph_with_edge = Graph {
        nodes: vec![source, target],
        edges: vec![Edge {
            id: "a-b".to_owned(),
            source: "a".to_owned(),
            source_handle: "out".to_owned(),
            target: "b".to_owned(),
            target_handle: "in".to_owned(),
        }],
    };
    let mut edge_project = sdk.create_project("Edges");
    edge_project.initialize_graph(graph_without_edge).unwrap();
    let added = edge_project.replace_graph(graph_with_edge, 0).unwrap();
    assert_eq!(added.changed_nodes().len(), 2);
    let removed = edge_project
        .replace_graph(
            Graph {
                nodes: edge_project.graph().nodes().to_vec(),
                edges: vec![],
            },
            1,
        )
        .unwrap();
    assert_eq!(removed.changed_nodes().len(), 2);
}

#[test]
fn resources_outputs_player_states_and_poisoning_cover_public_boundaries() {
    let sdk = OpenQuartz::new(Environment::headless());
    let mut catalog = ResourceCatalog::default();
    assert!(catalog.is_empty());
    let resource = Resource::new(
        ResourceId::new("image"),
        ResourceKind::Image,
        ResourceSource::Path("image.png".to_owned()),
    );
    assert_eq!(resource.kind(), ResourceKind::Image);
    assert_eq!(
        resource.source(),
        &ResourceSource::Path("image.png".to_owned())
    );
    catalog.add(resource).unwrap();
    assert!(!catalog.is_empty());

    let invalid_graph = Graph {
        nodes: vec![
            node("same", NodeType::Input, vec![], vec![]),
            node("same", NodeType::Input, vec![], vec![]),
        ],
        edges: vec![],
    };
    assert_eq!(
        sdk.player(&invalid_graph)
            .build()
            .err()
            .expect("invalid graph")
            .code,
        SdkErrorCode::InvalidGraph
    );

    let graph = Graph {
        nodes: vec![node(
            "source",
            NodeType::Input,
            vec![],
            vec![
                port("value", DataType::Float, PortDirection::Output),
                port("texture", DataType::Sampler2d, PortDirection::Output),
            ],
        )],
        edges: vec![],
    };
    let mut player = sdk.player(&graph).with_resources(&catalog).build().unwrap();
    let mut invalid_lifecycle = sdk.player(&graph).build().unwrap();
    invalid_lifecycle.play().unwrap();
    assert_eq!(
        invalid_lifecycle.play().unwrap_err().code,
        SdkErrorCode::InvalidState
    );
    invalid_lifecycle.stop().unwrap();
    assert_eq!(
        invalid_lifecycle.resume().unwrap_err().code,
        SdkErrorCode::InvalidState
    );
    invalid_lifecycle.stop().unwrap();
    invalid_lifecycle.close().unwrap();
    assert_eq!(
        invalid_lifecycle.pause().unwrap_err().code,
        SdkErrorCode::Disposed
    );
    assert_eq!(
        invalid_lifecycle.resume().unwrap_err().code,
        SdkErrorCode::Disposed
    );
    assert_eq!(
        invalid_lifecycle.stop().unwrap_err().code,
        SdkErrorCode::Disposed
    );
    assert_eq!(player.outputs().len(), 2);
    assert_eq!(player.outputs()[0].key().node_id, "source");
    let mut latest = player.outputs()[0]
        .subscribe(OutputPolicy::latest())
        .unwrap();
    let mut every = player.outputs()[0]
        .subscribe(OutputPolicy::every())
        .unwrap();
    let texture_output = player.outputs()[1].clone();
    let changed = texture_output.subscribe(OutputPolicy::on_change()).unwrap();
    drop(changed);
    latest.close().unwrap();
    every.close().unwrap();
    every.close().unwrap();

    let invalid_apply = Graph {
        nodes: vec![
            node("duplicate", NodeType::Input, vec![], vec![]),
            node("duplicate", NodeType::Input, vec![], vec![]),
        ],
        edges: vec![],
    };
    assert_eq!(
        player
            .apply_graph(&invalid_apply, &GraphChange::default())
            .unwrap_err()
            .code,
        SdkErrorCode::InvalidGraph
    );

    let revision = player
        .apply_graph(&Graph::default(), &GraphChange::default())
        .unwrap();
    assert_eq!(revision, 2);
    assert!(player.outputs().is_empty());
    assert_eq!(
        player.apply_graph(&graph, &GraphChange::default()).unwrap(),
        3
    );
    player.close().unwrap();
    player.close().unwrap();
    assert_eq!(
        player
            .apply_graph(&graph, &GraphChange::default())
            .unwrap_err()
            .code,
        SdkErrorCode::Disposed
    );

    let dropped_output = {
        let owner = sdk.player(&graph).build().unwrap();
        owner.outputs()[0].clone()
    };
    assert_eq!(
        dropped_output
            .subscribe(OutputPolicy::latest())
            .err()
            .expect("dropped output")
            .code,
        SdkErrorCode::Disposed
    );

    let missing_runtime = Arc::new(Mutex::new(Runtime::new(RuntimeCapabilities {
        data_paths: vec![],
    })));
    recover_lock(&missing_runtime)
        .set_graph(&Graph::default())
        .unwrap();
    let missing_output = Output {
        key: OutputKey::new("missing", "missing"),
        data_type: DataType::Float,
        runtime: Arc::downgrade(&missing_runtime),
        subscription_counter: Arc::new(AtomicU64::new(1)),
    };
    assert!(missing_output.subscribe(OutputPolicy::latest()).is_err());

    let disposed_owner = sdk.player(&graph).build().unwrap();
    let mut disposed_subscription = disposed_owner.outputs()[0]
        .subscribe(OutputPolicy::latest())
        .unwrap();
    let disposed_runtime = disposed_subscription.runtime.upgrade().unwrap();
    recover_lock(&disposed_runtime).dispose().unwrap();
    assert!(disposed_subscription.close().is_err());
    drop(disposed_owner);

    let mut orphaned_subscription = {
        let owner = sdk.player(&graph).build().unwrap();
        owner.outputs()[0]
            .subscribe(OutputPolicy::latest())
            .unwrap()
    };
    orphaned_subscription.close().unwrap();
    assert!(orphaned_subscription.is_closed());
    assert_eq!(map_player_state(EngineState::Empty), PlayerState::Empty);

    let poisoned = Arc::new(Mutex::new(Runtime::new(RuntimeCapabilities {
        data_paths: vec![],
    })));
    let worker_runtime = poisoned.clone();
    let _ = thread::spawn(move || {
        let _guard = recover_lock(&worker_runtime);
        panic!("poison runtime mutex");
    })
    .join();
    drop(recover_lock(&poisoned));
    assert_eq!(
        lock_runtime(&poisoned)
            .err()
            .expect("poisoned runtime")
            .code,
        SdkErrorCode::InvalidState
    );
    let poisoned_output = Output {
        key: OutputKey::new("source", "value"),
        data_type: DataType::Float,
        runtime: Arc::downgrade(&poisoned),
        subscription_counter: Arc::new(AtomicU64::new(1)),
    };
    assert_eq!(
        poisoned_output
            .subscribe(OutputPolicy::latest())
            .err()
            .unwrap()
            .code,
        SdkErrorCode::InvalidState
    );
    let mut poisoned_subscription = Subscription {
        runtime: Arc::downgrade(&poisoned),
        subscription_id: "poisoned".to_owned(),
        closed: false,
    };
    assert_eq!(
        poisoned_subscription.close().unwrap_err().code,
        SdkErrorCode::InvalidState
    );

    let mut poisoned_player = Player {
        runtime: poisoned,
        outputs: vec![],
        resources: vec![],
        subscription_counter: Arc::new(AtomicU64::new(1)),
        now_ns: 0,
    };
    assert_eq!(poisoned_player.state(), PlayerState::Closed);
    assert_eq!(poisoned_player.graph_revision(), 0);
    assert_eq!(
        poisoned_player.play().unwrap_err().code,
        SdkErrorCode::InvalidState
    );
    assert_eq!(
        poisoned_player.pause().unwrap_err().code,
        SdkErrorCode::InvalidState
    );
    assert_eq!(
        poisoned_player.resume().unwrap_err().code,
        SdkErrorCode::InvalidState
    );
    assert_eq!(
        poisoned_player.stop().unwrap_err().code,
        SdkErrorCode::InvalidState
    );
    assert_eq!(
        poisoned_player
            .apply_graph(&Graph::default(), &GraphChange::default())
            .unwrap_err()
            .code,
        SdkErrorCode::InvalidState
    );
    assert_eq!(
        poisoned_player.close().unwrap_err().code,
        SdkErrorCode::InvalidState
    );
}

#[test]
fn migrated_public_contracts_cover_sdk_ownership_layer() {
    let sdk = OpenQuartz::new(Environment::headless());
    let incompatible = ProjectFile {
        version: "0.3.0".to_owned(),
        name: "Old".to_owned(),
        created_at: String::new(),
        updated_at: String::new(),
        graph: Graph::default(),
    };
    assert_eq!(
        Project::try_from_file(incompatible).unwrap_err().code,
        SdkErrorCode::InvalidState
    );

    let mut project = Project::try_from_file(project_file(renderer_graph())).unwrap();
    let saver = project.screen_saver_graph("renderer", 1920, 1080).unwrap();
    assert!(saver
        .nodes()
        .iter()
        .any(|node| node.id == SCREEN_SAVER_RESAMPLE_NODE_ID));
    project
        .layout_mut()
        .set_position(NodeId::new("source"), Position { x: 10.0, y: 20.0 })
        .unwrap();
    assert_eq!(
        project.layout().position(&NodeId::new("source")),
        Some(Position { x: 10.0, y: 20.0 })
    );

    let invalid = Graph {
        nodes: vec![
            node("same", NodeType::Input, vec![], vec![]),
            node("same", NodeType::Input, vec![], vec![]),
        ],
        edges: vec![],
    };
    assert_eq!(
        project.replace_graph(invalid, 0).unwrap_err().code,
        SdkErrorCode::InvalidGraph
    );

    let mut history = sdk.create_project("History");
    history
        .create_graph_node(NodeFactoryRequest::Renderer { position: None }, 0)
        .unwrap();
    history.rollback_graph(1).unwrap();
    history.redo_graph(2).unwrap();

    let (shader, _) = history
        .create_graph_node(
            NodeFactoryRequest::Shader {
                position: None,
                code: "not wgsl".to_owned(),
                label: "Parse".to_owned(),
                template_name: None,
                shader_template_id: None,
            },
            history.graph_revision(),
        )
        .unwrap();
    history
        .apply_graph_command(
            GraphCommand::UpdateShaderCode {
                node_id: NodeId::new(&shader.id),
                shader_code: "not wgsl".to_owned(),
            },
            history.graph_revision(),
        )
        .unwrap();
    history
        .apply_graph_command(
            GraphCommand::UpdateShaderCode {
                node_id: NodeId::new(&shader.id),
                shader_code: "@fragment fn main() -> @location(0) vec4f { return vec4f(1.0); }"
                    .to_owned(),
            },
            history.graph_revision(),
        )
        .unwrap();
    assert!(!history
        .graph()
        .nodes()
        .iter()
        .find(|node| node.id == shader.id)
        .unwrap()
        .data
        .extra
        .contains_key("parseError"));

    let mut resources = ResourceCatalog::default();
    let resource = Resource::new(
        ResourceId::new("resource"),
        ResourceKind::Video,
        ResourceSource::Url("https://example.invalid/video".to_owned()),
    );
    assert_eq!(resource.id().as_str(), "resource");
    resources.add(resource.clone()).unwrap();
    assert_eq!(resources.len(), 1);
    assert_eq!(
        resources.add(resource).unwrap_err().code,
        SdkErrorCode::InvalidResource
    );
    assert_eq!(project.resources().len(), 0);
    project
        .resources_mut()
        .add(Resource::new(
            ResourceId::new("embedded"),
            ResourceKind::Bytes,
            ResourceSource::Embedded("payload".to_owned()),
        ))
        .unwrap();
    assert_eq!(project.resources().len(), 1);

    let mut player = sdk
        .player(project.graph())
        .with_resources(project.resources())
        .build()
        .unwrap();
    assert_eq!(player.state(), PlayerState::Ready);
    assert_eq!(player.graph_revision(), 1);
    assert_eq!(player.resources().len(), 1);
    player.play().unwrap();
    assert_eq!(player.state(), PlayerState::Playing);
    player.pause().unwrap();
    assert_eq!(player.state(), PlayerState::Paused);
    player.resume().unwrap();
    assert_eq!(player.state(), PlayerState::Playing);
    player.stop().unwrap();
    assert_eq!(player.state(), PlayerState::Stopped);
    player.close().unwrap();
    assert_eq!(player.state(), PlayerState::Closed);

    assert_eq!(map_player_state(EngineState::Ready), PlayerState::Ready);
    assert_eq!(map_player_state(EngineState::Running), PlayerState::Playing);
    assert_eq!(map_player_state(EngineState::Paused), PlayerState::Paused);
    assert_eq!(map_player_state(EngineState::Stopped), PlayerState::Stopped);
    assert_eq!(map_player_state(EngineState::Disposed), PlayerState::Closed);
}
