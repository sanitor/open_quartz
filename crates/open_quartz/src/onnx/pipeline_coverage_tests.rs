use super::*;
use std::collections::VecDeque;

struct FakeRunner {
    outputs: VecDeque<Result<crate::onnx::TensorOutput, String>>,
}

impl FakeRunner {
    fn one(data: Vec<f32>, shape: Vec<i64>) -> Self {
        Self {
            outputs: VecDeque::from([Ok(crate::onnx::TensorOutput { data, shape })]),
        }
    }
}

impl TensorRunner for FakeRunner {
    fn run_f32(
        &mut self,
        _input: Vec<f32>,
        _shape: Vec<i64>,
    ) -> Result<crate::onnx::TensorOutput, String> {
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
        OnnxTask::Generic,
        OnnxTask::StyleTransfer,
        OnnxTask::Denoising,
    ] {
        let mut runner = FakeRunner::one(vec![1.0; 12], vec![1, 3, 2, 2]);
        let output = run_native_image_task_with_runner(
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
    let output = run_native_image_task_with_runner(
        &mut super_resolution,
        OnnxTask::SuperResolution,
        "realesrgan-x4plus",
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
    let output = run_native_image_task_with_runner(
        &mut three_x,
        OnnxTask::SuperResolution,
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
        let output = run_native_image_task_with_runner(
            &mut background,
            OnnxTask::BackgroundRemoval,
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
    assert!(run_native_image_task_with_runner(
        &mut depth,
        OnnxTask::DepthEstimation,
        "midas",
        &source,
        2,
        2,
        2,
        0.25,
        0.45,
    )
    .is_ok());

    let mut detection = FakeRunner::one(vec![0.0; 84], vec![1, 84, 1]);
    let detected = run_native_image_task_with_runner(
        &mut detection,
        OnnxTask::Detection,
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
    let segmented = run_native_image_task_with_runner(
        &mut segmentation,
        OnnxTask::Segmentation,
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
fn native_pipeline_helpers_cover_shapes_pixels_resize_and_errors() {
    assert!(tensor_to_rgba(&[0.5], 1, 1).is_ok());
    assert!(tensor_to_rgba(&[1.0, 0.0, 0.0], 1, 1).is_ok());
    assert!(tensor_to_rgba(&[], 1, 1).is_err());
    assert_eq!(output_size(&[1, 3, 4, 5], 1, 2), (5, 4));
    assert_eq!(output_size(&[0, 0], 1, 2), (1, 2));
    assert_eq!(output_size(&[], 1, 2), (1, 2));
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
    assert!(run_depth(&mut short_depth, &rgba(), 2, 2).is_err());
    assert!(run_native_image_task_with_runner(
        &mut FakeRunner::one(vec![], vec![]),
        OnnxTask::Generic,
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
        (OnnxTask::SuperResolution, "realesrgan-x4plus"),
        (OnnxTask::BackgroundRemoval, "u2netp"),
        (OnnxTask::DepthEstimation, "depth"),
        (OnnxTask::Detection, "yolo"),
        (OnnxTask::Segmentation, "segment"),
    ] {
        assert!(run_native_image_task_with_runner(
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
        assert!(run_native_image_task_with_runner(
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
