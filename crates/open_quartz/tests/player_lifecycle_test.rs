use open_quartz::{Environment, OpenQuartz, PlayerState};

#[test]
fn public_player_behavior_is_target_neutral() {
    let sdk = OpenQuartz::new(Environment::headless());
    let project = sdk.create_project("Player lifecycle");
    let mut player = sdk.player(project.graph()).build().unwrap();

    player.play().unwrap();
    assert_eq!(player.state(), PlayerState::Playing);
    player.pause().unwrap();
    assert_eq!(player.state(), PlayerState::Paused);
    player.resume().unwrap();
    player.stop().unwrap();
    assert_eq!(player.state(), PlayerState::Stopped);
}
