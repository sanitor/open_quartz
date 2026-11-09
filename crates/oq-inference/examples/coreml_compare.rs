//! Reproducible CPU-reference/Core ML EP comparison for the catalog's 3× SR model.
//! ORT_DYLIB_PATH=... cargo run -p oq-inference --features ort --example coreml_compare -- MODEL.onnx REPORT_DIR
use oq_inference::{BackendPreference, ExecutionPolicy, FallbackPolicy, InferenceSession};
use std::{collections::BTreeMap, error::Error, path::PathBuf, time::Instant};

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let model_path = PathBuf::from(
        args.next()
            .ok_or("expected super-resolution-3x ONNX path")?,
    );
    let directory = PathBuf::from(
        args.next()
            .ok_or("expected output directory for profile/report")?,
    );
    if args.next().is_some() {
        return Err("unexpected arguments".into());
    }
    std::fs::create_dir_all(&directory)?;
    let model = std::fs::read(model_path)?;
    let input: Vec<f32> = (0..224 * 224)
        .map(|i| ((i % 224) as f32 * 0.7 + (i / 224) as f32 * 0.3) / 223.0)
        .collect();
    let mut cpu = InferenceSession::from_memory(&model)?;
    let cpu_info = cpu.info().clone();
    let reference = cpu.run_f32(input.clone(), vec![1, 1, 224, 224])?;
    let policy = ExecutionPolicy {
        backend: BackendPreference::OrtCoreMl,
        fallback: FallbackPolicy::AllowCpu,
        ..Default::default()
    };
    let started = Instant::now();
    let mut native = InferenceSession::from_memory_with_policy(&model, &policy)?;
    let preparation_ms = started.elapsed().as_secs_f64() * 1000.0;
    let native_info = native.info().clone();
    let result = native.run_f32(input.clone(), vec![1, 1, 224, 224])?;
    if result.shape != reference.shape || result.data.len() != reference.data.len() {
        return Err("Core ML result shape differs from CPU reference".into());
    }
    let mut maximum_error = 0.0_f32;
    let mut total_error = 0.0_f64;
    for (&actual, &expected) in result.data.iter().zip(&reference.data) {
        if !actual.is_finite() || !expected.is_finite() {
            return Err("non-finite output".into());
        }
        let error = (actual - expected).abs();
        maximum_error = maximum_error.max(error);
        total_error += f64::from(error);
    }
    let mean_error = total_error / reference.data.len() as f64;
    let mut cpu_ms = Vec::new();
    let mut coreml_ms = Vec::new();
    for _ in 0..5 {
        let started = Instant::now();
        cpu.run_f32(input.clone(), vec![1, 1, 224, 224])?;
        cpu_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        let started = Instant::now();
        native.run_f32(input.clone(), vec![1, 1, 224, 224])?;
        coreml_ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    // Provider proof uses a separate session so profiling overhead is outside latency samples.
    let mut profiled =
        InferenceSession::from_memory_with_profile(&model, &policy, &directory.join("coreml"))?;
    profiled.run_f32(input.clone(), vec![1, 1, 224, 224])?;
    let trace_path = profiled.end_profiling()?;
    let trace: serde_json::Value = serde_json::from_slice(&std::fs::read(&trace_path)?)?;
    let mut provider_events = BTreeMap::<String, u64>::new();
    for event in trace.as_array().ok_or("invalid ORT profile")? {
        if let Some(provider) = event["args"]["provider"].as_str() {
            *provider_events.entry(provider.to_owned()).or_default() += 1;
        }
    }
    let report = serde_json::json!({
        "cpu": cpu_info, "native": native_info, "preparationMs": preparation_ms,
        "outputShape": result.shape, "maxAbsoluteError": maximum_error, "meanAbsoluteError": mean_error,
        "cpuInvocationMs": cpu_ms, "coremlInvocationMs": coreml_ms,
        "profilePath": trace_path, "providerExecutionEvents": provider_events,
        "scope": "host FP32 tensors; Core ML operator dispatch does not prove GPU/ANE placement or residency",
    });
    let report_path = directory.join("comparison.json");
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    if native_info.plan.configured_backend != BackendPreference::OrtCoreMl {
        return Err("Core ML activation fell back to CPU; see comparison report".into());
    }
    if !provider_events
        .iter()
        .any(|(provider, &events)| provider == "CoreMLExecutionProvider" && events > 0)
    {
        return Err(
            "profile has no Core ML execution events; registration alone is not native execution"
                .into(),
        );
    }
    if maximum_error > 0.02 || mean_error > 0.002 {
        return Err(
            "Core ML exceeds declared SR comparison tolerance (max 0.02, mean 0.002)".into(),
        );
    }
    Ok(())
}
