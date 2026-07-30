use open_quartz::types::{DataType, ProjectFile, PROJECT_FILE_VERSION};
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

    assert_eq!(encoded, fixture);
    assert!(encoded["graph"]["nodes"][0]["data"]
        .get("collapsed")
        .is_none());
}
