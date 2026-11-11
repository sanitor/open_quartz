use super::*;
use std::collections::VecDeque;

struct FakeRunner {
    outputs: VecDeque<Result<crate::inference::TensorOutput, String>>,
}

impl FakeRunner {
    fn one(data: Vec<f32>, shape: Vec<i64>) -> Self {
        Self {
            outputs: VecDeque::from([Ok(crate::inference::TensorOutput { data, shape })]),
        }
    }
}

impl TensorRunner for FakeRunner {
    fn run_f32(
        &mut self,
        _input: Vec<f32>,
        _shape: Vec<i64>,
    ) -> Result<crate::inference::TensorOutput, String> {
        self.outputs
            .pop_front()
            .unwrap_or_else(|| Err("missing fake output".to_owned()))
    }
}

fn rgba() -> Vec<u8> {
    vec![
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
    ]
}

#[test]
fn every_native_task_family_runs_through_tensor_runner() {
    let source = rgba();
    for task in [
        InferenceTask::Generic,
        InferenceTask::StyleTransfer,
        InferenceTask::Denoising,
    ] {
        let mut runner = FakeRunner::one(vec![1.0; 12], vec![1, 3, 2, 2]);
        let output = run_image_task_with_runner(
            &mut runner,
            task,
            "generic",
            &source,
            2,
            2,
            2,
            0.25,
            0.45,
        )
        .unwrap();
        assert_eq!((output.width, output.height, output.rgba.len()), (2, 2, 16));
        assert!(output.data.is_none());
    }

    let mut super_resolution = FakeRunner::one(vec![1.0; 3 * 8 * 8], vec![1, 3, 8, 8]);
    let output = run_image_task_with_runner(
        &mut super_resolution,
        InferenceTask::SuperResolution,
        "realesrgan-x4",
        &source,
        2,
        2,
        2,
        0.25,
        0.45,
    )
    .unwrap();
    assert_eq!((output.width, output.height), (8, 8));

    let mut three_x = FakeRunner::one(vec![0.5; 672 * 672], vec![1, 1, 672, 672]);
    let output = run_image_task_with_runner(
        &mut three_x,
        InferenceTask::SuperResolution,
        "super-resolution-3x",
        &source,
        2,
        2,
        2,
        0.25,
        0.45,
    )
    .unwrap();
    assert_eq!((output.width, output.height), (6, 6));

    for model in ["u2netp", "other"] {
        let size = if model == "u2netp" { 320 } else { 512 };
        let mut background =
            FakeRunner::one(vec![0.5; size * size], vec![1, 1, size as i64, size as i64]);
        let output = run_image_task_with_runner(
            &mut background,
            InferenceTask::BackgroundRemoval,
            model,
            &source,
            2,
            2,
            2,
            0.25,
            0.45,
        )
        .unwrap();
        assert_eq!(output.rgba.len(), 16);
    }

    let mut depth = FakeRunner::one(vec![0.5; 256 * 256], vec![1, 1, 256, 256]);
    assert!(run_image_task_with_runner(
        &mut depth,
        InferenceTask::DepthEstimation,
        "midas",
        &source,
        2,
        2,
        2,
        0.25,
        0.45,
    )
    .is_ok());

    let mut detection = FakeRunner::one(vec![0.0; 84 * 8400], vec![1, 84, 8400]);
    let detected = run_image_task_with_runner(
        &mut detection,
        InferenceTask::Detection,
        "yolo",
        &source,
        2,
        2,
        0,
        0.25,
        0.45,
    )
    .unwrap();
    assert!(detected.data.is_some());

    let mut segmentation = FakeRunner::one(vec![0.0; 19], vec![1, 19, 1, 1]);
    let segmented = run_image_task_with_runner(
        &mut segmentation,
        InferenceTask::Segmentation,
        "segment",
        &source,
        2,
        2,
        0,
        0.25,
        0.45,
    )
    .unwrap();
    assert!(segmented.data.is_some());
}

