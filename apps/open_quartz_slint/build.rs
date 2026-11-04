fn main() {
    let config = slint_build::CompilerConfiguration::new()
        .with_include_paths(vec!["../../crates/slint_code_editor/ui".into()])
        .with_debug_info(std::env::var_os("CARGO_FEATURE_UI_AUTOMATION").is_some());
    slint_build::compile_with_config("ui/app.slint", config)
        .expect("Cannot compile the Slint application UI");
}
