#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os().nth(1).ok_or("usage: cargo run -p oq-media --features videotoolbox --example decode -- VIDEO")?;
    let mut session = oq_media::Session::open(path)?;
    let mut count = 0;
    while let Some(lease) = session.next_frame()? {
        if count == 0 { println!("pipeline: {:?}", session.actual_pipeline()); }
        let frame = &lease.frame;
        println!("{}: {}x{} {:?} {:?} pts={:?}", lease.sequence, frame.width(), frame.height(), frame.format(), frame.color(), frame.pts_ns());
        count += 1;
    }
    if count == 0 { return Err("video contained no frames".into()); }
    Ok(())
}
#[cfg(not(target_os = "macos"))]
fn main() { eprintln!("VideoToolbox decoding requires macOS"); std::process::exit(1); }
