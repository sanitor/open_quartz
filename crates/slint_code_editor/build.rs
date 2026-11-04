fn main() {
    slint_build::compile_with_config(
        "ui/demo.slint",
        slint_build::CompilerConfiguration::new().with_debug_info(true),
    )
    .unwrap();
}