#[test]
fn detection_overlay_uses_source_pixels_and_preserves_normalized_results() {
    // A 16x8 source is letterboxed into 16x16 with four rows of top padding.
    // Row masks independently specify every painted source pixel (bit x).
    let cases = [
        (
            [8.0, 8.0, 7.5, 3.5],
            [0.265625, 0.28125, 0.734375, 0.71875],
            [0u16, 0, 0x1ff0, 0x1ff0, 0x1830, 0x1ff0, 0x1ff0, 0],
        ),
        (
            [8.0, 8.0, 32.0, 32.0],
            [0.0, 0.0, 1.0, 1.0],
            [0xffff, 0xffff, 0xc003, 0xc003, 0xc003, 0xc003, 0xffff, 0xffff],
        ),
        (
            [16.0, 12.0, 0.0, 0.0],
            [1.0, 1.0, 1.0, 1.0],
            [0, 0, 0, 0, 0, 0, 0x8000, 0xc000],
        ),
    ];
    let source = [7, 11, 19, 255].repeat(16 * 8);
    for (xywh, normalized_bbox, expected_rows) in cases {
        let mut raw = vec![0.0; 84 * 8400];
        for (channel, value) in xywh.into_iter().enumerate() {
            raw[channel * 8400] = value;
        }
        raw[4 * 8400] = 0.9;
        let mut runner = FakeRunner::one(raw, vec![1, 84, 8400]);
        let output = run_image_task_with_runner(
            &mut runner, InferenceTask::Detection, "yolo", &source,
            16, 8, 16, 0.25, 0.45,
        ).unwrap();
        assert_eq!((output.width, output.height), (16, 8));
        let detections = output.data.unwrap();
        assert_eq!(detections.as_array().unwrap().len(), 1);
        assert_eq!(detections[0]["bbox"], json!(normalized_bbox));
        for (y, row) in expected_rows.into_iter().enumerate() {
            for x in 0..16 {
                let expected = if row & (1u16 << x) != 0 {
                    [255, 59, 48, 255]
                } else {
                    [7, 11, 19, 255]
                };
                let index = (y * 16 + x) * 4;
                assert_eq!(&output.rgba[index..index + 4], &expected,
                    "box {xywh:?}, pixel ({x}, {y})");
            }
        }
    }
}

#[test]
fn native_pipeline_helpers_cover_shapes_pixels_resize_and_errors() {
    let rgb = TensorOutput { data: vec![1.0, 0.0, 0.0], shape: vec![1, 3, 1, 1] };
    assert_eq!(image_output_size(&rgb, 3, false).unwrap(), (1, 1));
    let gray = TensorOutput { data: vec![0.5], shape: vec![1, 1, 1, 1] };
    assert!(image_output_size(&gray, 3, false).is_err());
    assert_eq!(image_output_size(&gray, 1, false).unwrap(), (1, 1));
    assert!(image_output_size(&TensorOutput { data: vec![], shape: vec![] }, 3, false).is_err());
    assert!(resize_rgba(&rgba(), 2, 2, 1, 1).is_ok());
    assert!(resize_rgba(&[], 0, 0, 0, 0).is_err());
    assert!(letterbox_preprocess(&[], 1, 1, 1).is_err());
    assert!(midas_preprocess(&[], 1, 1).is_err());
    assert!(apply_alpha_mask(&[], &[1.0], 1, 1).is_err());

    let mut pixels = vec![0; 4 * 4 * 4];
    draw_box(&mut pixels, 4, 4, [-2.0, -2.0, 8.0, 8.0], [1, 2, 3, 4]);
    assert!(pixels.iter().any(|value| *value != 0));
    let mut short = vec![];
    set_pixel(&mut short, 1, 0, 0, [1, 2, 3, 4]);
    assert_eq!(to_byte(-1.0), 0);
    assert_eq!(to_byte(2.0), 255);
    assert_eq!(blend(10, 20), 15);

    let mut error_runner = FakeRunner {
        outputs: VecDeque::from([Err("inference".to_owned())]),
    };
    assert!(run_generic(&mut error_runner, &rgba(), 2, 2).is_err());
    let mut short_depth = FakeRunner::one(vec![], vec![1, 1, 2, 2]);
    assert!(run_depth(&mut short_depth, "midas", &rgba(), 2, 2).is_err());
    assert!(run_image_task_with_runner(
        &mut FakeRunner::one(vec![], vec![]),
        InferenceTask::Generic,
        "generic",
        &[],
        0,
        0,
        1,
        0.25,
        0.45,
    )
    .is_err());
}

