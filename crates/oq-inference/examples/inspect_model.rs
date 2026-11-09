//! Inspect declared model interfaces without loading any inference runtime.
//! cargo run -p oq-inference --example inspect_model -- MODEL.onnx
use std::{error::Error, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = PathBuf::from(args.next().ok_or("expected ONNX model path")?);
    if args.next().is_some() {
        return Err("unexpected arguments".into());
    }
    let bytes = std::fs::read(path)?;
    let interface = oq_inference::inspect_onnx(&bytes)?;
    println!("{}", serde_json::to_string_pretty(&interface)?);
    Ok(())
}
