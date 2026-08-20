use open_quartz::{Environment, OpenQuartz, PlayerState};
use open_quartz::ffi::OpenQuartzBinding;
use serde_json::Value;

#[test]
fn public_objects_create_an_independent_project_and_player() {
    let sdk = OpenQuartz::new(Environment::headless());
    let project = sdk.create_project("Untitled");

    assert_eq!(project.name(), "Untitled");
    assert!(project.graph().nodes().is_empty());
    assert!(project.graph().edges().is_empty());

    let player = sdk
        .player(project.graph())
        .build()
        .expect("an empty project graph should produce a ready player");

    assert_eq!(player.state(), PlayerState::Ready);
    assert_eq!(player.graph_revision(), 1);
    assert!(player.outputs().is_empty());
}

#[test]
fn player_owns_applied_graph_state_instead_of_borrowing_project() {
    let sdk = OpenQuartz::new(Environment::headless());
    let mut project = sdk.create_project("Editable");
    let player = sdk
        .player(project.graph())
        .build()
        .expect("player construction should succeed");

    project.set_name("Edited after player creation");
    drop(project);

    assert_eq!(player.state(), PlayerState::Ready);
    assert_eq!(player.graph_revision(), 1);
}

#[test]
fn project_graph_revision_is_atomic_and_rejects_stale_edits() {
    let sdk = OpenQuartz::new(Environment::headless());
    let mut project = sdk.create_project("Revisioned");
    assert_eq!(project.graph_revision(), 0);

    let change = project
        .replace_graph(project.graph_snapshot(), 0)
        .expect("matching revision should commit");
    assert!(change.is_empty());
    assert_eq!(project.graph_revision(), 1);

    let stale = project
        .replace_graph(project.graph_snapshot(), 0)
        .expect_err("stale revision must be rejected");
    assert_eq!(stale.code, open_quartz::SdkErrorCode::StaleRevision);
}

#[test]
fn project_graph_rollback_restores_previous_snapshot_and_advances_revision() {
    let sdk = OpenQuartz::new(Environment::headless());
    let mut project = sdk.create_project("Rollback");
    let original = project.graph_snapshot();

    project.replace_graph(open_quartz::Graph::default(), 0).unwrap();
    project.rollback_graph(1).unwrap();

    assert_eq!(project.graph_snapshot(), original);
    assert_eq!(project.graph_revision(), 2);
}

#[test]
fn binding_graph_and_player_contract_round_trips_errors_and_dispose() {
    let sdk = OpenQuartzBinding::new();
    let project = sdk.create_project("Binding");
    let graph = project.graph();

    assert_eq!(graph.revision(), 0);
    assert_eq!(graph.snapshot_json().unwrap(), r#"{"nodes":[],"edges":[]}"#);
    assert_eq!(
        serde_json::from_str::<Value>(&graph.replace(r#"{"nodes":[],"edges":[]}"#, 0).unwrap())
            .unwrap()["revision"],
        1
    );
    let stale: Value =
        serde_json::from_str(&graph.replace(r#"{"nodes":[],"edges":[]}"#, 0).unwrap_err())
            .unwrap();
    assert_eq!(stale["code"], "stale-revision");

    assert_eq!(
        serde_json::from_str::<Value>(&graph.rollback(1).unwrap()).unwrap()["revision"],
        2
    );
    let project_json = project.to_json().unwrap();
    let reopened = sdk.open_project(&project_json).unwrap();
    assert_eq!(reopened.name(), "Binding");

    let mut player = project.create_player().unwrap();
    player.close().unwrap();
    let disposed: Value = serde_json::from_str(&player.play().unwrap_err()).unwrap();
    assert_eq!(disposed["code"], "disposed");
}
