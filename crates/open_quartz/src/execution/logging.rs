pub(crate) fn sdk_log(level: &str, event: &str, fields: &str) {
    eprintln!("[oq:rust] {level} {event} {fields}");
}
