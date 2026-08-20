use open_quartz_sdk::{Environment, OpenQuartz, PlayerState, ProjectFile};

#[test]
fn sdk_crate_owns_public_project_and_player_aggregates() {
    let sdk = OpenQuartz::new(Environment::headless());
    let project = sdk.create_project("SDK crate");
    let player = sdk
        .player(project.graph())
        .build()
        .expect("empty graph should produce a ready player");

    let _: ProjectFile = project.to_file();
    assert_eq!(player.state(), PlayerState::Ready);
    assert_eq!(player.graph_revision(), 1);
}