#[test]
fn pipeline_task_error_propagation_paths() {
    for (task, model) in [
        (InferenceTask::SuperResolution, "realesrgan-x4plus"),
        (InferenceTask::BackgroundRemoval, "u2netp"),
        (InferenceTask::DepthEstimation, "depth"),
        (InferenceTask::Detection, "yolo"),
        (InferenceTask::Segmentation, "segment"),
    ] {
        assert!(run_image_task_with_runner(
            &mut FakeRunner::one(vec![], vec![]),
            task,
            model,
            &[],
            1,
            1,
            2,
            0.25,
            0.45,
        )
        .is_err());
        let mut runner = FakeRunner {
            outputs: VecDeque::from([Err("inference".to_owned())]),
        };
        assert!(run_image_task_with_runner(
            &mut runner,
            task,
            model,
            &rgba(),
            2,
            2,
            2,
            0.25,
            0.45,
        )
        .is_err());
    }
    let mut bad_super = FakeRunner::one(vec![], vec![1, 3, 2, 2]);
    assert!(run_super_resolution(&mut bad_super, "realesrgan-x4plus", &rgba(), 2, 2).is_err());
    let mut bad_three_x = FakeRunner::one(vec![], vec![1, 1, 1, 1]);
    assert!(run_super_resolution(&mut bad_three_x, "super-resolution-3x", &rgba(), 2, 2).is_err());
    let mut errored_three_x = FakeRunner {
        outputs: VecDeque::from([Err("inference".to_owned())]),
    };
    assert!(
        run_super_resolution(&mut errored_three_x, "super-resolution-3x", &rgba(), 2, 2).is_err()
    );
    let mut invalid_three_x = FakeRunner::one(vec![], vec![]);
    assert!(run_super_resolution(&mut invalid_three_x, "super-resolution-3x", &[], 1, 1).is_err());
    let mut bad_background = FakeRunner::one(vec![], vec![1, 1, 1, 1]);
    assert!(run_background_removal(&mut bad_background, "u2netp", &rgba(), 2, 2).is_err());
    let mut bad_segmentation = FakeRunner::one(vec![], vec![1, 19, 1, 1]);
    assert!(run_segmentation(&mut bad_segmentation, &rgba(), 2, 2, 2).is_err());
    let mut invalid_output = FakeRunner::one(vec![], vec![1, 3, 1, 1]);
    assert!(run_generic(&mut invalid_output, &rgba(), 2, 2).is_err());
}

#[test]
fn detection_pipeline_draws_nonempty_detection_overlay() {
    const BOXES: usize = 8400;
    let mut raw = vec![0.0; 84 * BOXES];
    raw[0] = 1.0;
    raw[BOXES] = 1.0;
    raw[BOXES * 2] = 1.0;
    raw[BOXES * 3] = 1.0;
    raw[BOXES * 4] = 1.0;
    let mut runner = FakeRunner::one(raw, vec![1, 84, BOXES as i64]);
    let output = run_detection(&mut runner, &rgba(), 2, 2, 2, 0.25, 0.45).unwrap();
    assert!(output.data.is_some());
    assert_ne!(output.rgba, rgba());
}

