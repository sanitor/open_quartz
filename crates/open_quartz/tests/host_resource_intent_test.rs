use open_quartz::resources::plan::{plan_host_resource_intents, HostResourceIntentRequest};
use serde_json::{json, Value};

fn plan(request: Value) -> Value {
    let request: HostResourceIntentRequest = serde_json::from_value(request).unwrap();
    serde_json::to_value(plan_host_resource_intents(request).unwrap()).unwrap()
}

fn input_node(id: &str, data: Value) -> Value {
    json!({
        "id": id,
        "type": "input",
        "position": { "x": 0, "y": 0 },
        "data": data,
    })
}

#[test]
fn native_video_replacement_and_replay_intents_are_keyed_by_source_and_playback() {
    let first = plan(json!({
        "graph": {
            "nodes": [input_node("video", json!({
                "type": "input",
                "inputMode": "video",
                "inputDataType": "sampler2D",
                "videoSourceType": "file",
                "videoFilePath": "C:/video/source-hevc.mp4",
                "videoUrl": "asset://video/source-hevc.mp4"
            }))],
            "edges": []
        }
    }));
    assert_eq!(first["intents"][0]["type"], "attach-video");
    assert!(first["graph"]["nodes"][0]["data"]
        .get("videoFilePath")
        .is_none());
    assert!(first["graph"]["nodes"][0]["data"].get("videoUrl").is_none());

    let same = plan(json!({
        "previousGraph": {
            "nodes": [input_node("video", json!({
                "type": "input",
                "inputMode": "video",
                "inputDataType": "sampler2D",
                "videoSourceType": "file",
                "videoFilePath": "C:/video/source-h264.mp4"
            }))],
            "edges": []
        },
        "graph": {
            "nodes": [input_node("video", json!({
                "type": "input",
                "inputMode": "video",
                "inputDataType": "sampler2D",
                "videoSourceType": "file",
                "videoFilePath": "C:/video/source-h264.mp4"
            }))],
            "edges": []
        }
    }));
    assert_eq!(same["intents"], json!([]));

    let replace = plan(json!({
        "previousGraph": {
            "nodes": [input_node("video", json!({
                "type": "input",
                "inputMode": "video",
                "inputDataType": "sampler2D",
                "videoSourceType": "file",
                "videoFilePath": "C:/video/source-hevc.mp4"
            }))],
            "edges": []
        },
        "graph": {
            "nodes": [input_node("video", json!({
                "type": "input",
                "inputMode": "video",
                "inputDataType": "sampler2D",
                "videoSourceType": "file",
                "videoFilePath": "C:/video/source-h264.mp4"
            }))],
            "edges": []
        }
    }));
    assert_eq!(
        replace["intents"]
            .as_array()
            .unwrap()
            .iter()
            .map(|intent| intent["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["detach-video", "attach-video"]
    );
}

#[test]
fn native_detaches_stale_video_before_uploading_replacement_image() {
    let result = plan(json!({
        "previousGraph": {
            "nodes": [input_node("resource", json!({
                "type": "input",
                "inputMode": "video",
                "inputDataType": "sampler2D",
                "videoSourceType": "file",
                "videoFilePath": "C:/video/input.mp4"
            }))],
            "edges": []
        },
        "graph": {
            "nodes": [input_node("resource", json!({
                "type": "input",
                "inputMode": "image",
                "inputDataType": "sampler2D",
                "rawDataUrl": "raw://image",
                "fbFormat": "rgba8",
                "fbWidth": 1,
                "fbHeight": 1
            }))],
            "edges": []
        }
    }));
    assert_eq!(
        result["intents"]
            .as_array()
            .unwrap()
            .iter()
            .map(|intent| intent["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["detach-video", "upload-image"]
    );
}

#[test]
fn native_onnx_intent_uses_rust_catalog_task_defaults_and_integrity() {
    let result = plan(json!({
        "graph": {
            "nodes": [json!({
                "id": "onnx",
                "type": "onnx",
                "position": { "x": 0, "y": 0 },
                "data": {
                    "type": "onnx",
                    "onnxCatalogId": "yolov8n",
                    "onnxModelId": "yolov8n"
                }
            })],
            "edges": []
        }
    }));
    assert_eq!(result["intents"][0]["type"], "load-inference");
    assert_eq!(result["intents"][0]["task"], "detection");
    assert_eq!(result["intents"][0]["targetSize"], 640);
    assert_eq!(result["intents"][0]["scoreThreshold"], 0.25);
    assert_eq!(result["intents"][0]["iouThreshold"], 0.45);
    assert_eq!(result["intents"][0]["download"]["modelId"], "yolov8n");
    assert_eq!(result["intents"][0]["download"]["expectedSize"], 12_851_098);
}

#[test]
fn mixed_legacy_and_canonical_inference_fields_use_canonical_precedence() {
    let result = plan(json!({
        "graph": {"nodes": [{"id":"mixed","type":"onnx","data": {
            "type":"inference", "onnxModelId":"legacy", "inferenceModelId":"canonical",
            "onnxTargetSize":320, "inferenceTargetSize":512,
            "onnxParams":{"scoreThreshold":0.1}, "inferenceParams":{"scoreThreshold":0.2},
            "onnxPolicy":{"backend":"ort-cpu"},
            "inferencePolicy":{"backend":"ort-direct-ml","fallback":"allow-cpu","residency":"cpu-transfers-allowed","placement":"any"}
        }}], "edges": []}
    }));
    let intent = &result["intents"][0];
    assert_eq!(intent["type"], "load-inference");
    assert_eq!(intent["modelId"], "canonical");
    assert_eq!(intent["targetSize"], 512);
    assert_eq!(intent["scoreThreshold"], 0.2);
    assert_eq!(intent["policy"]["backend"], "ort-direct-ml");
    assert_eq!(result["graph"]["nodes"][0]["type"], "inference");
    assert_eq!(result["graph"]["nodes"][0]["data"]["type"], "inference");
    assert_eq!(result["graph"]["nodes"][0]["data"]["inferenceModelId"], "canonical");
    assert!(result["graph"]["nodes"][0]["data"].get("onnxModelId").is_none());
}

#[test]
fn legacy_directml_policy_is_preserved_and_invalid_policy_is_rejected() {
    let first = json!({"id":"model","data":{"type":"onnx","onnxModelId":"custom","onnxParams":{"preferDirectMl":true}}});
    let second = json!({"id":"model","data":{"type":"inference","inferenceModelId":"custom","inferencePolicy":{"backend":"ort-cpu"}}});
    let result = plan(json!({
        "previousGraph":{"nodes":[first],"edges":[]},
        "graph":{"nodes":[second],"edges":[]}
    }));
    assert_eq!(result["intents"][0]["type"], "unload-inference");
    assert_eq!(result["intents"][1]["type"], "load-inference");
    let invalid: HostResourceIntentRequest = serde_json::from_value(json!({
        "graph":{"nodes":[{"id":"bad","data":{"type":"inference","inferenceModelId":"custom","inferencePolicy":{"backend":"mystery"}}}],"edges":[]}
    })).unwrap();
    assert!(plan_host_resource_intents(invalid).is_err());
}
#[test]
fn native_onnx_unloads_stale_model_when_removed() {
    let result = plan(json!({
        "previousGraph": {
            "nodes": [json!({
                "id": "onnx",
                "type": "onnx",
                "position": { "x": 0, "y": 0 },
                "data": {
                    "type": "onnx",
                    "onnxCatalogId": "yolov8n",
                    "onnxModelId": "yolov8n"
                }
            })],
            "edges": []
        },
        "graph": { "nodes": [], "edges": [] }
    }));
    assert_eq!(
        result["intents"],
        json!([{ "type": "unload-inference", "nodeId": "onnx" }])
    );
}
