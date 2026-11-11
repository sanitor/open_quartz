#[cfg(not(target_arch = "wasm32"))]
use serde_json::{json, Value};

#[cfg(not(target_arch = "wasm32"))]
use super::{
    apply_alpha_mask, detect_postprocess, letterbox_preprocess, midas_preprocess,
    rgb_output_to_rgba, rgba_to_chw, segment_postprocess, InferenceSession, InferenceTask, TensorOutput, RequestStamp,
};

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug)]
pub struct InferenceImageOutput {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub data: Option<Value>,
}

pub(crate) trait TensorRunner {
    fn validate_image(&self) -> Result<(), String> { Ok(()) }
    fn run_f32(&mut self, input: Vec<f32>, shape: Vec<i64>) -> Result<TensorOutput, String>;
    fn run_u8(&mut self, _input: Vec<f32>, _shape: Vec<i64>) -> Result<(Vec<u8>, Vec<i64>), String> {
        Err("this diagnostic tensor runner does not support UInt8 outputs".to_owned())
    }
}

#[derive(Clone, Debug, Default)]
pub struct InferenceImageMetrics {
    pub preprocess_ns: u64,
    pub runtime_ns: u64,
    pub postprocess_ns: u64,
    pub input_cpu_bytes: u64,
    pub output_cpu_bytes: u64,
}

pub fn validate_image_session(session: &InferenceSession) -> Result<(), String> {
    session.validate_single_image().map_err(|error| error.to_string())?;
    session.validate_image_shape().map_err(|error| error.to_string())
}


use super::contract::*;
pub fn validate_image_task_session(
    session: &InferenceSession,
    config: &super::InferenceConfig,
) -> Result<ImageTaskContract, String> {
    // Task validation checks the one input and explicitly selected output, not
    // the generic single-output image convenience API's cardinality gate.
    image_task_contract(config, session.info())
}

struct StampedImageRunner<'a> {
    session: &'a mut InferenceSession,
    contract: ImageTaskContract,
    stamp: RequestStamp,
    metrics: InferenceImageMetrics,
    stage_started: std::time::Instant,
}

fn duration_ns(duration: std::time::Duration) -> u64 {
    duration.as_nanos().min(u64::MAX as u128) as u64
}

impl TensorRunner for StampedImageRunner<'_> {
    fn validate_image(&self) -> Result<(), String> {
        validate_declared_shape(&self.contract.input, &self.contract.input_shape, &self.contract.input.dimensions)?;
        validate_declared_shape(&self.contract.output, &self.contract.output_shape, &self.contract.input.dimensions)
    }

    fn run_f32(&mut self, input: Vec<f32>, shape: Vec<i64>) -> Result<TensorOutput, String> {
        self.contract.validate_input(&shape, input.len())?;
        let input_shape: [i64; 4] = shape.as_slice().try_into()
            .map_err(|_| "image task input requires rank 4".to_owned())?;
        self.metrics.preprocess_ns = self.metrics.preprocess_ns.saturating_add(duration_ns(self.stage_started.elapsed()));
        let output_names = std::slice::from_ref(&self.contract.output.name);
        let inputs = vec![super::NamedTensor { name: self.contract.input.name.clone(), data: input, shape }];
        let result = if self.contract.output_shape.iter().any(|constraint| matches!(constraint,
            ImageDimensionConstraint::InputHeightTimes(_) | ImageDimensionConstraint::InputWidthTimes(_))) {
            let output_shape = self.contract.concrete_output_shape(&input_shape)?;
            self.session.invoke_with_output_shapes(self.stamp, inputs, output_names,
                &[(self.contract.output.name.as_str(), output_shape.as_slice())])
        } else { self.session.invoke(self.stamp, inputs, output_names) }
            .map_err(|error| error.to_string())?;
        self.stage_started = std::time::Instant::now();
        if result.stamp != self.stamp {
            return Err("image invocation returned a different request stamp".to_owned());
        }
        self.metrics.runtime_ns = self.metrics.runtime_ns.saturating_add(duration_ns(result.metrics.runtime_duration));
        self.metrics.input_cpu_bytes = self.metrics.input_cpu_bytes.saturating_add(result.metrics.input_cpu_bytes);
        self.metrics.output_cpu_bytes = self.metrics.output_cpu_bytes.saturating_add(result.metrics.output_cpu_bytes);
        let output = single_image_output(result.outputs, &self.contract.output.name)?;
        self.contract.validate_output(&output, &input_shape)?;
        Ok(output)
    }

    fn run_u8(&mut self, input: Vec<f32>, shape: Vec<i64>) -> Result<(Vec<u8>, Vec<i64>), String> {
        self.contract.validate_input(&shape, input.len())?;
        self.metrics.preprocess_ns = self.metrics.preprocess_ns.saturating_add(duration_ns(self.stage_started.elapsed()));
        let input_bytes = (input.len() as u64).saturating_mul(4);
        let started = std::time::Instant::now();
        let output = self.session.run_u8(input, shape.clone()).map_err(|error| error.to_string())?;
        self.metrics.runtime_ns = self.metrics.runtime_ns.saturating_add(duration_ns(started.elapsed()));
        self.metrics.input_cpu_bytes = self.metrics.input_cpu_bytes.saturating_add(input_bytes);
        self.metrics.output_cpu_bytes = self.metrics.output_cpu_bytes.saturating_add(output.0.len() as u64);
        self.stage_started = std::time::Instant::now();
        self.contract.validate_u8_output(&output.1, output.0.len(), &shape)?;
        Ok(output)
    }
}

