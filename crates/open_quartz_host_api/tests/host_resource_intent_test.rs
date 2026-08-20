use open_quartz_host_api::{plan_host_resource_intents, HostResourceIntentRequest};
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
fn browser_video_intents_keep_decoder_for_same_key_and_replace_after_new_attach() {
    let previous = json!({
        "nodes": [input_node("video", json!({
            "type": "input",
            "inputMode": "video",
            "inputDataType": "sampler2D",
            "videoSourceType": "file",
            "videoUrl": "blob:current",
            "videoLoop": true,
            "videoPlaybackRate": 1.0
        }))],
        "edges": []
    });
    let same = plan(json!({
        "host": "browser",
        "previousGraph": previous,
        "graph": {
            "nodes": [input_node("video", json!({
                "type": "input",
                "inputMode": "video",
                "inputDataType": "sampler2D",
                "videoSourceType": "file",
                "videoUrl": "blob:current",
                "videoLoop": false,
                "videoPlaybackRate": 0.5
            }))],
            "edges": []
        }
    }));
    assert_eq!(
        same["intents"],
        json!([{
            "type": "update-video",
            "nodeId": "video",
            "key": "file:blob:current",
            "looping": false,
            "playbackRate": 0.5
        }])
    );

    let replace = plan(json!({
        "host": "browser",
        "previousGraph": {
            "nodes": [input_node("video", json!({
                "type": "input",
                "inputMode": "video",
                "inputDataType": "sampler2D",
                "videoSourceType": "file",
                "videoUrl": "blob:current"
            }))],
            "edges": []
        },
        "graph": {
            "nodes": [input_node("video", json!({
                "type": "input",
                "inputMode": "video",
                "inputDataType": "sampler2D",
                "videoSourceType": "file",
                "videoUrl": "blob:replacement"
            }))],
            "edges": []
        }
    }));
    assert_eq!(replace["intents"][0]["type"], "attach-video");
    assert_eq!(replace["intents"][0]["source"], "blob:replacement");
}

#[test]
fn native_video_replacement_and_replay_intents_are_keyed_by_source_and_playback() {
    let first = plan(json!({
        "host": "native",
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
    assert!(first["graph"]["nodes"][0]["data"].get("videoFilePath").is_none());
    assert!(first["graph"]["nodes"][0]["data"].get("videoUrl").is_none());

    let same = plan(json!({
        "host": "native",
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
        "host": "native",
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
        replace["intents"].as_array().unwrap().iter().map(|intent| intent["type"].as_str().unwrap()).collect::<Vec<_>>(),
        vec!["detach-video", "attach-video"]
    );
}

#[test]
fn native_detaches_stale_video_before_uploading_replacement_image() {
    let result = plan(json!({
        "host": "native",
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
        result["intents"].as_array().unwrap().iter().map(|intent| intent["type"].as_str().unwrap()).collect::<Vec<_>>(),
        vec!["detach-video", "upload-image"]
    );
}

#[test]
fn native_onnx_intent_uses_rust_catalog_task_defaults_and_integrity() {
    let result = plan(json!({
        "host": "native",
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
    assert_eq!(result["intents"][0]["type"], "load-onnx");
    assert_eq!(result["intents"][0]["task"], "detection");
    assert_eq!(result["intents"][0]["targetSize"], 640);
    assert_eq!(result["intents"][0]["scoreThreshold"], 0.25);
    assert_eq!(result["intents"][0]["iouThreshold"], 0.45);
    assert_eq!(result["intents"][0]["download"]["modelId"], "yolov8n");
    assert_eq!(result["intents"][0]["download"]["expectedSize"], 12_851_098);
}

#[test]
fn native_onnx_unloads_stale_model_when_removed() {
    let result = plan(json!({
        "host": "native",
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
    assert_eq!(result["intents"], json!([{ "type": "unload-onnx", "nodeId": "onnx" }]));
}