#[test]
fn image_adapter_refuses_multiple_or_misnamed_outputs() {
    let tensor = |name: &str| crate::inference::NamedTensor {
        name: name.to_owned(), data: vec![1.0], shape: vec![1],
    };
    assert!(single_image_output(vec![], "image").unwrap_err().contains("exactly one"));
    assert!(single_image_output(vec![tensor("image"), tensor("aux")], "image")
        .unwrap_err().contains("exactly one"));
    assert!(single_image_output(vec![tensor("wrong")], "image")
        .unwrap_err().contains("expected output"));
    assert_eq!(single_image_output(vec![tensor("image")], "image").unwrap().data, vec![1.0]);
}

#[test]
fn image_interface_is_validated_before_pixels_are_preprocessed() {
    struct InvalidImage;
    impl TensorRunner for InvalidImage {
        fn validate_image(&self) -> Result<(), String> {
            Err("image model requires exactly one named input and output".to_owned())
        }
        fn run_f32(&mut self, _: Vec<f32>, _: Vec<i64>) -> Result<crate::inference::TensorOutput, String> {
            panic!("incompatible image models must never execute");
        }
    }
    let error = run_image_task_with_runner(&mut InvalidImage, InferenceTask::Generic,
        "invalid", &[], 0, 0, 640, 0.25, 0.45).unwrap_err();
    assert_eq!(error, "image model requires exactly one named input and output");
}

fn task_info(input: &[u64], output: &[u64]) -> super::super::InferenceSessionInfo {
    let tensor = |name: &str, shape: &[u64]| oq_inference::TensorMetadata {
        name: name.to_owned(),
        element_type: oq_inference::TensorElementType::Float32,
        dimensions: shape.iter().copied().map(oq_inference::TensorDimension::Static).collect(),
    };
    super::super::InferenceSessionInfo {
        input_names: vec!["pixels".to_owned()], output_names: vec!["prediction".to_owned()],
        inputs: vec![tensor("pixels", input)], outputs: vec![tensor("prediction", output)],
        ..Default::default()
    }
}

fn task_config(task: InferenceTask, model: &str) -> super::super::InferenceConfig {
    super::super::InferenceConfig { task, model_id: model.to_owned(), ..Default::default() }
}

#[test]
fn task_contracts_preserve_catalog_profiles_and_generic_identity() {
    for (task, model, input, output, adapter) in [
        (InferenceTask::Generic, "identity", vec![1, 3, 2, 2], vec![1, 3, 2, 2], ImageTaskAdapter::RgbImage),
        (InferenceTask::SuperResolution, "super-resolution-3x", vec![1, 1, 224, 224], vec![1, 1, 672, 672], ImageTaskAdapter::LuminanceSuperResolution3x),
        (InferenceTask::SuperResolution, "realesrgan-x4", vec![1, 3, 2, 2], vec![1, 3, 8, 8], ImageTaskAdapter::RgbImage),
        (InferenceTask::Detection, "yolov8n", vec![1, 3, 640, 640], vec![1, 84, 8400], ImageTaskAdapter::CocoYolo8400),
        (InferenceTask::Segmentation, "yolo26n-sem", vec![1, 3, 640, 640], vec![1, 640, 640], ImageTaskAdapter::UInt8ClassMap),
        (InferenceTask::Segmentation, "generic-segmentation", vec![1, 3, 640, 640], vec![1, 19, 160, 160], ImageTaskAdapter::Cityscapes19),
        (InferenceTask::BackgroundRemoval, "u2netp", vec![1, 3, 320, 320], vec![1, 1, 320, 320], ImageTaskAdapter::AlphaMask),
        (InferenceTask::BackgroundRemoval, "modnet", vec![1, 3, 512, 512], vec![1, 1, 512, 512], ImageTaskAdapter::AlphaMask),
        (InferenceTask::DepthEstimation, "midas-small", vec![1, 3, 256, 256], vec![1, 256, 256], ImageTaskAdapter::DepthMap),
        (InferenceTask::DepthEstimation, "midas-small", vec![1, 3, 256, 256], vec![1, 1, 256, 256], ImageTaskAdapter::DepthMap),
    ] {
        let mut info = task_info(&input, &output);
        if model == "u2netp" {
            info.output_names[0] = "1959".to_owned();
            info.outputs[0].name = "1959".to_owned();
        }
        if model == "yolo26n-sem" { info.outputs[0].element_type = oq_inference::TensorElementType::Uint8; }
        let contract = image_task_contract(&task_config(task, model), &info).unwrap();
        assert_eq!(contract.adapter, adapter);
        assert_eq!(contract.input.name, "pixels");
        assert_eq!(contract.output.name, if model == "u2netp" { "1959" } else { "prediction" });
        assert_eq!(contract.input_layout, "NCHW");
    }
    let depth = image_task_contract(&task_config(InferenceTask::DepthEstimation, "midas-small"),
        &task_info(&[1, 3, 256, 256], &[1, 256, 256])).unwrap();
    assert_eq!(depth.preprocessing, ImageTaskPreprocessing::RgbZeroToOne);
}

