fn main() {
    // A .scr extracts its native runtime before invoking FFmpeg. Delay-load the
    // shared FFmpeg imports so the Windows loader does not require sidecar DLLs
    // beside the exported .scr before main() can extract them.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        println!("cargo:rustc-link-lib=delayimp");
        for library in [
            "avcodec-63.dll",
            "avformat-63.dll",
            "avutil-61.dll",
            "avdevice-63.dll",
            "avfilter-12.dll",
            "swresample-7.dll",
            "swscale-10.dll",
        ] {
            println!("cargo:rustc-link-arg=/DELAYLOAD:{library}");
        }
    }
}
