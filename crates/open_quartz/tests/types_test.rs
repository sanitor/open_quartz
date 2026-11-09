use open_quartz::{DataType, ProjectFile, PROJECT_FILE_VERSION};
use serde_json::json;

#[test]
fn data_type_uses_typescript_wire_names() {
    assert_eq!(
        serde_json::to_value(DataType::Sampler2d).unwrap(),
        json!("sampler2D")
    );
    assert_eq!(
        serde_json::to_value(DataType::SamplerCube).unwrap(),
        json!("samplerCube")
    );
    assert_eq!(
        serde_json::from_value::<DataType>(json!("rgba8"))
            .unwrap_err()
            .classify(),
        serde_json::error::Category::Data
    );
}

#[test]
fn project_file_round_trips_the_typescript_contract() {
    let fixture = json!({
        "version": PROJECT_FILE_VERSION,
        "name": "SDK fixture",
        "createdAt": "2026-07-29T00:00:00.000Z",
        "updatedAt": "2026-07-29T00:00:01.000Z",
        "graph": {
            "nodes": [{
                "id": "shader_1",
                "type": "shader",
                "position": { "x": 12.5, "y": 24.0 },
                "data": {
                    "type": "shader",
                    "label": "Shader",
                    "shaderCode": "@fragment fn main() -> @location(0) vec4f { return vec4f(1.0); }",
                    "inputs": [{
                        "id": "input_1",
                        "label": "source",
                        "dataType": "sampler2D",
                        "direction": "input",
                        "description": "Source texture"
                    }],
                    "outputs": [],
                    "uniforms": { "gain": 0.5 },
                    "onnxParams": { "enabled": true, "threshold": 0.25 },
                    "collapsed": true,
                    "expanded": false,
                    "resolvedWidth": 1920,
                    "resolvedHeight": 1080,
                    "onnxStatus": "ready",
                    "onnxProgress": 1,
                    "onnxBackend": "wasm",
                    "onnxCustomFileName": "model.onnx",
                    "imageFileName": "image.png",
                    "rawFileName": "frame.bin",
                    "videoFileName": "clip.mp4",
                    "pluginMetadata": { "owner": "test" }
                }
            }],
            "edges": [{
                "id": "edge_1",
                "source": "input_1",
                "sourceHandle": "out",
                "target": "shader_1",
                "targetHandle": "source"
            }]
        }
    });

    let project: ProjectFile = serde_json::from_value(fixture.clone()).unwrap();
    let encoded = serde_json::to_value(project).unwrap();
    let mut expected = fixture;
    let data = expected["graph"]["nodes"][0]["data"].as_object_mut().unwrap();
    for suffix in ["Params", "Status", "Progress", "Backend", "CustomFileName"] {
        let value = data.remove(&format!("onnx{suffix}")).unwrap();
        data.insert(format!("inference{suffix}"), value);
    }

    assert_eq!(encoded, expected);
}

