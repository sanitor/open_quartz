use open_quartz::{Graph, InferenceSource, InferenceTask, NodeType, ProjectNode};
use serde_json::Value;

/// Recorded project metadata only; live inference sessions are projected separately.
pub(crate) fn inspect(
    node: &ProjectNode,
    graph: &Graph,
) -> (Vec<crate::InspectorInferenceRow>, Vec<crate::InspectorInferenceRow>) {
    if node.node_type != NodeType::Inference {
        return (Vec::new(), Vec::new());
    }
    let data = &node.data;
    let connected = data
        .inputs
        .iter()
        .filter(|port| {
            graph
                .edges
                .iter()
                .any(|edge| edge.target == node.id && edge.target_handle == port.id)
        })
        .count();
    let mut configuration = vec![
        row("Model", &data.label),
        row("Resident capability", "Metal GPU · super-resolution-3x / Super Resolution only (Conv, ReLU, PixelShuffle). Other models and tasks fail preparation; no host execution fallback."),
        row("Task", match data.inference_task {
            Some(InferenceTask::SuperResolution) => "Super resolution",
            Some(InferenceTask::BackgroundRemoval) => "Background removal",
            Some(InferenceTask::Detection) => "Object detection",
            Some(InferenceTask::Segmentation) => "Segmentation",
            Some(InferenceTask::StyleTransfer) => "Style transfer",
            Some(InferenceTask::Denoising) => "Denoising",
            Some(InferenceTask::DepthEstimation) => "Depth estimation",
            Some(InferenceTask::Generic) => "Generic",
            None => "Not recorded · metadata does not select a task",
        }),
        row(
            "Source",
            match data.inference_source {
                Some(InferenceSource::Catalog) => "Catalog",
                Some(InferenceSource::Custom) => "Custom",
                None => "Not recorded",
            },
        ),
        row(
            "Model ID",
            data.inference_model_id.as_deref().unwrap_or("Not recorded"),
        ),
        row(
            "Catalog ID",
            data.inference_catalog_id.as_deref().unwrap_or("Not recorded"),
        ),
        row(
            "Custom path (not checked)",
            data.inference_model_path.as_deref().unwrap_or("Not recorded"),
        ),
        row(
            "Input connections",
            &format!("{connected} / {} input ports connected", data.inputs.len()),
        ),
    ];
    for (label, value) in [
        (
            "Saved score threshold",
            data.inference_score_threshold.map(|v| v.to_string()),
        ),
        (
            "Saved IoU threshold",
            data.inference_iou_threshold.map(|v| v.to_string()),
        ),
        (
            "Saved target size",
            data.inference_target_size.map(|v| v.to_string()),
        ),
    ] {
        configuration.push(row(label, value.as_deref().unwrap_or("Not recorded")));
    }
    if let Some(params) = &data.inference_params {
        for (name, value) in params {
            let value =
                serde_json::to_string(value).expect("Inference parameter is a finite number or boolean");
            configuration.push(row(&format!("Saved parameter · {name}"), &value));
        }
    }
    let mut saved = Vec::new();
    for (key, legacy_key, label) in [
        ("inferenceCustomFileName", "onnxCustomFileName", "Saved file name"),
        ("inferenceStatus", "onnxStatus", "Saved status"),
        ("inferenceBackend", "onnxBackend", "Saved backend"),
        ("inferenceNativeBackend", "onnxNativeBackend", "Saved native backend"),
        ("inferenceError", "onnxError", "Saved error"),
    ] {
        if let Some(value) = data.extra.get(key).or_else(|| data.extra.get(legacy_key)) {
            saved.push(row(label, &saved_value(value)));
        }
    }
    if let Some(value) = data.extra.get("inferenceProgress").or_else(|| data.extra.get("onnxProgress")) {
        saved.push(row("Saved download progress", &saved_progress(value)));
    }
    (configuration, saved)
}

fn row(label: &str, value: &str) -> crate::InspectorInferenceRow {
    crate::InspectorInferenceRow {
        label: label.into(),
        value: value.into(),
    }
}
fn saved_value(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

fn saved_progress(value: &Value) -> String {
    match progress_percentage(value) {
        Some(value) => format!("{value:.1}% (saved, not a live download)"),
        None => format!(
            "Invalid saved progress: {} (expected a number from 0 to 1)",
            saved_value(value)
        ),
    }
}

fn progress_percentage(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
        .map(|value| value * 100.0)
}

#[cfg(test)]
mod tests {
    use super::progress_percentage;
    use serde_json::json;

    #[test]
    fn saved_progress_accepts_both_endpoints_and_keeps_zero_distinct_from_absence() {
        assert_eq!(progress_percentage(&json!(0)), Some(0.0));
        assert_eq!(progress_percentage(&json!(1)), Some(100.0));
        assert_eq!(progress_percentage(&json!(0.375)), Some(37.5));
    }

    #[test]
    fn invalid_saved_progress_cannot_masquerade_as_completed_download() {
        for value in [
            json!(-0.01),
            json!(1.01),
            json!("1"),
            json!(null),
            json!(true),
            json!({"status":"ready"}),
        ] {
            assert_eq!(progress_percentage(&value), None, "{value}");
        }
    }
}