#[test]
fn task_contracts_reject_static_incompatible_channels_ranks_and_profiles() {
    for (task, model, input, output) in [
        (InferenceTask::Generic, "identity", vec![1, 3, 2, 2], vec![1, 1, 2, 2]),
        (InferenceTask::Generic, "yolov8n", vec![1, 3, 640, 640], vec![1, 84, 8400]),
        (InferenceTask::Detection, "yolov8n", vec![1, 3, 320, 320], vec![1, 84, 8400]),
        (InferenceTask::Detection, "yolov8n", vec![1, 3, 640, 640], vec![1, 8400, 84]),
        (InferenceTask::Detection, "yolov8n", vec![1, 3, 640, 640], vec![1, 84, 1]),
        (InferenceTask::Detection, "yolov8n", vec![1, 3, 640, 640], vec![1, 84, 1, 8400]),
        (InferenceTask::SuperResolution, "super-resolution-3x", vec![1, 3, 224, 224], vec![1, 1, 672, 672]),
        (InferenceTask::SuperResolution, "super-resolution-3x", vec![1, 1, 256, 256], vec![1, 1, 672, 672]),
        (InferenceTask::SuperResolution, "super-resolution-3x", vec![1, 1, 224, 224], vec![1, 3, 672, 672]),
        (InferenceTask::SuperResolution, "super-resolution-3x", vec![1, 1, 224, 224], vec![1, 1, 448, 448]),
        (InferenceTask::SuperResolution, "realesrgan-x4", vec![1, 3, 2, 2], vec![1, 3, 4, 4]),
        (InferenceTask::Segmentation, "segment", vec![1, 3, 640, 640], vec![1, 20, 160, 160]),
        (InferenceTask::Segmentation, "segment", vec![1, 3, 640, 640], vec![1, 19, 80, 160]),
        (InferenceTask::Generic, "identity", vec![2, 3, 2, 2], vec![1, 3, 2, 2]),
        (InferenceTask::Generic, "identity", vec![1, 2, 2, 3], vec![1, 3, 2, 2]),
    ] {
        assert!(image_task_contract(&task_config(task, model), &task_info(&input, &output)).is_err(), "{model}: {input:?} -> {output:?}");
    }
    let config = task_config(InferenceTask::Generic, "identity");
    let mut info = task_info(&[1, 3, 2, 2], &[1, 3, 2, 2]);
    info.outputs[0].element_type = oq_inference::TensorElementType::Int64;
    assert!(image_task_contract(&config, &info).unwrap_err().contains("FP32"));
    info.outputs[0].element_type = oq_inference::TensorElementType::Float32;
    info.output_names[0] = "wrong".to_owned();
    assert!(image_task_contract(&config, &info).unwrap_err().contains("names"));
}