#[test]
fn legacy_inference_project_and_graph_preserve_identity_and_emit_canonical_data() {
    let legacy_graph = json!({
        "nodes": [{
            "id": "onnx_7", "type": "onnx", "position": {"x": 4.5, "y": -2.0},
            "data": {
                "type": "onnx", "label": "Legacy detector", "shaderCode": "",
                "inputs": [{"id": "onnx_in_image", "label": "image", "dataType": "sampler2D", "direction": "input"}],
                "outputs": [{"id": "onnx_out_overlay", "label": "overlay", "dataType": "sampler2D", "direction": "output"}],
                "uniforms": {}, "onnxModelId": "yolov8n", "onnxCatalogId": "yolov8n",
                "onnxSource": "custom", "onnxTask": "detection", "onnxCustomPath": "/models/detector.onnx",
                "onnxScoreThreshold": 0.25, "onnxIouThreshold": 0.45, "onnxTargetSize": 640,
                "onnxParams": {"enabled": true, "threshold": 0.25},
                "onnxStatus": "ready", "onnxProgress": 1.0, "onnxBackend": "cpu",
                "onnxCustomFileName": "detector.onnx", "onnxFutureExtension": {"opaque": [1, 2]},
                "pluginMetadata": {"onnxLiteral": "keep me"}, "customExtension": [true, "opaque"]
            }
        }, {
            "id": "renderer_2", "type": "renderer", "position": {"x": 10.0, "y": 0.0},
            "data": {"type": "renderer", "label": "Renderer", "shaderCode": "",
                "inputs": [{"id": "image_in", "label": "image", "dataType": "sampler2D", "direction": "input"}],
                "outputs": [], "uniforms": {}}
        }],
        "edges": [{"id": "onnx_edge_9", "source": "onnx_7", "sourceHandle": "onnx_out_overlay",
            "target": "renderer_2", "targetHandle": "image_in"}]
    });
    let legacy_file = json!({
        "version": "0.4.0", "name": "Legacy inference",
        "createdAt": "2026-01-01T00:00:00Z", "updatedAt": "2026-01-02T00:00:00Z",
        "graph": legacy_graph
    });
    let file: ProjectFile = serde_json::from_value(legacy_file.clone()).unwrap();
    let graph: open_quartz::Graph = serde_json::from_value(legacy_file["graph"].clone()).unwrap();
    graph.validate().unwrap();
    assert_eq!(graph.nodes[0].node_type, open_quartz::NodeType::Inference);
    assert_eq!(graph.nodes[0].data.inference_task, Some(open_quartz::InferenceTask::Detection));
    assert_eq!(graph.nodes[0].data.inference_model_path.as_deref(), Some("/models/detector.onnx"));
    assert_eq!(graph.nodes[0].data.extra["inferenceFutureExtension"], json!({"opaque": [1, 2]}));

    let encoded = serde_json::to_value(&file).unwrap();
    assert_eq!(PROJECT_FILE_VERSION, "0.4.0");
    assert_eq!(encoded["version"], legacy_file["version"]);
    assert_eq!(encoded["createdAt"], legacy_file["createdAt"]);
    assert_eq!(encoded["updatedAt"], legacy_file["updatedAt"]);
    assert_eq!(encoded["graph"]["edges"], legacy_file["graph"]["edges"]);
    for (old, new) in legacy_file["graph"]["nodes"].as_array().unwrap().iter()
        .zip(encoded["graph"]["nodes"].as_array().unwrap()) {
        assert_eq!(new["id"], old["id"]);
        assert_eq!(new["position"], old["position"]);
        assert_eq!(new["data"]["inputs"], old["data"]["inputs"]);
        assert_eq!(new["data"]["outputs"], old["data"]["outputs"]);
    }
    let mut expected_graph = legacy_file["graph"].clone();
    expected_graph["nodes"][0]["type"] = json!("inference");
    let data = expected_graph["nodes"][0]["data"].as_object_mut().unwrap();
    data.insert("type".to_owned(), json!("inference"));
    let old_keys: Vec<_> = data.keys().filter(|key| key.starts_with("onnx")).cloned().collect();
    for key in old_keys {
        let value = data.remove(&key).unwrap();
        let canonical = if key == "onnxCustomPath" {
            "inferenceModelPath".to_owned()
        } else {
            format!("inference{}", &key[4..])
        };
        data.insert(canonical, value);
    }
    assert_eq!(encoded["graph"], expected_graph);
    assert_eq!(serde_json::to_value(&graph).unwrap(), expected_graph);
    assert_eq!(serde_json::from_value::<ProjectFile>(encoded.clone()).unwrap(), file);
    assert_eq!(serde_json::to_value(serde_json::from_value::<ProjectFile>(encoded.clone()).unwrap()).unwrap(), encoded);
    assert_eq!(serde_json::from_value::<open_quartz::Graph>(expected_graph).unwrap(), graph);
}

