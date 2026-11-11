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
        |intent| matches!(intent, HostResourceIntent::UnloadInference { node_id } if node_id == "model")
    ));
    assert!(plan.intents.iter().any(|intent| matches!(
        intent,
        HostResourceIntent::AttachVideo {
            kind: VideoSourceKind::Camera,
            ..
        }
    )));
    assert!(plan.intents.iter().any(|intent| matches!(intent, HostResourceIntent::UploadImage { node_id, source: ImageSourceIntent::Encoded { .. }, .. } if node_id == "image")));
    assert!(plan.intents.iter().any(|intent| matches!(intent, HostResourceIntent::LoadInference { node_id, model_path: Some(_), download: None, .. } if node_id == "model")));
    assert!(!plan.intents.iter().any(|intent| matches!(intent, HostResourceIntent::UploadImage { node_id, .. } if node_id == "stable-image")));
    assert!(!plan.intents.iter().any(|intent| matches!(intent, HostResourceIntent::LoadInference { node_id, .. } if node_id == "stable-model")));
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
    assert!(plan.intents.iter().any(|intent| matches!(intent, HostResourceIntent::LoadInference { node_id, .. } if node_id == "model")));
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
        HostResourceIntent::LoadInference {
            node_id,
            task: InferenceTask::Generic,
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

#[test]
fn legacy_custom_path_and_bare_params_preserve_configuration_and_canonical_graph() {
    let legacy = json!({
        "id": "onnx-original-id", "type": "onnx",
        "data": {
            "type": "onnx", "onnxModelId": "custom-model", "customPath": "/models/custom.onnx",
            "inputs": [{"id":"onnx-input"}], "outputs": [{"id":"onnx-output"}],
            "params": {"targetSize":320,"scoreThreshold":0.3,"iouThreshold":0.6,"preferDirectMl":true}
        }
    });
    let source = graph(vec![legacy.clone()]);
    let edges = source.edges.clone();
    let plan = plan_host_resource_intents(HostResourceIntentRequest {
        previous_graph: None,
        graph: source,
    })
    .unwrap();
    assert_eq!(plan.intents.len(), 1);
    assert!(
        matches!(&plan.intents[0], HostResourceIntent::LoadInference {
        model_path: Some(path), target_size: 320, score_threshold, iou_threshold, policy,
        download: None, ..
    } if path == "/models/custom.onnx" && *score_threshold == 0.3 && *iou_threshold == 0.6
        && policy.backend == BackendPreference::OrtDirectMl)
    );
    let saved = &plan.graph.nodes[0];
    assert_eq!(saved["id"], "onnx-original-id");
    assert_eq!(saved["type"], "inference");
    assert_eq!(saved["data"]["type"], "inference");
    assert_eq!(saved["data"]["inferenceModelId"], "custom-model");
    assert_eq!(saved["data"]["inferenceParams"], legacy["data"]["params"]);
    assert_eq!(saved["data"]["inputs"], legacy["data"]["inputs"]);
    assert_eq!(saved["data"]["outputs"], legacy["data"]["outputs"]);
    assert!(saved["data"].get("params").is_none());
    assert!(saved["data"].get("customPath").is_none());
    assert_eq!(plan.graph.edges, edges);
}

#[test]
fn canonical_and_onnx_fields_override_bare_params_before_and_after_save() {
    let source = graph(vec![json!({
        "id":"mixed", "data": {
            "type":"inference", "inferenceModelId":"custom",
            "inferenceParams":{"targetSize":512,"preferDirectMl":false},
            "inferenceScoreThreshold":0.4,
            "onnxParams":{"iouThreshold":0.7},
            "params":{"targetSize":320,"scoreThreshold":0.1,"iouThreshold":0.2,"preferDirectMl":true,"customOption":9}
        }
    })]);
    let first = plan_host_resource_intents(HostResourceIntentRequest {
        previous_graph: None,
        graph: source,
    })
    .unwrap();
    assert!(
        matches!(&first.intents[0], HostResourceIntent::LoadInference {
        target_size: 512, score_threshold, iou_threshold, policy, ..
    } if *score_threshold == 0.4 && *iou_threshold == 0.7
        && *policy == ExecutionPolicy::resident())
    );
    let saved = &first.graph.nodes[0]["data"];
    assert!(saved.get("params").is_none());
    assert_eq!(saved["inferenceParams"]["targetSize"], 512);
    assert_eq!(saved["inferenceParams"]["preferDirectMl"], false);
    assert_eq!(saved["inferenceParams"]["iouThreshold"], 0.7);
    assert_eq!(saved["inferenceParams"]["customOption"], 9);
    let restored = plan_host_resource_intents(HostResourceIntentRequest {
        previous_graph: None,
        graph: first.graph,
    })
    .unwrap();
    assert_eq!(restored.intents, first.intents);
}

#[test]
fn bare_params_on_other_nodes_are_preserved_and_invalid_legacy_preference_errors() {
    let shader = json!({"id":"shader","data":{"type":"shader","params":{"targetSize":5}}});
    let plan = plan_host_resource_intents(HostResourceIntentRequest {
        previous_graph: None,
        graph: graph(vec![shader.clone()]),
    })
    .unwrap();
    assert_eq!(plan.graph.nodes[0], shader);
    assert!(effective_inference_policy(&json!({"params":{"preferDirectMl":"true"}})).is_err());
}

#[test]
fn legacy_custom_path_and_bare_params_keep_same_resource_key_after_typed_roundtrip() {
    let legacy = json!({
        "id":"onnx-kept", "type":"onnx", "data": {
            "type":"onnx", "label":"Legacy model", "shaderCode":"", "uniforms":{},
            "inputs":[], "outputs":[], "onnxModelId":"custom", "customPath":"/custom.onnx",
            "params":{"targetSize":256,"scoreThreshold":0.35,"iouThreshold":0.65,"preferDirectMl":true}
        }
    });
    let typed: crate::project::model::NodeData =
        serde_json::from_value(legacy["data"].clone()).unwrap();
    let mut canonical = legacy.clone();
    canonical["type"] = json!("inference");
    canonical["data"] = serde_json::to_value(typed).unwrap();
    assert!(canonical["data"].get("params").is_none());
    for param in ["targetSize", "scoreThreshold", "iouThreshold"] {
        assert_eq!(
            canonical["data"]["inferenceParams"][param].as_f64(),
            legacy["data"]["params"][param].as_f64(),
        );
    }
    assert_eq!(canonical["data"]["inferenceParams"]["preferDirectMl"], true);
    assert_eq!(canonical["data"]["inferenceModelPath"], "/custom.onnx");
    let unchanged = plan_host_resource_intents(HostResourceIntentRequest {
        previous_graph: Some(graph(vec![legacy])),
        graph: graph(vec![canonical]),
    })
    .unwrap();
    assert!(unchanged.intents.is_empty());
    assert_eq!(
        effective_inference_policy(&json!({
            "inferenceParams":{"targetSize":512}, "params":{"preferDirectMl":true}
        }))
        .unwrap()
        .backend,
        BackendPreference::OrtDirectMl
    );
}

#[test]
fn implicit_inference_policy_is_strict_resident_but_saved_host_intent_stays_readable() {
    assert_eq!(
        effective_inference_policy(&json!({})).unwrap(),
        ExecutionPolicy::resident()
    );
    assert_eq!(
        effective_inference_policy(&json!({"preferDirectMl":false})).unwrap(),
        ExecutionPolicy::resident()
    );
    let saved = json!({"inferencePolicy":ExecutionPolicy::default(),"preferDirectMl":true});
    assert_eq!(
        effective_inference_policy(&saved).unwrap(),
        ExecutionPolicy::default()
    );
}

#[test]
fn canonical_top_level_settings_survive_legacy_parameter_normalization() {
    let node = json!({"id":"model", "type":"inference", "data":{
        "type":"inference", "label":"model", "shaderCode":"", "inputs":[], "outputs":[], "uniforms":{},
        "inferenceModelId":"custom", "inferenceModelPath":null, "onnxCustomPath":"obsolete.onnx",
        "inferenceTargetSize":128, "inferenceIouThreshold":0.6, "inferencePreferDirectMl":false,
        "inferenceParams":{"scoreThreshold":0.8},
        "onnxParams":{"targetSize":256,"iouThreshold":0.7,"preferDirectMl":true},
        "params":{"targetSize":64,"scoreThreshold":0.1,"iouThreshold":0.2,"preferDirectMl":true}
    }});
    let original = plan_host_resource_intents(HostResourceIntentRequest {
        previous_graph: None,
        graph: graph(vec![node.clone()]),
    })
    .unwrap();
    assert!(
        matches!(&original.intents[0], HostResourceIntent::LoadInference {
        target_size:128, score_threshold, iou_threshold, model_path:None, policy, ..
    } if *score_threshold == 0.8 && *iou_threshold == 0.6 && *policy == ExecutionPolicy::resident())
    );
    let typed: crate::project::model::NodeData =
        serde_json::from_str(&node["data"].to_string()).unwrap();
    let mut restored = node;
    restored["data"] = serde_json::to_value(typed).unwrap();
    let restored = plan_host_resource_intents(HostResourceIntentRequest {
        previous_graph: None,
        graph: graph(vec![restored]),
    })
    .unwrap();
    assert_eq!(restored.intents, original.intents);
    let normalized = plan_host_resource_intents(HostResourceIntentRequest {
        previous_graph: None,
        graph: original.graph,
    })
    .unwrap();
    assert_eq!(normalized.intents, original.intents);
}

#[test]
fn invalid_inference_task_rejects_both_new_and_previous_resource_snapshots() {
    let bad = graph(vec![json!({"id":"bad-model","data":{
        "type":"inference","inferenceModelId":"custom","inferenceTask":"unrecognized-task"
    }})]);
    assert!(plan_host_resource_intents(HostResourceIntentRequest {
        previous_graph: None,
        graph: bad.clone(),
    })
    .is_err());
    assert!(plan_host_resource_intents(HostResourceIntentRequest {
        previous_graph: Some(bad),
        graph: graph(vec![]),
    })
    .is_err());
}

#[test]
fn typed_project_data_rejects_nonobjects_and_invalid_inference_params() {
    for source in ["[]", "null", "42"] {
        assert!(serde_json::from_str::<crate::project::model::NodeData>(source).is_err());
    }
    let base = json!({"type":"inference","label":"model","shaderCode":"","inputs":[],"outputs":[],"uniforms":{}});
    for malformed in [json!(false), json!([1]), json!({"targetSize":"large"})] {
        let mut input = base.clone();
        input["params"] = malformed;
        assert!(serde_json::from_value::<crate::project::model::NodeData>(input).is_err());
    }
    let mut shader = base;
    shader["type"] = json!("shader");
    shader["params"] = json!({"targetSize":"shader-specific"});
    let shader: crate::project::model::NodeData = serde_json::from_value(shader).unwrap();
    assert!(shader.inference_params.is_none());
    assert_eq!(
        shader.extra["params"],
        json!({"targetSize":"shader-specific"})
    );
}
