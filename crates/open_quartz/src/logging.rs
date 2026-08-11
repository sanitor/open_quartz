#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(
    inline_js = "export function runtime_log(level, event, fields) { const fn = console[level] || console.log; fn('[oq:wasm] ' + event, fields || {}); }"
)]
extern "C" {
    fn runtime_log(level: &str, event: &str, fields: &str);
}

pub(crate) fn sdk_log(level: &str, event: &str, fields: &str) {
    #[cfg(target_arch = "wasm32")]
    runtime_log(level, event, fields);
    #[cfg(not(target_arch = "wasm32"))]
    eprintln!("[oq:rust] {level} {event} {fields}");
}