#[test]
fn model_path_aliases_and_flattened_extensions_write_canonical_names() {
    for alias in ["onnxCustomPath", "onnxModelPath", "inferenceCustomPath", "customPath", "inferenceModelPath"] {
        let mut input = serde_json::to_value(open_quartz::NodeData::default()).unwrap();
        input[alias] = json!("/models/custom.onnx");
        let data: open_quartz::NodeData = serde_json::from_value(input).unwrap();
        assert_eq!(data.inference_model_path.as_deref(), Some("/models/custom.onnx"));
        let encoded = serde_json::to_value(data).unwrap();
        assert_eq!(encoded["inferenceModelPath"], json!("/models/custom.onnx"));
        for legacy in ["onnxCustomPath", "onnxModelPath", "inferenceCustomPath", "customPath"] {
            assert!(encoded.get(legacy).is_none());
        }
    }
    let mut data = open_quartz::NodeData::default();
    data.extra.insert("onnxFutureKey".to_owned(), json!({"nested": "unchanged"}));
    data.extra.insert("pluginKey".to_owned(), json!([1, 2, 3]));
    let encoded = serde_json::to_value(data).unwrap();
    assert_eq!(encoded["inferenceFutureKey"], json!({"nested": "unchanged"}));
    assert_eq!(encoded["pluginKey"], json!([1, 2, 3]));
    assert!(encoded.get("onnxFutureKey").is_none());
}

#[test]
fn legacy_custom_model_params_survive_typed_save_and_resource_planning() {
    use open_quartz::resources::plan::{plan_host_resource_intents, HostResourceIntentRequest};
    let legacy = json!({"nodes": [{
        "id": "onnx_custom_8", "type": "onnx", "position": {"x": 1, "y": 2},
        "data": {"type": "onnx", "label": "Custom", "shaderCode": "", "inputs": [], "outputs": [],
            "uniforms": {}, "customPath": "/models/custom.onnx", "onnxSource": "custom",
            "params": {"targetSize": 320, "scoreThreshold": 0.4, "iouThreshold": 0.6, "preferDirectMl": true},
            "pluginMetadata": {"owner": "keep"}}
    }], "edges": []});
    let graph: open_quartz::Graph = serde_json::from_value(legacy.clone()).unwrap();
    let data = &graph.nodes[0].data;
    assert_eq!(data.inference_model_path.as_deref(), Some("/models/custom.onnx"));
    let params = data.inference_params.as_ref().unwrap();
    assert_eq!(params["targetSize"], open_quartz::InferenceParamValue::Number(320.0));
    assert_eq!(params["scoreThreshold"], open_quartz::InferenceParamValue::Number(0.4));
    assert_eq!(params["iouThreshold"], open_quartz::InferenceParamValue::Number(0.6));
    assert_eq!(params["preferDirectMl"], open_quartz::InferenceParamValue::Boolean(true));
    let saved = serde_json::to_value(&graph).unwrap();
    assert_eq!(saved["nodes"][0]["id"], legacy["nodes"][0]["id"]);
    assert_eq!(saved["edges"], legacy["edges"]);
    assert_eq!(saved["nodes"][0]["data"]["inferenceModelPath"], legacy["nodes"][0]["data"]["customPath"]);
    assert_eq!(saved["nodes"][0]["data"]["inferenceParams"]["preferDirectMl"], true);
    assert!(saved["nodes"][0]["data"].get("params").is_none());
    assert!(saved["nodes"][0]["data"].get("customPath").is_none());
    assert_eq!(saved["nodes"][0]["data"]["pluginMetadata"], legacy["nodes"][0]["data"]["pluginMetadata"]);
    let plan = |graph| {
        let request: HostResourceIntentRequest = serde_json::from_value(json!({"graph": graph})).unwrap();
        serde_json::to_value(plan_host_resource_intents(request).unwrap()).unwrap()
    };
    assert_eq!(plan(legacy)["intents"], plan(saved)["intents"]);
}

