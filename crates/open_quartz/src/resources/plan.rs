use std::collections::BTreeMap;

use crate::inference::{BackendPreference, ExecutionPolicy};
use crate::project::model::InferenceTask;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostResourceIntentRequest {
    #[serde(default)]
    pub previous_graph: Option<HostGraphSnapshot>,
    pub graph: HostGraphSnapshot,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HostGraphSnapshot {
    pub nodes: Vec<Value>,
    #[serde(default)]
    pub edges: Vec<Value>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HostResourceIntentPlan {
    pub graph: HostGraphSnapshot,
    pub intents: Vec<HostResourceIntent>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum HostResourceIntent {
    AttachVideo {
        node_id: String,
        key: String,
        kind: VideoSourceKind,
        source: String,
        looping: bool,
        playback_rate: f64,
    },
    DetachVideo {
        node_id: String,
    },
    UploadImage {
        node_id: String,
        key: String,
        source: ImageSourceIntent,
    },
    RemoveImage {
        node_id: String,
    },
    LoadInference {
        node_id: String,
        key: String,
        model_id: String,
        task: InferenceTask,
        #[serde(skip_serializing_if = "Option::is_none")]
        model_path: Option<String>,
        target_size: u32,
        score_threshold: f64,
        iou_threshold: f64,
        policy: ExecutionPolicy,
        #[serde(skip_serializing_if = "Option::is_none")]
        download: Option<InferenceDownloadIntent>,
    },
    UnloadInference {
        node_id: String,
    },
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum VideoSourceKind {
    File,
    Camera,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ImageSourceIntent {
    Encoded {
        source: String,
    },
    Raw {
        source: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        format: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        width: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        height: Option<u32>,
    },
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InferenceDownloadIntent {
    pub model_id: String,
    pub url: String,
    pub expected_size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq)]
struct VideoResource {
    key: String,
    kind: VideoSourceKind,
    source: String,
    looping: bool,
    playback_rate: f64,
}

#[derive(Clone, Debug, PartialEq)]
struct ImageResource {
    key: String,
    source: ImageSourceIntent,
}

#[derive(Clone, Debug, PartialEq)]
struct InferenceResource {
    key: String,
    model_id: String,
    task: InferenceTask,
    model_path: Option<String>,
    target_size: u32,
    score_threshold: f64,
    iou_threshold: f64,
    policy: ExecutionPolicy,
    download: Option<InferenceDownloadIntent>,
}

pub fn plan_host_resource_intents(
    request: HostResourceIntentRequest,
) -> Result<HostResourceIntentPlan, String> {
    let previous_graph = request.previous_graph.as_ref();
    let graph = strip_native_resource_payloads(&request.graph);
    let intents = native_intents(previous_graph, &request.graph)?;
    Ok(HostResourceIntentPlan { graph, intents })
}

fn native_intents(
    previous_graph: Option<&HostGraphSnapshot>,
    graph: &HostGraphSnapshot,
) -> Result<Vec<HostResourceIntent>, String> {
    let previous_video = video_resources(previous_graph);
    let current_video = video_resources(Some(graph));
    let previous_image = image_resources(previous_graph);
    let current_image = image_resources(Some(graph));
    let previous_inference = inference_resources(previous_graph)?;
    let current_inference = inference_resources(Some(graph))?;
    let mut intents = Vec::new();

    for (node_id, prior) in &previous_video {
        if current_video.get(node_id).map(|resource| &resource.key) != Some(&prior.key) {
            intents.push(HostResourceIntent::DetachVideo {
                node_id: node_id.clone(),
            });
        }
    }
    for (node_id, prior) in &previous_image {
        if current_image.get(node_id).map(|resource| &resource.key) != Some(&prior.key) {
            intents.push(HostResourceIntent::RemoveImage {
                node_id: node_id.clone(),
            });
        }
    }
    for (node_id, prior) in &previous_inference {
        if current_inference.get(node_id).map(|resource| &resource.key) != Some(&prior.key) {
            intents.push(HostResourceIntent::UnloadInference {
                node_id: node_id.clone(),
            });
        }
    }

    for (node_id, resource) in current_video {
        if previous_video.get(&node_id).map(|prior| &prior.key) == Some(&resource.key) {
            continue;
        }
        intents.push(HostResourceIntent::AttachVideo {
            node_id,
            key: resource.key,
            kind: resource.kind,
            source: resource.source,
            looping: resource.looping,
            playback_rate: resource.playback_rate,
        });
    }
    for (node_id, resource) in current_image {
        if previous_image.get(&node_id).map(|prior| &prior.key) == Some(&resource.key) {
            continue;
        }
        intents.push(HostResourceIntent::UploadImage {
            node_id,
            key: resource.key,
            source: resource.source,
        });
    }
    for (node_id, resource) in current_inference {
        if previous_inference.get(&node_id).map(|prior| &prior.key) == Some(&resource.key) {
            continue;
        }
        intents.push(HostResourceIntent::LoadInference {
            node_id,
            key: resource.key,
            model_id: resource.model_id,
            task: resource.task,
            model_path: resource.model_path,
            target_size: resource.target_size,
            score_threshold: resource.score_threshold,
            iou_threshold: resource.iou_threshold,
            policy: resource.policy,
            download: resource.download,
        });
    }
    Ok(intents)
}

fn video_resources(graph: Option<&HostGraphSnapshot>) -> BTreeMap<String, VideoResource> {
    let mut resources = BTreeMap::new();
    let Some(graph) = graph else {
        return resources;
    };
    for node in &graph.nodes {
        let Some(node_id) = node_id(node) else {
            continue;
        };
        let Some(data) = node.get("data") else {
            continue;
        };
        if string_field(data, "type") != Some("input")
            || string_field(data, "inputMode") != Some("video")
        {
            continue;
        }
        let kind = match string_field(data, "videoSourceType") {
            Some("camera") => VideoSourceKind::Camera,
            _ => VideoSourceKind::File,
        };
        let source = match kind {
            VideoSourceKind::Camera => {
                let Some(device_id) = string_field(data, "videoDeviceId") else {
                    continue;
                };
                device_id.to_owned()
            }
            VideoSourceKind::File => {
                let Some(path) = string_field(data, "videoFilePath") else {
                    continue;
                };
                path.to_owned()
            }
        };
        let looping = bool_field(data, "videoLoop").unwrap_or(true);
        let playback_rate = number_field(data, "videoPlaybackRate").unwrap_or(1.0);
        let key = format!(
            "{}|{}|{}|{}",
            match kind {
                VideoSourceKind::Camera => "camera",
                VideoSourceKind::File => "file",
            },
            source,
            looping,
            playback_rate
        );
        resources.insert(
            node_id.to_owned(),
            VideoResource {
                key,
                kind,
                source,
                looping,
                playback_rate,
            },
        );
    }
    resources
}

fn image_resources(graph: Option<&HostGraphSnapshot>) -> BTreeMap<String, ImageResource> {
    let mut resources = BTreeMap::new();
    let Some(graph) = graph else {
        return resources;
    };
    for node in &graph.nodes {
        let Some(node_id) = node_id(node) else {
            continue;
        };
        let Some(data) = node.get("data") else {
            continue;
        };
        if string_field(data, "type") != Some("input")
            || string_field(data, "inputDataType") != Some("sampler2D")
            || string_field(data, "inputMode") == Some("video")
        {
            continue;
        }
        let source = if let Some(source) = string_field(data, "imageDataUrl") {
            ImageSourceIntent::Encoded {
                source: source.to_owned(),
            }
        } else if let Some(source) = string_field(data, "rawDataUrl") {
            ImageSourceIntent::Raw {
                source: source.to_owned(),
                format: string_field(data, "fbFormat").map(str::to_owned),
                width: u32_field(data, "fbWidth"),
                height: u32_field(data, "fbHeight"),
            }
        } else {
            continue;
        };
        let key = match &source {
            ImageSourceIntent::Encoded { source } => {
                format!("{source}|||")
            }
            ImageSourceIntent::Raw {
                source,
                format,
                width,
                height,
            } => format!(
                "{}|{}|{}|{}",
                source,
                format.as_deref().unwrap_or(""),
                width.map(|value| value.to_string()).unwrap_or_default(),
                height.map(|value| value.to_string()).unwrap_or_default()
            ),
        };
        resources.insert(node_id.to_owned(), ImageResource { key, source });
    }
    resources
}

fn inference_resources(
    graph: Option<&HostGraphSnapshot>,
) -> Result<BTreeMap<String, InferenceResource>, String> {
    let mut resources = BTreeMap::new();
    let Some(graph) = graph else {
        return Ok(resources);
    };
    let catalog = crate::project::catalog::inference_models()
        .into_iter()
        .map(|model| (model.id.clone(), model))
        .collect::<BTreeMap<_, _>>();
    for node in &graph.nodes {
        let Some(node_id) = node_id(node) else {
            continue;
        };
        let Some(data) = node.get("data") else {
            continue;
        };
        let kind = data.get("type").or_else(|| node.get("type"));
        if !matches!(kind.and_then(Value::as_str), Some("onnx" | "inference")) {
            continue;
        }
        let policy = effective_inference_policy(data)
            .map_err(|error| format!("Inference node {node_id}: {error}"))?;
        let catalog_id =
            inference_field(data, "inferenceCatalogId", "onnxCatalogId").and_then(Value::as_str);
        let model_id = inference_field(data, "inferenceModelId", "onnxModelId")
            .and_then(Value::as_str)
            .or(catalog_id);
        let Some(model_id) = model_id else {
            continue;
        };
        let catalog_entry = catalog_id.and_then(|id| catalog.get(id));
        let task = match inference_field(data, "inferenceTask", "onnxTask") {
            Some(value) if !value.is_null() => serde_json::from_value(value.clone())
                .map_err(|error| format!("Inference node {node_id}: invalid task: {error}"))?,
            _ => catalog_entry
                .map(|entry| entry.task)
                .unwrap_or(InferenceTask::Generic),
        };
        let target_size =
            inference_number(data, "targetSize", "inferenceTargetSize", "onnxTargetSize")
                .unwrap_or(640.0) as u32;
        let score_threshold = inference_number(
            data,
            "scoreThreshold",
            "inferenceScoreThreshold",
            "onnxScoreThreshold",
        )
        .unwrap_or(0.25);
        let iou_threshold = inference_number(
            data,
            "iouThreshold",
            "inferenceIouThreshold",
            "onnxIouThreshold",
        )
        .unwrap_or(0.45);
        let model_path = [
            "inferenceModelPath",
            "inferenceCustomPath",
            "onnxModelPath",
            "onnxCustomPath",
            "customPath",
        ]
        .into_iter()
        .find_map(|field| data.get(field))
        .and_then(Value::as_str)
        .map(str::to_owned);
        let key = serde_json::to_string(&(
            model_id,
            &model_path,
            task,
            target_size,
            score_threshold,
            iou_threshold,
            &policy,
        ))
        .map_err(|error| format!("Cannot encode inference resource key: {error}"))?;
        let download = if model_path.is_none() {
            catalog_entry.map(|entry| InferenceDownloadIntent {
                model_id: model_id.to_owned(),
                url: entry.download_url.clone(),
                expected_size: entry.file_size,
                sha256: entry.sha256.clone(),
            })
        } else {
            None
        };
        resources.insert(
            node_id.to_owned(),
            InferenceResource {
                key,
                model_id: model_id.to_owned(),
                task,
                model_path,
                target_size,
                score_threshold,
                iou_threshold,
                policy,
                download,
            },
        );
    }
    Ok(resources)
}

fn inference_field<'a>(data: &'a Value, canonical: &str, legacy: &str) -> Option<&'a Value> {
    data.get(canonical).or_else(|| data.get(legacy))
}

fn inference_number(data: &Value, param: &str, canonical: &str, legacy: &str) -> Option<f64> {
    data.get("inferenceParams")
        .and_then(|params| params.get(param))
        .or_else(|| data.get(canonical))
        .or_else(|| data.get("onnxParams").and_then(|params| params.get(param)))
        .or_else(|| data.get(legacy))
        .or_else(|| data.get("params").and_then(|params| params.get(param)))
        .and_then(Value::as_f64)
}

/// Resolve canonical policy first, preserving explicitly stored legacy DirectML intent.
/// Invalid policy objects and legacy preference values are errors, never defaults.
pub fn effective_inference_policy(data: &Value) -> Result<ExecutionPolicy, String> {
    if let Some(value) = inference_field(data, "inferencePolicy", "onnxPolicy") {
        return serde_json::from_value(value.clone())
            .map_err(|error| format!("Invalid inference policy: {error}"));
    }
    let preference = data
        .get("inferencePreferDirectMl")
        .or_else(|| {
            data.get("inferenceParams")
                .and_then(|params| params.get("preferDirectMl"))
        })
        .or_else(|| data.get("preferDirectMl"))
        .or_else(|| data.get("onnxPreferDirectMl"))
        .or_else(|| {
            data.get("onnxParams")
                .and_then(|params| params.get("preferDirectMl"))
        })
        .or_else(|| {
            data.get("params")
                .and_then(|params| params.get("preferDirectMl"))
        });
    let mut policy = ExecutionPolicy::resident();
    if let Some(value) = preference {
        let prefer_direct_ml = value
            .as_bool()
            .ok_or_else(|| "Invalid preferDirectMl: expected a boolean".to_owned())?;
        if prefer_direct_ml {
            policy = ExecutionPolicy {
                backend: BackendPreference::OrtDirectMl,
                ..ExecutionPolicy::default()
            };
        }
    }
    Ok(policy)
}

fn strip_native_resource_payloads(graph: &HostGraphSnapshot) -> HostGraphSnapshot {
    HostGraphSnapshot {
        nodes: graph
            .nodes
            .iter()
            .map(strip_node_resource_payloads)
            .collect(),
        edges: graph.edges.clone(),
    }
}

fn strip_node_resource_payloads(node: &Value) -> Value {
    let mut node = node.clone();
    let is_inference = matches!(
        node.get("data")
            .and_then(|data| data.get("type"))
            .or_else(|| node.get("type"))
            .and_then(Value::as_str),
        Some("inference" | "onnx")
    );
    if node.get("type").and_then(Value::as_str) == Some("onnx") {
        node["type"] = Value::String("inference".into());
    }
    let Some(data) = node.get_mut("data").and_then(Value::as_object_mut) else {
        return node;
    };
    if data.get("type").and_then(Value::as_str) == Some("onnx") {
        data.insert("type".into(), Value::String("inference".into()));
    }
    // Migrate semantic keys only; values such as model IDs, filenames and port IDs stay exact.
    let legacy_keys = data
        .keys()
        .filter(|key| key.starts_with("onnx"))
        .cloned()
        .collect::<Vec<_>>();
    for legacy in legacy_keys {
        let canonical = format!("inference{}", &legacy[4..]);
        if let Some(mut value) = data.remove(&legacy) {
            if legacy == "onnxParams" {
                if let Some(params) = value.as_object_mut() {
                    for (param, canonical_field, legacy_field) in [
                        ("targetSize", "inferenceTargetSize", "onnxTargetSize"),
                        ("scoreThreshold", "inferenceScoreThreshold", "onnxScoreThreshold"),
                        ("iouThreshold", "inferenceIouThreshold", "onnxIouThreshold"),
                    ] {
                        if data.contains_key(canonical_field) || data.contains_key(legacy_field) {
                            params.remove(param);
                        }
                    }
                    if ["inferencePreferDirectMl", "onnxPreferDirectMl", "preferDirectMl"]
                        .iter().any(|key| data.contains_key(*key)) {
                        params.remove("preferDirectMl");
                    }
                }
                if let (Some(prior), Some(current)) = (
                    value.as_object(),
                    data.get_mut(&canonical).and_then(Value::as_object_mut),
                ) {
                    for (key, value) in prior {
                        current.entry(key.clone()).or_insert_with(|| value.clone());
                    }
                    continue;
                }
            }
            data.entry(canonical).or_insert(value);
        }
    }
    if is_inference {
        if let Some(mut legacy_params) = data.remove("params") {
            if let Some(params) = legacy_params.as_object_mut() {
                // A bare legacy parameter must remain below an explicit top-level value
                // after saving it under the canonical parameter collection.
                for (param, field) in [
                    ("targetSize", "inferenceTargetSize"),
                    ("scoreThreshold", "inferenceScoreThreshold"),
                    ("iouThreshold", "inferenceIouThreshold"),
                ] {
                    if data.contains_key(field) {
                        params.remove(param);
                    }
                }
                if data.contains_key("inferencePreferDirectMl")
                    || data.contains_key("preferDirectMl")
                {
                    params.remove("preferDirectMl");
                }
            }
            if let (Some(prior), Some(current)) = (
                legacy_params.as_object(),
                data.get_mut("inferenceParams")
                    .and_then(Value::as_object_mut),
            ) {
                for (key, value) in prior {
                    current.entry(key.clone()).or_insert_with(|| value.clone());
                }
            } else {
                data.entry("inferenceParams".to_owned())
                    .or_insert(legacy_params);
            }
        }
    }
    for alias in ["inferenceCustomPath", "customPath"] {
        if let Some(value) = data.remove(alias) {
            data.entry("inferenceModelPath".to_owned()).or_insert(value);
        }
    }
    for field in [
        "imageDataUrl",
        "rawDataUrl",
        "videoUrl",
        "videoFilePath",
        "videoDeviceId",
        "inferenceModelPath",
    ] {
        data.remove(field);
    }
    node
}

fn node_id(node: &Value) -> Option<&str> {
    node.get("id").and_then(Value::as_str)
}

fn string_field<'a>(data: &'a Value, field: &str) -> Option<&'a str> {
    data.get(field).and_then(Value::as_str)
}

fn bool_field(data: &Value, field: &str) -> Option<bool> {
    data.get(field).and_then(Value::as_bool)
}

fn number_field(data: &Value, field: &str) -> Option<f64> {
    data.get(field).and_then(Value::as_f64)
}

fn u32_field(data: &Value, field: &str) -> Option<u32> {
    data.get(field)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
}

#[cfg(test)]
#[path = "plan_coverage_tests.rs"]
mod coverage_tests;
