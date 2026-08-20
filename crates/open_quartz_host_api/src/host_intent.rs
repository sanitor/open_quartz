use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use open_quartz_schema::OnnxTask;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostResourceIntentRequest {
    pub host: HostResourceTarget,
    #[serde(default)]
    pub previous_graph: Option<HostGraphSnapshot>,
    pub graph: HostGraphSnapshot,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum HostResourceTarget {
    Browser,
    Native,
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
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
pub enum HostResourceIntent {
    AttachVideo {
        node_id: String,
        key: String,
        kind: VideoSourceKind,
        source: String,
        looping: bool,
        playback_rate: f64,
    },
    UpdateVideo {
        node_id: String,
        key: String,
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
    LoadOnnx {
        node_id: String,
        key: String,
        model_id: String,
        task: OnnxTask,
        #[serde(skip_serializing_if = "Option::is_none")]
        model_path: Option<String>,
        target_size: u32,
        score_threshold: f64,
        iou_threshold: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        download: Option<OnnxDownloadIntent>,
    },
    UnloadOnnx {
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
pub struct OnnxDownloadIntent {
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
struct OnnxResource {
    key: String,
    model_id: String,
    task: OnnxTask,
    model_path: Option<String>,
    target_size: u32,
    score_threshold: f64,
    iou_threshold: f64,
    download: Option<OnnxDownloadIntent>,
}

pub fn plan_host_resource_intents(
    request: HostResourceIntentRequest,
) -> Result<HostResourceIntentPlan, String> {
    let previous_graph = request.previous_graph.as_ref();
    let graph = match request.host {
        HostResourceTarget::Browser => request.graph.clone(),
        HostResourceTarget::Native => strip_native_resource_payloads(&request.graph),
    };
    let intents = match request.host {
        HostResourceTarget::Browser => browser_intents(previous_graph, &request.graph),
        HostResourceTarget::Native => native_intents(previous_graph, &request.graph)?,
    };
    Ok(HostResourceIntentPlan { graph, intents })
}

fn browser_intents(
    previous_graph: Option<&HostGraphSnapshot>,
    graph: &HostGraphSnapshot,
) -> Vec<HostResourceIntent> {
    let previous = video_resources(previous_graph, HostResourceTarget::Browser);
    let current = video_resources(Some(graph), HostResourceTarget::Browser);
    let mut intents = Vec::new();
    for (node_id, resource) in &current {
        match previous.get(node_id) {
            Some(prior) if prior.key == resource.key => {
                intents.push(HostResourceIntent::UpdateVideo {
                    node_id: node_id.clone(),
                    key: resource.key.clone(),
                    looping: resource.looping,
                    playback_rate: resource.playback_rate,
                });
            }
            _ => intents.push(HostResourceIntent::AttachVideo {
                node_id: node_id.clone(),
                key: resource.key.clone(),
                kind: resource.kind,
                source: resource.source.clone(),
                looping: resource.looping,
                playback_rate: resource.playback_rate,
            }),
        }
    }
    for node_id in previous.keys() {
        if !current.contains_key(node_id) {
            intents.push(HostResourceIntent::DetachVideo {
                node_id: node_id.clone(),
            });
        }
    }
    intents
}

fn native_intents(
    previous_graph: Option<&HostGraphSnapshot>,
    graph: &HostGraphSnapshot,
) -> Result<Vec<HostResourceIntent>, String> {
    let previous_video = video_resources(previous_graph, HostResourceTarget::Native);
    let current_video = video_resources(Some(graph), HostResourceTarget::Native);
    let previous_image = image_resources(previous_graph);
    let current_image = image_resources(Some(graph));
    let previous_onnx = onnx_resources(previous_graph)?;
    let current_onnx = onnx_resources(Some(graph))?;
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
    for (node_id, prior) in &previous_onnx {
        if current_onnx.get(node_id).map(|resource| &resource.key) != Some(&prior.key) {
            intents.push(HostResourceIntent::UnloadOnnx {
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
    for (node_id, resource) in current_onnx {
        if previous_onnx.get(&node_id).map(|prior| &prior.key) == Some(&resource.key) {
            continue;
        }
        intents.push(HostResourceIntent::LoadOnnx {
            node_id,
            key: resource.key,
            model_id: resource.model_id,
            task: resource.task,
            model_path: resource.model_path,
            target_size: resource.target_size,
            score_threshold: resource.score_threshold,
            iou_threshold: resource.iou_threshold,
            download: resource.download,
        });
    }
    Ok(intents)
}

fn video_resources(
    graph: Option<&HostGraphSnapshot>,
    target: HostResourceTarget,
) -> BTreeMap<String, VideoResource> {
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
        let source = match (target, kind) {
            (HostResourceTarget::Browser, VideoSourceKind::Camera) => {
                string_field(data, "videoDeviceId").unwrap_or("default").to_owned()
            }
            (HostResourceTarget::Browser, VideoSourceKind::File) => {
                let Some(url) = string_field(data, "videoUrl") else {
                    continue;
                };
                url.to_owned()
            }
            (HostResourceTarget::Native, VideoSourceKind::Camera) => {
                let Some(device_id) = string_field(data, "videoDeviceId") else {
                    continue;
                };
                device_id.to_owned()
            }
            (HostResourceTarget::Native, VideoSourceKind::File) => {
                let Some(path) = string_field(data, "videoFilePath") else {
                    continue;
                };
                path.to_owned()
            }
        };
        let looping = bool_field(data, "videoLoop").unwrap_or(true);
        let playback_rate = number_field(data, "videoPlaybackRate").unwrap_or(1.0);
        let key = match target {
            HostResourceTarget::Browser => match kind {
                VideoSourceKind::Camera => format!("camera:{source}"),
                VideoSourceKind::File => format!("file:{source}"),
            },
            HostResourceTarget::Native => format!(
                "{}|{}|{}|{}",
                match kind {
                    VideoSourceKind::Camera => "camera",
                    VideoSourceKind::File => "file",
                },
                source,
                looping,
                playback_rate
            ),
        };
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

fn onnx_resources(
    graph: Option<&HostGraphSnapshot>,
) -> Result<BTreeMap<String, OnnxResource>, String> {
    let mut resources = BTreeMap::new();
    let Some(graph) = graph else {
        return Ok(resources);
    };
    let catalog = open_quartz_execution::catalog::onnx_models()
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
        if string_field(data, "type") != Some("onnx") {
            continue;
        }
        let catalog_id = string_field(data, "onnxCatalogId");
        let model_id = string_field(data, "onnxModelId").or(catalog_id);
        let Some(model_id) = model_id else {
            continue;
        };
        let catalog_entry = catalog_id.and_then(|id| catalog.get(id));
        let task = catalog_entry
            .map(|entry| entry.task)
            .unwrap_or(OnnxTask::Generic);
        let params = data.get("onnxParams");
        let target_size = number_param(params, "targetSize")
            .or_else(|| number_field(data, "onnxTargetSize"))
            .unwrap_or(640.0) as u32;
        let score_threshold = number_param(params, "scoreThreshold")
            .or_else(|| number_field(data, "onnxScoreThreshold"))
            .unwrap_or(0.25);
        let iou_threshold = number_param(params, "iouThreshold")
            .or_else(|| number_field(data, "onnxIouThreshold"))
            .unwrap_or(0.45);
        let model_path = string_field(data, "onnxCustomPath").map(str::to_owned);
        let key = format!(
            "{}|{}|{:?}|{}|{}|{}",
            model_id,
            model_path.as_deref().unwrap_or(""),
            task,
            target_size,
            score_threshold,
            iou_threshold
        );
        let download = if model_path.is_none() {
            catalog_entry.map(|entry| OnnxDownloadIntent {
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
            OnnxResource {
                key,
                model_id: model_id.to_owned(),
                task,
                model_path,
                target_size,
                score_threshold,
                iou_threshold,
                download,
            },
        );
    }
    Ok(resources)
}

fn strip_native_resource_payloads(graph: &HostGraphSnapshot) -> HostGraphSnapshot {
    HostGraphSnapshot {
        nodes: graph.nodes.iter().map(strip_node_resource_payloads).collect(),
        edges: graph.edges.clone(),
    }
}

fn strip_node_resource_payloads(node: &Value) -> Value {
    let mut node = node.clone();
    let Some(data) = node.get_mut("data").and_then(Value::as_object_mut) else {
        return node;
    };
    for field in [
        "imageDataUrl",
        "rawDataUrl",
        "videoUrl",
        "videoFilePath",
        "videoDeviceId",
        "onnxCustomPath",
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

fn number_param(params: Option<&Value>, field: &str) -> Option<f64> {
    params.and_then(|value| value.get(field)).and_then(Value::as_f64)
}
