use super::*;

fn take(value: *mut c_char) -> String {
    assert!(!value.is_null());
    let result = unsafe { CStr::from_ptr(value) }
        .to_str()
        .unwrap()
        .to_owned();
    open_quartz_jni_free_string(value);
    result
}

#[test]
fn c_abi_covers_success_errors_strings_and_releases() {
    open_quartz_jni_free_string(ptr::null_mut());
    assert_eq!(
        take(open_quartz_jni_sdk_version()),
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(open_quartz_jni_create_project(0, ptr::null()), 0);
    let invalid_utf8 = [0xff_u8, 0];
    assert_eq!(
        open_quartz_jni_create_project(0, invalid_utf8.as_ptr().cast()),
        0
    );

    let client = open_quartz_jni_create_client();
    let name = CString::new("JNI ABI").unwrap();
    let project = open_quartz_jni_create_project(client, name.as_ptr());
    assert_ne!(project, 0);
    assert_eq!(open_quartz_jni_create_project(0, name.as_ptr()), 0);
    let player = open_quartz_jni_create_player(project);
    assert_ne!(player, 0);
    assert_eq!(open_quartz_jni_create_player(0), 0);

    assert!(open_quartz_jni_player_play(player).is_null());
    assert!(open_quartz_jni_player_pause(player).is_null());
    assert!(open_quartz_jni_player_resume(player).is_null());
    assert!(open_quartz_jni_player_stop(player).is_null());

    let error_project = open_quartz_jni_create_project(client, name.as_ptr());
    let error_player = open_quartz_jni_create_player(error_project);
    assert!(take(open_quartz_jni_player_pause(error_player)).contains("invalid-state"));
    assert!(open_quartz_jni_player_play(error_player).is_null());
    assert!(take(open_quartz_jni_player_play(error_player)).contains("invalid-state"));
    assert!(open_quartz_jni_player_stop(error_player).is_null());
    assert!(take(open_quartz_jni_player_resume(error_player)).contains("invalid-state"));
    {
        let mut table = table().lock().unwrap();
        table
            .with_player(error_player, |player| {
                player.close().map_err(|error| error.to_json())
            })
            .unwrap();
    }
    assert!(take(open_quartz_jni_player_stop(error_player)).contains("disposed"));
    assert!(open_quartz_jni_release_player(error_player));

    assert!(open_quartz_jni_release_player(player));
    assert!(!open_quartz_jni_release_player(player));
    assert!(take(open_quartz_jni_player_play(player)).contains("stale"));
    assert!(take(open_quartz_jni_player_pause(player)).contains("stale"));
    assert!(take(open_quartz_jni_player_resume(player)).contains("stale"));
    assert!(take(open_quartz_jni_player_stop(player)).contains("stale"));
    assert!(open_quartz_jni_release_client(client));
    assert!(!open_quartz_jni_release_client(client));
}