#[test]
fn symbolic_task_output_constraints_bind_to_actual_results() {
    use oq_inference::TensorDimension::{Dynamic, Symbol};
    let config = task_config(InferenceTask::Detection, "yolov8n");
    let mut info = task_info(&[1, 3, 640, 640], &[1, 84, 8400]);
    info.outputs[0].dimensions = vec![Symbol("batch".into()), Dynamic, Symbol("boxes".into())];
    let contract = image_task_contract(&config, &info).unwrap();
    let input = [1, 3, 640, 640];
    assert!(contract.validate_output(&TensorOutput { data: vec![0.0; 84 * 8400], shape: vec![1, 84, 8400] }, &input).is_ok());
    assert!(contract.validate_output(&TensorOutput { data: vec![0.0; 84], shape: vec![1, 84, 1] }, &input).is_err());
    assert!(contract.validate_output(&TensorOutput { data: vec![0.0; 84 * 8400], shape: vec![1, 8400, 84] }, &input).is_err());
    assert!(contract.validate_output(&TensorOutput { data: vec![], shape: vec![1, 84, 8400] }, &input).is_err());

    let mut info = task_info(&[1, 1, 224, 224], &[1, 1, 672, 672]);
    info.outputs[0].dimensions = vec![Dynamic; 4];
    let contract = image_task_contract(&task_config(InferenceTask::SuperResolution, "super-resolution-3x"), &info).unwrap();
    assert!(contract.validate_output(&TensorOutput { data: vec![0.0; 672 * 672], shape: vec![1, 1, 672, 672] }, &[1, 1, 224, 224]).is_ok());
    assert!(contract.validate_output(&TensorOutput { data: vec![0.0; 4], shape: vec![1, 1, 2, 2] }, &[1, 1, 224, 224]).is_err());

    let mut info = task_info(&[1, 3, 640, 640], &[1, 19, 2, 2]);
    info.outputs[0].dimensions = vec![Dynamic; 4];
    let contract = image_task_contract(&task_config(InferenceTask::Segmentation, "segment"), &info).unwrap();
    assert!(contract.validate_output(&TensorOutput { data: vec![0.0; 19 * 2 * 3], shape: vec![1, 19, 2, 3] }, &input).is_err());

    let mut info = task_info(&[1, 3, 2, 2], &[1, 3, 8, 8]);
    info.inputs[0].dimensions = vec![Dynamic; 4];
    info.outputs[0].dimensions = vec![Dynamic; 4];
    let contract = image_task_contract(&task_config(InferenceTask::SuperResolution, "realesrgan-x4"), &info).unwrap();
    assert!(contract.validate_input(&[1, 3, 2, 2], 12).is_ok());
    assert!(contract.validate_input(&[1, 1, 2, 2], 4).is_err());
    assert!(contract.validate_input(&[2, 3, 2, 2], 24).is_err());
    assert!(contract.validate_output(&TensorOutput { data: vec![0.0; 3 * 8 * 8], shape: vec![1, 3, 8, 8] }, &[1, 3, 2, 2]).is_ok());
    assert!(contract.validate_output(&TensorOutput { data: vec![0.0; 3 * 4 * 4], shape: vec![1, 3, 4, 4] }, &[1, 3, 2, 2]).is_err());
}

