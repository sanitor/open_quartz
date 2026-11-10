fn main() {
    println!("cargo:rerun-if-changed=src/native/catalog_graph.m");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos")
        && std::env::var_os("CARGO_FEATURE_GPU").is_some()
    {
        cc::Build::new()
            .file("src/native/catalog_graph.m")
            .flag("-fobjc-arc")
            .flag("-fblocks")
            .flag("-Wno-deprecated-declarations")
            .compile("oq_catalog_graph");
        for framework in ["Foundation", "Metal", "MetalPerformanceShaders", "MetalPerformanceShadersGraph"] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
    }
}
