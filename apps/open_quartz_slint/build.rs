fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/runtime");
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Resources/runtime");
    }
    let config = slint_build::CompilerConfiguration::new()
        .with_include_paths(vec![std::env::var_os("DEP_SLINT_CODE_EDITOR_UI_DIR").expect("Slint source directory").into()])
        .with_debug_info(std::env::var_os("CARGO_FEATURE_UI_AUTOMATION").is_some());
    slint_build::compile_with_config("ui/app.slint", config)
        .expect("Cannot compile the Slint application UI");
}
