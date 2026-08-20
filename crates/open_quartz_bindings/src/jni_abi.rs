use std::ffi::{c_char, CStr, CString};
use std::ptr;
use std::sync::LazyLock;

use crate::jni::SharedJniHandleTable;

static TABLE: LazyLock<SharedJniHandleTable> = LazyLock::new(crate::jni::new_handle_table);

fn table() -> &'static SharedJniHandleTable {
    &TABLE
}

fn string_ptr(value: String) -> *mut c_char {
    CString::new(value)
        .expect("JNI result cannot contain NUL")
        .into_raw()
}

unsafe fn string_ref<'a>(value: *const c_char) -> Result<&'a str, String> {
    if value.is_null() {
        return Err("JNI string argument is null".to_owned());
    }
    CStr::from_ptr(value)
        .to_str()
        .map_err(|error| error.to_string())
}

#[no_mangle]
pub extern "C" fn open_quartz_jni_free_string(value: *mut c_char) {
    if value.is_null() {
        return;
    }
    unsafe {
        drop(CString::from_raw(value));
    }
}

#[no_mangle]
pub extern "C" fn open_quartz_jni_create_client() -> u64 {
    table()
        .lock()
        .expect("JNI handle table poisoned")
        .create_client()
}

#[no_mangle]
pub extern "C" fn open_quartz_jni_release_client(handle: u64) -> bool {
    table()
        .lock()
        .expect("JNI handle table poisoned")
        .release_client(handle)
}

#[no_mangle]
pub extern "C" fn open_quartz_jni_sdk_version() -> *mut c_char {
    string_ptr(env!("CARGO_PKG_VERSION").to_owned())
}

#[no_mangle]
pub extern "C" fn open_quartz_jni_create_project(client: u64, name: *const c_char) -> u64 {
    let name = unsafe { string_ref(name) };
    let Ok(name) = name else { return 0 };
    table()
        .lock()
        .expect("JNI handle table poisoned")
        .create_project(client, name)
        .unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn open_quartz_jni_create_player(project: u64) -> u64 {
    table()
        .lock()
        .expect("JNI handle table poisoned")
        .create_player(project)
        .unwrap_or(0)
}

pub extern "C" fn open_quartz_jni_player_play(player: u64) -> *mut c_char {
    let result = table()
        .lock()
        .expect("JNI handle table poisoned")
        .with_player(player, |player| {
            player.play().map_err(|error| error.to_json())
        });
    match result {
        Ok(()) => ptr::null_mut(),
        Err(error) => string_ptr(error),
    }
}

#[no_mangle]
pub extern "C" fn open_quartz_jni_player_pause(player: u64) -> *mut c_char {
    let result = table()
        .lock()
        .expect("JNI handle table poisoned")
        .with_player(player, |player| {
            player.pause().map_err(|error| error.to_json())
        });
    match result {
        Ok(()) => ptr::null_mut(),
        Err(error) => string_ptr(error),
    }
}

#[no_mangle]
pub extern "C" fn open_quartz_jni_player_resume(player: u64) -> *mut c_char {
    let result = table()
        .lock()
        .expect("JNI handle table poisoned")
        .with_player(player, |player| {
            player.resume().map_err(|error| error.to_json())
        });
    match result {
        Ok(()) => ptr::null_mut(),
        Err(error) => string_ptr(error),
    }
}

#[no_mangle]
pub extern "C" fn open_quartz_jni_player_stop(player: u64) -> *mut c_char {
    let result = table()
        .lock()
        .expect("JNI handle table poisoned")
        .with_player(player, |player| {
            player.stop().map_err(|error| error.to_json())
        });
    match result {
        Ok(()) => ptr::null_mut(),
        Err(error) => string_ptr(error),
    }
}

#[no_mangle]
pub extern "C" fn open_quartz_jni_release_player(player: u64) -> bool {
    table()
        .lock()
        .expect("JNI handle table poisoned")
        .release_player(player)
}
