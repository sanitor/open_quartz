use super::*;
use serde_json::{json, Value};

fn graph(nodes: Vec<Value>) -> HostGraphSnapshot {
    HostGraphSnapshot {
        nodes,
        edges: vec![json!({"id":"edge"})],
    }
}

fn video(id: &str, kind: &str, source: Option<&str>) -> Value {
    json!({
        "id": id,
        "data": {
            "type": "input",
            "inputMode": "video",
            "videoSourceType": kind,
            "videoFilePath": source,
            "videoDeviceId": if kind == "camera" { source } else { None },
            "videoLoop": false,
            "videoPlaybackRate": 2.0
        }
    })
}

#[test]
fn native_plans_all_resource_replacements_stable_resources_and_strips_payloads() {
    let previous = graph(vec![
        video("video", "file", Some("/old.mp4")),
        json!({"id":"image","data":{"type":"input","inputDataType":"sampler2D","inputMode":"image","imageDataUrl":"data:old"}}),
        json!({"id":"model","data":{"type":"onnx","onnxModelId":"custom","onnxCustomPath":"/old.onnx"}}),
        json!({"id":"stable-image","data":{"type":"input","inputDataType":"sampler2D","inputMode":"image","rawDataUrl":"raw","fbFormat":"rgba8","fbWidth":1,"fbHeight":1}}),
        video("stable-video", "file", Some("/stable.mp4")),
        json!({"id":"stable-model","data":{"type":"onnx","onnxCatalogId":"yolov8n"}}),
    ]);
    let current = graph(vec![
        video("video", "camera", Some("camera-1")),
        json!({"id":"image","data":{"type":"input","inputDataType":"sampler2D","inputMode":"image","imageDataUrl":"data:new"}}),
        json!({"id":"model","data":{"type":"onnx","onnxModelId":"custom","onnxCustomPath":"/new.onnx","onnxTargetSize":2,"onnxScoreThreshold":0.3,"onnxIouThreshold":0.4}}),
        json!({"id":"stable-image","data":{"type":"input","inputDataType":"sampler2D","inputMode":"image","rawDataUrl":"raw","fbFormat":"rgba8","fbWidth":1,"fbHeight":1}}),
        video("stable-video", "file", Some("/stable.mp4")),
        json!({"id":"stable-model","data":{"type":"onnx","onnxCatalogId":"yolov8n"}}),
        json!({"id":"empty-image","data":{"type":"input","inputDataType":"sampler2D","inputMode":"image"}}),
        json!({"id":"empty-model","data":{"type":"onnx"}}),
        json!({"data":{"type":"onnx","onnxModelId":"ignored"}}),
        json!({"id":"no-data"}),
    ]);
    let plan = plan_host_resource_intents(HostResourceIntentRequest {
        previous_graph: Some(previous),
        graph: current,
    })
    .unwrap();
    assert!(plan.intents.iter().any(
        |intent| matches!(intent, HostResourceIntent::DetachVideo { node_id } if node_id == "video")
    ));
    assert!(plan.intents.iter().any(
        |intent| matches!(intent, HostResourceIntent::RemoveImage { node_id } if node_id == "image")
    ));
    assert!(plan.intents.iter().any(
        |intent| matches!(intent, HostResourceIntent::UnloadOnnx { node_id } if node_id == "model")
    ));
    assert!(plan.intents.iter().any(|intent| matches!(
        intent,
        HostResourceIntent::AttachVideo {
            kind: VideoSourceKind::Camera,
            ..
        }
    )));
    assert!(plan.intents.iter().any(|intent| matches!(intent, HostResourceIntent::UploadImage { node_id, source: ImageSourceIntent::Encoded { .. }, .. } if node_id == "image")));
    assert!(plan.intents.iter().any(|intent| matches!(intent, HostResourceIntent::LoadOnnx { node_id, model_path: Some(_), download: None, .. } if node_id == "model")));
    assert!(!plan.intents.iter().any(|intent| matches!(intent, HostResourceIntent::UploadImage { node_id, .. } if node_id == "stable-image")));
    assert!(!plan.intents.iter().any(|intent| matches!(intent, HostResourceIntent::LoadOnnx { node_id, .. } if node_id == "stable-model")));
    assert!(!plan.intents.iter().any(|intent| matches!(intent, HostResourceIntent::AttachVideo { node_id, .. } if node_id == "stable-video")));
    let encoded_graph = serde_json::to_string(&plan.graph).unwrap();
    assert!(!encoded_graph.contains("data:new"));
    assert!(!encoded_graph.contains("/new.onnx"));
}

