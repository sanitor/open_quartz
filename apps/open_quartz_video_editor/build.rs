fn main() {
    let config = slint_build::CompilerConfiguration::new()
        .with_debug_info(std::env::var_os("CARGO_FEATURE_UI_AUTOMATION").is_some());
    slint_build::compile_with_config("ui/editor.slint", config)
        .expect("cannot compile OpenQuartz video editor UI");
}
