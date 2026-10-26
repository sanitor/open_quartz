use super::*;

#[test]
fn aggregate_handles_are_released_and_stale_handles_rejected() {
    let mut table = JniHandleTable::default();
    let client = table.create_client();
    let project = table.create_project(client, "JNI").unwrap();
    let player = table.create_player(project).unwrap();
    assert!(table
        .with_player(player, |player| player
            .play()
            .map_err(|error| error.to_json()))
        .is_ok());
    assert_eq!(
        table
            .with_player(player, |_| Err::<(), _>("operation failed".to_owned()))
            .unwrap_err(),
        "operation failed"
    );
    assert!(table.release_player(player));
    assert!(table.with_player(player, |_| Ok(())).is_err());
    assert!(table.release_client(client));
    assert!(!table.release_player(player));
    assert!(!table.release_client(client));
    assert!(table.create_project(client, "stale").is_err());
    assert!(table.create_player(project).is_err());

    let mut empty = JniHandleTable::default();
    assert!(empty.create_player(1).is_err());
    assert!(empty.create_project(1, "missing").is_err());
    assert_eq!(Arc::strong_count(&new_handle_table()), 1);

    table.next = u64::MAX;
    let exhausted =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| table.create_client()));
    assert!(exhausted.is_err());
}
