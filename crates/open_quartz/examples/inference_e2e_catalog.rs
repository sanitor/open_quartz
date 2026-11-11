// Canonical descriptors are deliberately exported without a second model list.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    serde_json::to_writer(std::io::stdout().lock(),
        &open_quartz::project::catalog::inference_models())?;
    println!();
    Ok(())
}
