use super::*;
use serde_json::{json, Value};

fn graph(nodes: Vec<Value>) -> HostGraphSnapshot {
    HostGraphSnapshot {
        nodes,
        edges: vec![json!({"id":"edge"})],
    }
}

fn video(id: &str, kind: &str, browser: Option<&str>, native: Option<&str>) -> Value {
    json!({
        "id": id,
        "data": {
            "type": "input",
            "inputMode": "video",
            "videoSourceType": kind,
            "videoUrl": browser,
            "videoFilePath": native,
            "videoDeviceId": if kind == "camera" { native } else { None },
            "videoLoop": false,
            "videoPlaybackRate": 2.0
        }
    })
}

#[test]
fn browser_plans_attach_update_detach_camera_defaults_and_skips_malformed_nodes() {
    let malformed = vec![
        json!({}),
        json!({"id":"missing-data"}),
        json!({"id":"other","data":{"type":"shader"}}),
    ];
    let previous = graph(vec![video("old", "file", Some("blob:old"), None)]);
    let current = graph(
        [
            malformed,
            vec![
                video("same", "file", Some("blob:same"), None),
                video("camera", "camera", None, None),
                video("missing-url", "file", None, None),
            ],
        ]
        .concat(),
    );
    let previous_with_same = graph(vec![
        video("old", "file", Some("blob:old"), None),
        video("same", "file", Some("blob:same"), None),
    ]);
    let plan = plan_host_resource_intents(HostResourceIntentRequest {
        host: HostResourceTarget::Browser,
        previous_graph: Some(previous_with_same),
        graph: current.clone(),
    })
    .unwrap();
    assert!(plan.intents.iter().any(|intent| matches!(intent, HostResourceIntent::UpdateVideo { node_id, .. } if node_id == "same")));
    assert!(plan.intents.iter().any(|intent| matches!(intent, HostResourceIntent::AttachVideo { node_id, kind: VideoSourceKind::Camera, source, .. } if node_id == "camera" && source == "default")));
    assert!(plan.intents.iter().any(
        |intent| matches!(intent, HostResourceIntent::DetachVideo { node_id } if node_id == "old")
    ));
    assert_eq!(plan.graph, current);

    let _ = plan_host_resource_intents(HostResourceIntentRequest {
        host: HostResourceTarget::Browser,
        previous_graph: Some(previous),
        graph: graph(vec![]),
    })
    .unwrap();
}

#[test]
fn native_plans_all_resource_replacements_stable_resources_and_strips_payloads() {
    let previous = graph(vec![
        video("video", "file", None, Some("/old.mp4")),
        json!({"id":"image","data":{"type":"input","inputDataType":"sampler2D","inputMode":"image","imageDataUrl":"data:old"}}),
        json!({"id":"model","data":{"type":"onnx","onnxModelId":"custom","onnxCustomPath":"/old.onnx"}}),
        json!({"id":"stable-image","data":{"type":"input","inputDataType":"sampler2D","inputMode":"image","rawDataUrl":"raw","fbFormat":"rgba8","fbWidth":1,"fbHeight":1}}),
        video("stable-video", "file", None, Some("/stable.mp4")),
        json!({"id":"stable-model","data":{"type":"onnx","onnxCatalogId":"yolov8n"}}),
    ]);
    let current = graph(vec![
        video("video", "camera", None, Some("camera-1")),
        json!({"id":"image","data":{"type":"input","inputDataType":"sampler2D","inputMode":"image","imageDataUrl":"data:new"}}),
        json!({"id":"model","data":{"type":"onnx","onnxModelId":"custom","onnxCustomPath":"/new.onnx","onnxTargetSize":2,"onnxScoreThreshold":0.3,"onnxIouThreshold":0.4}}),
        json!({"id":"stable-image","data":{"type":"input","inputDataType":"sampler2D","inputMode":"image","rawDataUrl":"raw","fbFormat":"rgba8","fbWidth":1,"fbHeight":1}}),
        video("stable-video", "file", None, Some("/stable.mp4")),
        json!({"id":"stable-model","data":{"type":"onnx","onnxCatalogId":"yolov8n"}}),
        json!({"id":"empty-image","data":{"type":"input","inputDataType":"sampler2D","inputMode":"image"}}),
        json!({"id":"empty-model","data":{"type":"onnx"}}),
        json!({"data":{"type":"onnx","onnxModelId":"ignored"}}),
        json!({"id":"no-data"}),
    ]);
    let plan = plan_host_resource_intents(HostResourceIntentRequest {
        host: HostResourceTarget::Native,
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
        video("camera", "camera", None, None),
        video("file", "file", None, None),
        json!({"id":"model","data":{"type":"onnx","onnxModelId":"custom","onnxParams":{"targetSize":"bad","scoreThreshold":"bad"}}}),
    ]);
    let plan = plan_host_resource_intents(HostResourceIntentRequest {
        host: HostResourceTarget::Native,
        previous_graph: None,
        graph: malformed,
    })
    .unwrap();
    assert!(plan.intents.iter().any(|intent| matches!(intent, HostResourceIntent::UploadImage { node_id, .. } if node_id == "overflow")));
    assert!(plan.intents.iter().any(|intent| matches!(intent, HostResourceIntent::LoadOnnx { node_id, .. } if node_id == "model")));
    assert_eq!(plan.graph.nodes[0], json!({}));
}