#[test]
fn realesrgan_consumer_requires_exact_four_times_spatial_shape() {
    use oq_inference::TensorDimension::{Static, Symbol};
    let mut info = task_info(&[1, 3, 2, 3], &[1, 3, 8, 12]);
    // The original artifact reuses input height/width symbols on its scaled output.
    let dimensions = vec![Static(1), Static(3), Symbol("height".into()), Symbol("width".into())];
    info.inputs[0].dimensions = dimensions.clone();
    info.outputs[0].dimensions = dimensions;
    let contract = image_task_contract(&task_config(InferenceTask::SuperResolution, "realesrgan-x4"), &info).unwrap();
    assert!(contract.validate_input(&[1, 3, 2, 3], 18).is_ok());
    assert_eq!(contract.concrete_output_shape(&[1, 3, 2, 3]).unwrap(), [1, 3, 8, 12]);
    assert_eq!(info.outputs[0].dimensions[2], Symbol("height".into()));
    assert!(contract.concrete_output_shape(&[1, 1, 2, 3]).is_err());
    assert!(contract.validate_output(&TensorOutput { data: vec![0.0; 3 * 8 * 12], shape: vec![1, 3, 8, 12] }, &[1, 3, 2, 3]).is_ok());
    assert!(contract.validate_output(&TensorOutput { data: vec![0.0; 18], shape: vec![1, 3, 2, 3] }, &[1, 3, 2, 3]).is_err());
    let source = vec![0u8; 3 * 2 * 4];
    let mut exact = FakeRunner::one(vec![0.5; 3 * 8 * 12], vec![1, 3, 8, 12]);
    let output = run_super_resolution(&mut exact, "realesrgan-x4", &source, 3, 2).unwrap();
    assert_eq!((output.width, output.height, output.rgba.len()), (12, 8, 12 * 8 * 4));
    for shape in [vec![1, 3, 2, 3], vec![1, 3, 4, 6], vec![1, 3, 12, 8]] {
        let count = shape.iter().product::<i64>() as usize;
        let mut incorrect = FakeRunner::one(vec![0.5; count], shape);
        assert!(run_super_resolution(&mut incorrect, "realesrgan-x4", &source, 3, 2).is_err());
    }
}

#[test]
fn decoder_never_guesses_image_channels_from_length() {
    for (data, shape) in [
        (vec![0.5; 4], vec![1, 1, 2, 2]),
        (vec![0.5; 12], vec![1, 1, 2, 2]),
        (vec![0.5; 12], vec![1, 3, 4]),
    ] {
        assert!(run_generic(&mut FakeRunner::one(data, shape), &rgba(), 2, 2).is_err());
    }
    let mut depth = FakeRunner::one(vec![0.5; 4], vec![1, 2, 2]);
    assert!(run_depth(&mut depth, "midas", &rgba(), 2, 2).is_ok());
    let mut detector = FakeRunner::one(vec![0.0; 84], vec![1, 84, 1]);
    assert!(run_detection(&mut detector, &rgba(), 2, 2, 2, 0.25, 0.45).is_err());
}

#[test]
fn catalog_u2netp_binds_primary_mask_without_hiding_side_outputs() {
    let mut info = task_info(&[1, 3, 320, 320], &[1, 1, 320, 320]);
    let template = info.outputs[0].clone();
    info.outputs = (1959..=1965).rev().map(|name| oq_inference::TensorMetadata {
        name: name.to_string(), ..template.clone()
    }).collect();
    info.output_names = info.outputs.iter().map(|output| output.name.clone()).collect();
    let config = task_config(InferenceTask::BackgroundRemoval, "u2netp");
    let contract = image_task_contract(&config, &info).unwrap();
    assert_eq!(contract.output.name, "1959");
    assert_eq!(info.outputs.len(), 7);
    assert_eq!(info.output_names.len(), 7);
    assert!(image_task_contract(&task_config(InferenceTask::Generic, "u2netp"), &info).is_err());
    assert!(image_task_contract(&task_config(InferenceTask::BackgroundRemoval, "custom"), &info).is_err());

    let output = single_image_output(vec![super::super::NamedTensor {
        name: "1959".to_owned(), data: vec![0.5; 320 * 320], shape: vec![1, 1, 320, 320],
    }], &contract.output.name).unwrap();
    assert!(contract.validate_output(&output, &[1, 3, 320, 320]).is_ok());
    assert!(single_image_output(vec![super::super::NamedTensor {
        name: "1960".to_owned(), data: vec![0.5; 320 * 320], shape: vec![1, 1, 320, 320],
    }], &contract.output.name).is_err());

    info.outputs.retain(|output| output.name != "1959");
    info.output_names.retain(|name| name != "1959");
    assert!(image_task_contract(&config, &info).unwrap_err().contains("1959"));
}
