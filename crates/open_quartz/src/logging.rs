#[cfg(all(target_arch = "wasm32", feature = "browser"))]
use wasm_bindgen::prelude::*;

#[cfg(all(target_arch = "wasm32", feature = "browser"))]
#[wasm_bindgen(
    inline_js = "export function runtime_log(level, event, fields) { const fn = console[level] || console.log; fn('[oq:wasm] ' + event, fields || {}); }"
)]
extern "C" {
    fn runtime_log(level: &str, event: &str, fields: &str);
}

pub(crate) fn sdk_log(level: &str, event: &str, fields: &str) {
    #[cfg(all(target_arch = "wasm32", feature = "browser"))]
    runtime_log(level, event, fields);
    #[cfg(not(all(target_arch = "wasm32", feature = "browser")))]
    eprintln!("[oq:rust] {level} {event} {fields}");
}