#[test]
fn mixed_inference_aliases_merge_params_with_canonical_values_winning() {
    let mut input = serde_json::to_value(open_quartz::NodeData::default()).unwrap();
    input["type"] = json!("onnx");
    input["customPath"] = json!("bare.onnx");
    input["onnxCustomPath"] = json!("legacy.onnx");
    input["onnxModelPath"] = json!("legacy-model.onnx");
    input["inferenceCustomPath"] = json!("custom.onnx");
    input["inferenceModelPath"] = json!("canonical.onnx");
    input["params"] = json!({"targetSize": 320, "scoreThreshold": 0.1, "iouThreshold": 0.5, "preferDirectMl": true});
    input["onnxParams"] = json!({"targetSize": 640, "scoreThreshold": 0.2});
    input["inferenceParams"] = json!({"scoreThreshold": 0.8});
    input["onnxModelId"] = json!("old-id");
    input["inferenceModelId"] = json!("canonical-id");
    let data: open_quartz::NodeData = serde_json::from_value(input.clone()).unwrap();
    assert_eq!(data.inference_model_path.as_deref(), Some("canonical.onnx"));
    assert_eq!(data.inference_model_id.as_deref(), Some("canonical-id"));
    let encoded = serde_json::to_value(data).unwrap();
    assert_eq!(encoded["inferenceParams"], json!({"targetSize": 640.0, "scoreThreshold": 0.8, "iouThreshold": 0.5, "preferDirectMl": true}));
    for key in ["params", "onnxParams", "customPath", "onnxCustomPath", "onnxModelPath", "inferenceCustomPath", "onnxModelId"] {
        assert!(encoded.get(key).is_none(), "{key}");
    }
    input["inferenceParams"] = serde_json::Value::Null;
    input["inferenceModelPath"] = serde_json::Value::Null;
    let data: open_quartz::NodeData = serde_json::from_value(input.clone()).unwrap();
    assert!(data.inference_params.is_none());
    assert!(data.inference_model_path.is_none());
    input["inferenceParams"] = json!("invalid canonical params");
    assert!(serde_json::from_value::<open_quartz::NodeData>(input).is_err());
}

#[test]
fn non_inference_bare_params_remain_unrelated_extra_data() {
    let mut input = serde_json::to_value(open_quartz::NodeData::default()).unwrap();
    input["params"] = json!({"plugin": ["opaque", 7], "preferDirectMl": "unrelated"});
    let data: open_quartz::NodeData = serde_json::from_value(input.clone()).unwrap();
    assert!(data.inference_params.is_none());
    assert_eq!(data.extra["params"], input["params"]);
    assert_eq!(serde_json::to_value(data).unwrap(), input);
}

#[test]
fn typed_save_keeps_top_level_settings_above_bare_legacy_params() {
    use open_quartz::resources::plan::{plan_host_resource_intents, HostResourceIntentRequest};
    for prefix in ["inference", "onnx"] {
        let mut raw = json!({"nodes": [{
            "id": "legacy-custom", "type": "onnx", "position": {"x": 0, "y": 0},
            "data": {"type": "onnx", "label": "Custom", "shaderCode": "", "inputs": [], "outputs": [],
                "uniforms": {}, "customPath": "custom.onnx", "onnxSource": "custom", "onnxModelId": "custom",
                "params": {"scoreThreshold": 0.1, "targetSize": 320, "iouThreshold": 0.2, "preferDirectMl": true}}
        }], "edges": []});
        let data = raw["nodes"][0]["data"].as_object_mut().unwrap();
        data.insert(format!("{prefix}ScoreThreshold"), json!(0.9));
        data.insert(format!("{prefix}TargetSize"), json!(640));
        data.insert(format!("{prefix}IouThreshold"), json!(0.7));
        data.insert(format!("{prefix}PreferDirectMl"), json!(false));
        let typed: open_quartz::Graph = serde_json::from_value(raw.clone()).unwrap();
        let saved = serde_json::to_value(&typed).unwrap();
        assert_eq!(typed.nodes[0].data.inference_score_threshold, Some(0.9));
        assert_eq!(saved["nodes"][0]["data"]["inferenceParams"], json!({}));
        let plan = |graph| {
            let request: HostResourceIntentRequest = serde_json::from_value(json!({"graph": graph})).unwrap();
            serde_json::to_value(plan_host_resource_intents(request).unwrap()).unwrap()
        };
        let before = plan(raw);
        let after = plan(saved);
        assert_eq!(before["intents"], after["intents"]);
        assert_eq!(after["intents"][0]["scoreThreshold"], json!(0.9));
        assert_eq!(after["intents"][0]["targetSize"], json!(640));
        assert_eq!(after["intents"][0]["iouThreshold"], json!(0.7));
    }
}
