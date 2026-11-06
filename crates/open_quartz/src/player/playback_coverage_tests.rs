use super::*;
use std::thread;

use crate::execution::runtime::RuntimeCapabilities;
use crate::player::{Environment, OpenQuartz};
use crate::project::{NodeType, Project, ResourceId};
use crate::resources::{ResourceKind, ResourceSource};

use super::super::test_support::{node, player_graph, project_file, recover_lock, renderer_graph};

#[test]
fn builder_and_graph_updates_validate_and_preserve_lifecycle_errors() {
    let sdk = OpenQuartz::new(Environment::headless());
    let mut catalog = ResourceCatalog::default();
    let resource = Resource::new(
        ResourceId::new("image"),
        ResourceKind::Image,
        ResourceSource::Path("image.png".to_owned()),
    );
    catalog.add(resource).unwrap();

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

    let graph = player_graph();
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
}

#[test]
fn state_mapping_and_poisoned_player_cover_private_runtime_failures() {
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
fn renderer_project_resources_and_playback_states_cover_player_integration() {
    let sdk = OpenQuartz::new(Environment::headless());
    let mut project = Project::try_from_file(project_file(renderer_graph())).unwrap();
    project
        .layout_mut()
        .set_position(
            crate::project::NodeId::new("source"),
            crate::project::Position { x: 10.0, y: 20.0 },
        )
        .unwrap();
    project
        .resources_mut()
        .add(Resource::new(
            ResourceId::new("embedded"),
            ResourceKind::Bytes,
            ResourceSource::Embedded("payload".to_owned()),
        ))
        .unwrap();

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