fn single_image_output(mut outputs: Vec<super::NamedTensor>, expected_name: &str) -> Result<TensorOutput, String> {
    if outputs.len() != 1 {
        return Err(format!("image invocation requires exactly one output, received {}", outputs.len()));
    }
    let output = outputs.pop().expect("one output was checked");
    if output.name != expected_name {
        return Err(format!("image invocation expected output {expected_name:?}, received {:?}", output.name));
    }
    Ok(TensorOutput { data: output.data, shape: output.shape })
}

/// Run an image task with the caller's independent request identity. The backend
/// must return the same request and session generation and exactly one named output.
#[allow(clippy::too_many_arguments)]
pub fn run_image_task_with_request(
    session: &mut InferenceSession,
    stamp: RequestStamp,
    task: InferenceTask,
    model_id: &str,
    source_rgba: &[u8],
    source_width: u32,
    source_height: u32,
    target_size: u32,
    score_threshold: f32,
    iou_threshold: f32,
) -> Result<InferenceImageOutput, String> {
    run_image_task_with_request_and_metrics(session, stamp, task, model_id, source_rgba,
        source_width, source_height, target_size, score_threshold, iou_threshold)
        .map(|(image, _)| image)
}

#[allow(clippy::too_many_arguments)]
pub fn run_image_task_with_request_and_metrics(
    session: &mut InferenceSession,
    stamp: RequestStamp,
    task: InferenceTask,
    model_id: &str,
    source_rgba: &[u8],
    source_width: u32,
    source_height: u32,
    target_size: u32,
    score_threshold: f32,
    iou_threshold: f32,
) -> Result<(InferenceImageOutput, InferenceImageMetrics), String> {
    let config = super::InferenceConfig {
        model_id: model_id.to_owned(), task, target_size, score_threshold, iou_threshold,
        policy: session.plan().requested_policy,
    };
    let contract = validate_image_task_session(session, &config)?;
    if stamp.session_generation != session.generation() {
        return Err("image request belongs to a different session generation".to_owned());
    }
    let mut runner = StampedImageRunner {
        session, contract, stamp, metrics: InferenceImageMetrics::default(),
        stage_started: std::time::Instant::now(),
    };
    let image = run_image_task_with_runner(&mut runner, task, model_id, source_rgba,
        source_width, source_height, target_size, score_threshold, iou_threshold)?;
    runner.metrics.postprocess_ns = duration_ns(runner.stage_started.elapsed());
    Ok((image, runner.metrics))
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_arguments)]
pub fn run_image_task(
    session: &mut InferenceSession,
    task: InferenceTask,
    model_id: &str,
    source_rgba: &[u8],
    source_width: u32,
    source_height: u32,
    target_size: u32,
    score_threshold: f32,
    iou_threshold: f32,
) -> Result<InferenceImageOutput, String> {
    static NEXT_REQUEST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let stamp = RequestStamp {
        request_id: NEXT_REQUEST.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        session_generation: session.generation(),
    };
    run_image_task_with_request(
        session, stamp, task, model_id, source_rgba, source_width, source_height,
        target_size, score_threshold, iou_threshold,
    )
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_arguments)]
fn run_image_task_with_runner(
    session: &mut impl TensorRunner,
    task: InferenceTask,
    model_id: &str,
    source_rgba: &[u8],
    source_width: u32,
    source_height: u32,
    target_size: u32,
    score_threshold: f32,
    iou_threshold: f32,
) -> Result<InferenceImageOutput, String> {
    session.validate_image()?;
    match task {
        InferenceTask::SuperResolution => {
            run_super_resolution(session, model_id, source_rgba, source_width, source_height)
        }
        InferenceTask::BackgroundRemoval => {
            run_background_removal(session, model_id, source_rgba, source_width, source_height)
        }
        InferenceTask::DepthEstimation => run_depth(session, model_id, source_rgba, source_width, source_height),
        InferenceTask::Detection => run_detection(
            session,
            source_rgba,
            source_width,
            source_height,
            if model_id == "yolov8n" { 640 } else { target_size.max(1) },
            score_threshold,
            iou_threshold,
        ),
        InferenceTask::Segmentation if model_id == "yolo26n-sem" => run_class_map(
            session, source_rgba, source_width, source_height,
        ),
        InferenceTask::Segmentation => run_segmentation(
            session,
            source_rgba,
            source_width,
            source_height,
            target_size.max(1),
        ),
        InferenceTask::StyleTransfer | InferenceTask::Denoising | InferenceTask::Generic => {
            run_generic(session, source_rgba, source_width, source_height)
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn run_generic(
    session: &mut impl TensorRunner,
    rgba: &[u8],
    width: u32,
    height: u32,
) -> Result<InferenceImageOutput, String> {
    let tensor = rgba_to_chw(rgba, width, height)?;
    let output = session.run_f32(tensor, vec![1, 3, height as i64, width as i64])?;
    let (output_width, output_height) = image_output_size(&output, 3, false)?;
    let rgba = rgb_output_to_rgba(&output.data, output_width, output_height)?;
    Ok(InferenceImageOutput {
        rgba,
        width: output_width,
        height: output_height,
        data: None,
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn run_super_resolution(
    session: &mut impl TensorRunner,
    model_id: &str,
    rgba: &[u8],
    width: u32,
    height: u32,
) -> Result<InferenceImageOutput, String> {
    if model_id == "super-resolution-3x" {
        const INPUT: u32 = 224;
        let resized = resize_rgba(rgba, width, height, INPUT, INPUT)?;
        let pixels = (INPUT * INPUT) as usize;
        let mut y = vec![0.0; pixels];
        let mut cb = vec![0.0; pixels];
        let mut cr = vec![0.0; pixels];
        for index in 0..pixels {
            let red = resized[index * 4] as f32 / 255.0;
            let green = resized[index * 4 + 1] as f32 / 255.0;
            let blue = resized[index * 4 + 2] as f32 / 255.0;
            y[index] = 0.299 * red + 0.587 * green + 0.114 * blue;
            cb[index] = -0.169 * red - 0.331 * green + 0.5 * blue + 0.5;
            cr[index] = 0.5 * red - 0.419 * green - 0.081 * blue + 0.5;
        }
        let output = session.run_f32(y, vec![1, 1, INPUT as i64, INPUT as i64])?;
        validate_output_shape(&output, &[Some(1), Some(1), Some(672), Some(672)])?;
        let (model_width, model_height) = (672, 672);
        let mut model_rgba = vec![0; (model_width * model_height * 4) as usize];
        for output_y in 0..model_height {
            for output_x in 0..model_width {
                let output_index = (output_y * model_width + output_x) as usize;
                let source_x = (output_x * INPUT / model_width).min(INPUT - 1);
                let source_y = (output_y * INPUT / model_height).min(INPUT - 1);
                let chroma_index = (source_y * INPUT + source_x) as usize;
                let luminance = output.data[output_index];
                let blue_difference = cb[chroma_index] - 0.5;
                let red_difference = cr[chroma_index] - 0.5;
                model_rgba[output_index * 4] = to_byte(luminance + 1.402 * red_difference);
                model_rgba[output_index * 4 + 1] =
                    to_byte(luminance - 0.344 * blue_difference - 0.714 * red_difference);
                model_rgba[output_index * 4 + 2] = to_byte(luminance + 1.772 * blue_difference);
                model_rgba[output_index * 4 + 3] = 255;
            }
        }
        let output_width = width.saturating_mul(3);
        let output_height = height.saturating_mul(3);
        let rgba = resize_rgba(
            &model_rgba,
            model_width,
            model_height,
            output_width,
            output_height,
        )
        .expect("generated model RGBA dimensions are valid");
        return Ok(InferenceImageOutput {
            rgba,
            width: output_width,
            height: output_height,
            data: None,
        });
    }

    let tensor = rgba_to_chw(rgba, width, height)?;
    let output = session.run_f32(tensor, vec![1, 3, height as i64, width as i64])?;
    if model_id == "realesrgan-x4" {
        let output_width = width.checked_mul(4).ok_or("RealESRGAN output width overflow")?;
        let output_height = height.checked_mul(4).ok_or("RealESRGAN output height overflow")?;
        validate_output_shape(&output, &[Some(1), Some(3), Some(output_height as i64), Some(output_width as i64)])?;
    }
    let (output_width, output_height) = image_output_size(&output, 3, false)?;
    Ok(InferenceImageOutput {
        rgba: rgb_output_to_rgba(&output.data, output_width, output_height)?,
        width: output_width,
        height: output_height,
        data: None,
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn run_background_removal(
    session: &mut impl TensorRunner,
    model_id: &str,
    rgba: &[u8],
    width: u32,
    height: u32,
) -> Result<InferenceImageOutput, String> {
    let input_size = if model_id == "u2netp" { 320 } else { 512 };
    let resized = resize_rgba(rgba, width, height, input_size, input_size)?;
    let tensor =
        rgba_to_chw(&resized, input_size, input_size).expect("resized RGBA dimensions are valid");
    let output = session.run_f32(tensor, vec![1, 3, input_size as i64, input_size as i64])?;
    let (mask_width, mask_height) = image_output_size(&output, 1, false)?;
    let resized_mask_source =
        resize_rgba(&resized, input_size, input_size, mask_width, mask_height)
            .expect("validated RGBA can always be resized");
    let mask_rgba = apply_alpha_mask(&resized_mask_source, &output.data, mask_width, mask_height)?;
    let rgba = resize_rgba(&mask_rgba, mask_width, mask_height, width, height)
        .expect("generated alpha mask dimensions are valid");
    Ok(InferenceImageOutput {
        rgba,
        width,
        height,
        data: None,
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn run_depth(
    session: &mut impl TensorRunner,
    model_id: &str,
    rgba: &[u8],
    width: u32,
    height: u32,
) -> Result<InferenceImageOutput, String> {
    const INPUT: u32 = 256;
    let resized = resize_rgba(rgba, width, height, INPUT, INPUT)?;
    let tensor = if model_id == "midas-small" {
        // The original MiDaS graph contains its own RGB channel Sub/Div.
        rgba_to_chw(&resized, INPUT, INPUT)
    } else { midas_preprocess(&resized, INPUT, INPUT) }
        .expect("resized depth RGBA dimensions are valid");
    let output = session.run_f32(tensor, vec![1, 3, INPUT as i64, INPUT as i64])?;
    let (depth_width, depth_height) = image_output_size(&output, 1, true)?;
    let count = output.data.len();
    let depth = &output.data;
    let minimum = depth.iter().copied().fold(f32::INFINITY, f32::min);
    let maximum = depth.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let range = (maximum - minimum).max(f32::EPSILON);
    let mut depth_rgba = vec![0; count * 4];
    for (index, value) in depth.iter().copied().enumerate() {
        let gray = to_byte((value - minimum) / range);
        depth_rgba[index * 4..index * 4 + 4].copy_from_slice(&[gray, gray, gray, 255]);
    }
    let rgba = resize_rgba(&depth_rgba, depth_width, depth_height, width, height)
        .expect("generated depth RGBA dimensions are valid");
    Ok(InferenceImageOutput {
        rgba,
        width,
        height,
        data: None,
    })
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_arguments)]
fn run_detection(
    session: &mut impl TensorRunner,
    rgba: &[u8],
    width: u32,
    height: u32,
    target_size: u32,
    score_threshold: f32,
    iou_threshold: f32,
) -> Result<InferenceImageOutput, String> {
    let input = letterbox_preprocess(rgba, width, height, target_size)?;
    let output = session.run_f32(
        input.tensor,
        input
            .shape
            .iter()
            .map(|dimension| *dimension as i64)
            .collect(),
    )?;
    // The existing COCO decoder has a fixed 8400-box channel stride.
    validate_output_shape(&output, &[Some(1), Some(84), Some(8400)])?;
    let detections = detect_postprocess(
        &output.data,
        width,
        height,
        input.scale,
        input.pad_x,
        input.pad_y,
        score_threshold,
        iou_threshold,
    );
    let mut overlay = rgba.to_vec();
    for detection in &detections {
        // Structured detections stay normalized; only rasterization uses pixels.
        let [left, top, right, bottom] = detection.bbox;
        draw_box(
            &mut overlay,
            width,
            height,
            [left * width as f32, top * height as f32,
                right * width as f32, bottom * height as f32],
            [255, 59, 48, 255],
        );
    }
    Ok(InferenceImageOutput {
        rgba: overlay,
        width,
        height,
        data: Some(json!(detections)),
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn run_class_map(session: &mut impl TensorRunner, rgba: &[u8], width: u32, height: u32)
    -> Result<InferenceImageOutput, String> {
    const SIDE: u32 = 640;
    let input = letterbox_preprocess(rgba, width, height, SIDE)?;
    let (classes, shape) = session.run_u8(input.tensor, vec![1, 3, SIDE as i64, SIDE as i64])?;
    if shape != [1, SIDE as i64, SIDE as i64] || classes.len() != (SIDE * SIDE) as usize {
        return Err("YOLO26 requires its original UInt8 class map [1,640,640]".to_owned());
    }
    let segmentation = super::postprocess::class_map_postprocess(&classes, SIDE as usize, SIDE as usize,
        width as usize, height as usize, input.scale, input.pad_x, input.pad_y)?;
    let mut overlay = rgba.to_vec();
    for (pixel, color) in overlay.chunks_exact_mut(4).zip(segmentation.mask_rgba.chunks_exact(4)) {
        for channel in 0..3 { pixel[channel] = blend(pixel[channel], color[channel]); }
    }
    Ok(InferenceImageOutput { rgba: overlay, width, height, data: Some(json!({
        "classCounts": segmentation.class_counts, "numClasses": segmentation.num_classes,
        "maskW": width, "maskH": height,
    })) })
}

#[cfg(not(target_arch = "wasm32"))]
fn run_segmentation(
    session: &mut impl TensorRunner,
    rgba: &[u8],
    width: u32,
    height: u32,
    target_size: u32,
) -> Result<InferenceImageOutput, String> {
    let input = letterbox_preprocess(rgba, width, height, target_size)?;
    let output = session.run_f32(
        input.tensor,
        input
            .shape
            .iter()
            .map(|dimension| *dimension as i64)
            .collect(),
    )?;
    let (mask_width, mask_height) = image_output_size(&output, 19, false)?;
    if mask_width != mask_height {
        return Err("Cityscapes decoder requires a square [1,19,H,H] output".to_owned());
    }
    let segmentation = segment_postprocess(
        &output.data,
        width as usize,
        height as usize,
        input.scale,
        input.pad_x,
        input.pad_y,
    )?;
    let mut overlay = rgba.to_vec();
    for index in 0..(width as usize * height as usize) {
        overlay[index * 4] = blend(overlay[index * 4], segmentation.mask_rgba[index * 4]);
        overlay[index * 4 + 1] = blend(
            overlay[index * 4 + 1],
            segmentation.mask_rgba[index * 4 + 1],
        );
        overlay[index * 4 + 2] = blend(
            overlay[index * 4 + 2],
            segmentation.mask_rgba[index * 4 + 2],
        );
    }
    Ok(InferenceImageOutput {
        rgba: overlay,
        width,
        height,
        data: Some(json!({
            "classCounts": segmentation.class_counts,
            "numClasses": segmentation.num_classes,
            "maskW": width,
            "maskH": height,
        })),
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn validate_output_shape(output: &TensorOutput, expected: &[Option<i64>]) -> Result<(), String> {
    if output.shape.len() != expected.len() {
        return Err(format!("task output requires shape {expected:?}, received {:?}", output.shape));
    }
    let mut length = 1usize;
    for (&actual, expected) in output.shape.iter().zip(expected) {
        if actual <= 0 || actual > u32::MAX as i64 || expected.is_some_and(|expected| expected != actual) {
            return Err(format!("task output requires shape {expected:?}, received {:?}", output.shape));
        }
        length = length.checked_mul(actual as usize)
            .ok_or_else(|| "task output tensor size overflow".to_owned())?;
    }
    if length != output.data.len() {
        return Err(format!("task output has {} values, expected {length}", output.data.len()));
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn image_output_size(output: &TensorOutput, channels: i64, allow_depth_rank3: bool) -> Result<(u32, u32), String> {
    if allow_depth_rank3 && output.shape.len() == 3 {
        validate_output_shape(output, &[Some(1), None, None])?;
    } else {
        validate_output_shape(output, &[Some(1), Some(channels), None, None])?;
    }
    let rank = output.shape.len();
    Ok((output.shape[rank - 1] as u32, output.shape[rank - 2] as u32))
}

#[cfg(not(target_arch = "wasm32"))]
fn resize_rgba(
    source: &[u8],
    source_width: u32,
    source_height: u32,
    target_width: u32,
    target_height: u32,
) -> Result<Vec<u8>, String> {
    if source_width == 0
        || source_height == 0
        || target_width == 0
        || target_height == 0
        || source.len() != (source_width * source_height * 4) as usize
    {
        return Err("RGBA resize dimensions do not match the source buffer".to_owned());
    }
    let mut output = vec![0; (target_width * target_height * 4) as usize];
    for y in 0..target_height {
        let source_y = (y * source_height / target_height).min(source_height - 1);
        for x in 0..target_width {
            let source_x = (x * source_width / target_width).min(source_width - 1);
            let source_index = ((source_y * source_width + source_x) * 4) as usize;
            let target_index = ((y * target_width + x) * 4) as usize;
            output[target_index..target_index + 4]
                .copy_from_slice(&source[source_index..source_index + 4]);
        }
    }
    Ok(output)
}

#[cfg(not(target_arch = "wasm32"))]
fn draw_box(rgba: &mut [u8], width: u32, height: u32, bbox: [f32; 4], color: [u8; 4]) {
    let left = bbox[0].floor().clamp(0.0, width.saturating_sub(1) as f32) as u32;
    let top = bbox[1].floor().clamp(0.0, height.saturating_sub(1) as f32) as u32;
    let right = bbox[2].ceil().clamp(0.0, width.saturating_sub(1) as f32) as u32;
    let bottom = bbox[3].ceil().clamp(0.0, height.saturating_sub(1) as f32) as u32;
    for thickness in 0..2 {
        let y1 = (top + thickness).min(height - 1);
        let y2 = bottom.saturating_sub(thickness);
        for x in left..=right {
            set_pixel(rgba, width, x, y1, color);
            set_pixel(rgba, width, x, y2, color);
        }
        let x1 = (left + thickness).min(width - 1);
        let x2 = right.saturating_sub(thickness);
        for y in top..=bottom {
            set_pixel(rgba, width, x1, y, color);
            set_pixel(rgba, width, x2, y, color);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn set_pixel(rgba: &mut [u8], width: u32, x: u32, y: u32, color: [u8; 4]) {
    let index = ((y * width + x) * 4) as usize;
    if index + 4 <= rgba.len() {
        rgba[index..index + 4].copy_from_slice(&color);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn to_byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(not(target_arch = "wasm32"))]
fn blend(source: u8, overlay: u8) -> u8 {
    ((source as u16 + overlay as u16) / 2) as u8
}

#[cfg(test)]
#[path = "pipeline_coverage_tests.rs"]
mod coverage_tests;
