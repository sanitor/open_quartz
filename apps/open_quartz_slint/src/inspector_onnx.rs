use open_quartz_sdk::{Graph, NodeType, OnnxSource, ProjectNode};
use serde_json::Value;

/// Recorded project metadata only; live ONNX sessions are projected separately.
pub(crate) fn inspect(
    node: &ProjectNode,
    graph: &Graph,
) -> (Vec<crate::InspectorOnnxRow>, Vec<crate::InspectorOnnxRow>) {
    if node.node_type != NodeType::Onnx {
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
        row(
            "Source",
            match data.onnx_source {
                Some(OnnxSource::Catalog) => "Catalog",
                Some(OnnxSource::Custom) => "Custom",
                None => "Not recorded",
            },
        ),
        row(
            "Model ID",
            data.onnx_model_id.as_deref().unwrap_or("Not recorded"),
        ),
        row(
            "Catalog ID",
            data.onnx_catalog_id.as_deref().unwrap_or("Not recorded"),
        ),
        row(
            "Custom path (not checked)",
            data.onnx_custom_path.as_deref().unwrap_or("Not recorded"),
        ),
        row(
            "Input connections",
            &format!("{connected} / {} input ports connected", data.inputs.len()),
        ),
    ];
    for (label, value) in [
        (
            "Saved score threshold",
            data.onnx_score_threshold.map(|v| v.to_string()),
        ),
        (
            "Saved IoU threshold",
            data.onnx_iou_threshold.map(|v| v.to_string()),
        ),
        (
            "Saved target size",
            data.onnx_target_size.map(|v| v.to_string()),
        ),
    ] {
        configuration.push(row(label, value.as_deref().unwrap_or("Not recorded")));
    }
    if let Some(params) = &data.onnx_params {
        for (name, value) in params {
            let value =
                serde_json::to_string(value).expect("ONNX parameter is a finite number or boolean");
            configuration.push(row(&format!("Saved parameter · {name}"), &value));
        }
    }
    let mut saved = Vec::new();
    for (key, label) in [
        ("onnxCustomFileName", "Saved file name"),
        ("onnxStatus", "Saved status"),
        ("onnxBackend", "Saved backend"),
        ("onnxNativeBackend", "Saved native backend"),
        ("onnxError", "Saved error"),
    ] {
        if let Some(value) = data.extra.get(key) {
            saved.push(row(label, &saved_value(value)));
        }
    }
    if let Some(value) = data.extra.get("onnxProgress") {
        saved.push(row("Saved download progress", &saved_progress(value)));
    }
    (configuration, saved)
}

fn row(label: &str, value: &str) -> crate::InspectorOnnxRow {
    crate::InspectorOnnxRow {
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