#[test]
fn helper_extractors_cover_none_wrong_types_overflow_and_payload_without_data() {
    let malformed = graph(vec![
        json!({}),
        json!({"id":"missing-data"}),
        json!({"id":"wrong","data":{"type":3,"inputMode":"video"}}),
        json!({"id":"overflow","data":{"type":"input","inputDataType":"sampler2D","inputMode":"image","rawDataUrl":"raw","fbWidth":u64::MAX,"fbHeight":"bad"}}),
        video("camera", "camera", None),
        video("file", "file", None),
        json!({"id":"default-video","data":{"type":"input","inputMode":"video","videoSourceType":false,"videoFilePath":"/default.mp4","videoLoop":"bad","videoPlaybackRate":false}}),
        json!({"id":"default-camera","data":{"type":"input","inputMode":"video","videoSourceType":"camera","videoDeviceId":"camera-default"}}),
        json!({"id":"model","data":{"type":"onnx","onnxModelId":"custom","onnxParams":{"targetSize":"bad","scoreThreshold":"bad"}}}),
    ]);
    let plan = plan_host_resource_intents(HostResourceIntentRequest {
        previous_graph: None,
        graph: malformed,
    })
    .unwrap();
    assert!(plan.intents.iter().any(|intent| matches!(intent, HostResourceIntent::UploadImage { node_id, .. } if node_id == "overflow")));
    assert!(plan.intents.iter().any(|intent| matches!(intent, HostResourceIntent::LoadOnnx { node_id, .. } if node_id == "model")));
    assert!(plan.intents.iter().any(|intent| matches!(
        intent,
        HostResourceIntent::AttachVideo {
            node_id,
            key,
            kind: VideoSourceKind::File,
            source,
            looping: true,
            playback_rate,
        } if node_id == "default-video" && key == "file|/default.mp4|true|1"
            && source == "/default.mp4" && *playback_rate == 1.0
    )));
    assert!(plan.intents.iter().any(|intent| matches!(
        intent,
        HostResourceIntent::AttachVideo {
            node_id,
            key,
            kind: VideoSourceKind::Camera,
            source,
            looping: true,
            playback_rate,
        } if node_id == "default-camera" && key == "camera|camera-default|true|1"
            && source == "camera-default" && *playback_rate == 1.0
    )));
    assert!(!plan.intents.iter().any(|intent| matches!(
        intent,
        HostResourceIntent::AttachVideo { node_id, .. }
            if node_id == "camera" || node_id == "file"
    )));
    assert!(plan.intents.iter().any(|intent| matches!(
        intent,
        HostResourceIntent::UploadImage {
            node_id,
            key,
            source: ImageSourceIntent::Raw { format: None, width: None, height: None, .. },
        } if node_id == "overflow" && key == "raw|||"
    )));
    assert!(plan.intents.iter().any(|intent| matches!(
        intent,
        HostResourceIntent::LoadOnnx {
            node_id,
            task: OnnxTask::Generic,
            target_size: 640,
            score_threshold,
            iou_threshold,
            model_path: None,
            download: None,
            ..
        } if node_id == "model" && *score_threshold == 0.25 && *iou_threshold == 0.45
    )));
    assert_eq!(plan.graph.nodes[0], json!({}));
}
