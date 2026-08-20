use open_quartz_execution::catalog::{catalog_snapshot, evaluate_math};
use open_quartz_schema::OnnxTask;
use open_quartz_schema::{DataType, PortDirection};
use serde_json::json;
use std::collections::HashSet;

#[test]
fn math_catalog_freezes_descriptors_categories_and_formulas() {
    let snapshot = catalog_snapshot();
    assert_eq!(
        serde_json::to_value(&snapshot.math_categories).unwrap(),
        json!([
            {"category": "Arithmetic", "ops": ["add", "subtract", "multiply", "divide", "negate", "modulo"]},
            {"category": "Range", "ops": ["min", "max", "clamp", "saturate", "step", "smoothstep", "abs", "sign"]},
            {"category": "Trigonometry", "ops": ["sin", "cos", "tan", "asin", "acos", "atan"]},
            {"category": "Exponential", "ops": ["pow", "sqrt", "exp", "log"]},
            {"category": "Interpolation", "ops": ["mix"]},
            {"category": "Rounding", "ops": ["floor", "ceil", "round", "fract"]}
        ])
    );

    let formulas = snapshot
        .math_ops
        .iter()
        .map(|op| (op.id.as_str(), op.input_count, op.formula.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(formulas.len(), 29);
    assert!(formulas.contains(&("divide", 2, "b == 0 ? 0 : a / b")));
    assert!(formulas.contains(&("smoothstep", 3, "t*t*(3-2*t), t=clamp((c-a)/(b-a),0,1)")));
    assert!(formulas.contains(&("round", 1, "a.fract() == -0.5 ? ceil(a) : floor(a + 0.5)")));

    let categorized = snapshot
        .math_categories
        .iter()
        .flat_map(|category| category.ops.iter().cloned())
        .collect::<Vec<_>>();
    let described = snapshot
        .math_ops
        .iter()
        .map(|op| op.id.clone())
        .collect::<HashSet<_>>();
    assert_eq!(categorized.len(), described.len());
    assert!(categorized.iter().all(|op| described.contains(op)));
}

#[test]
fn math_catalog_evaluator_owns_boundary_behavior() {
    let cases = [
        ("divide", vec![1.0, 0.0], 0.0),
        ("modulo", vec![1.0, 0.0], 0.0),
        ("clamp", vec![5.0, 0.0, 1.0], 1.0),
        ("smoothstep", vec![0.0, 1.0, 0.5], 0.5),
        ("mix", vec![0.0, 10.0, 0.3], 3.0),
        ("round", vec![-1.5], -1.0),
        ("fract", vec![-0.3], 0.7),
    ];
    for (operation, input, expected) in cases {
        let actual = evaluate_math(operation, &input);
        assert!(
            (actual - expected).abs() < 1e-9,
            "{operation}: expected {expected}, got {actual}"
        );
    }
}

#[test]
fn onnx_catalog_freezes_task_io_defaults_and_integrity_fields() {
    let snapshot = catalog_snapshot();
    assert_eq!(
        snapshot.onnx_categories,
        vec![
            "Background Removal",
            "Depth Estimation",
            "Detection",
            "Segmentation",
            "Super-Resolution"
        ]
    );
    assert_eq!(snapshot.onnx_models.len(), 7);
    for entry in &snapshot.onnx_models {
        assert_eq!(entry.expected_io.inputs.len(), 1);
        assert!(entry.download_url.starts_with("https://"));
        assert!(entry.file_size > 0);
        assert_eq!(entry.expected_io.inputs[0].direction, PortDirection::Input);
        assert!(entry
            .expected_io
            .outputs
            .iter()
            .all(|port| port.direction == PortDirection::Output));
    }

    let yolo = snapshot
        .onnx_models
        .iter()
        .find(|entry| entry.id == "yolov8n")
        .unwrap();
    assert_eq!(yolo.task, OnnxTask::Detection);
    assert_eq!(yolo.file_size, 12_851_098);
    assert_eq!(yolo.sha256, "");
    assert_eq!(yolo.expected_io.outputs.len(), 2);
    assert_eq!(yolo.expected_io.outputs[0].data_type, DataType::Roi);
    assert_eq!(
        yolo.default_params.as_ref().unwrap()["scoreThreshold"].default,
        json!(0.25)
    );
    assert_eq!(
        yolo.default_params.as_ref().unwrap()["iouThreshold"].default,
        json!(0.45)
    );

    let sr = snapshot
        .onnx_models
        .iter()
        .find(|entry| entry.id == "super-resolution-3x")
        .unwrap();
    assert_eq!(sr.task, OnnxTask::SuperResolution);
    assert!(sr.default_params.is_none());
    assert_eq!(sr.expected_io.outputs[0].label, "upscaled");
}

#[test]
fn shader_catalog_freezes_template_groups_and_port_contracts() {
    let snapshot = catalog_snapshot();
    let groups = snapshot
        .shader_groups
        .iter()
        .map(|group| (group.category.as_str(), group.items.len()))
        .collect::<Vec<_>>();
    assert_eq!(
        groups,
        vec![
            ("FILTER", 7),
            ("COLOR", 7),
            ("GENERATOR", 5),
            ("BLEND", 7),
            ("DISTORTION", 5),
            ("FEEDBACK", 1)
        ]
    );

    let templates = snapshot
        .shader_groups
        .iter()
        .flat_map(|group| group.items.iter())
        .collect::<Vec<_>>();
    assert_eq!(templates.len(), 32);
    assert_eq!(
        templates
            .iter()
            .map(|template| template.label.as_str())
            .collect::<HashSet<_>>()
            .len(),
        templates.len()
    );

    let sobel = templates
        .iter()
        .find(|template| template.label == "Sobel Edge Detection")
        .unwrap();
    assert_eq!(
        sobel
            .inputs
            .iter()
            .map(|port| (port.label.as_str(), port.data_type))
            .collect::<Vec<_>>(),
        vec![("inputImage", DataType::Sampler2d), ("intensity", DataType::Float)]
    );
    assert_eq!(sobel.outputs[0].data_type, DataType::Vec4);

    let feedback = templates
        .iter()
        .find(|template| template.label == "Gray-Scott Reaction-Diffusion")
        .unwrap();
    assert_eq!(feedback.inputs.len(), 5);
    assert_eq!(feedback.outputs[0].label, "fragColor");
}
