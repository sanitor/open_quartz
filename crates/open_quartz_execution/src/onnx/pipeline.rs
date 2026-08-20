#[cfg(not(target_arch = "wasm32"))]
use serde_json::{json, Value};

#[cfg(not(target_arch = "wasm32"))]
use super::{
    apply_alpha_mask, detect_postprocess, letterbox_preprocess, midas_preprocess,
    rgb_output_to_rgba, rgba_to_chw, segment_postprocess, OnnxSession, OnnxTask,
};

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug)]
pub struct NativeOnnxImageOutput {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub data: Option<Value>,
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_arguments)]
pub fn run_native_image_task(
    session: &mut OnnxSession,
    task: OnnxTask,
    model_id: &str,
    source_rgba: &[u8],
    source_width: u32,
    source_height: u32,
    target_size: u32,
    score_threshold: f32,
    iou_threshold: f32,
) -> Result<NativeOnnxImageOutput, String> {
    match task {
        OnnxTask::SuperResolution => {
            run_super_resolution(session, model_id, source_rgba, source_width, source_height)
        }
        OnnxTask::BackgroundRemoval => {
            run_background_removal(session, model_id, source_rgba, source_width, source_height)
        }
        OnnxTask::DepthEstimation => run_depth(session, source_rgba, source_width, source_height),
        OnnxTask::Detection => run_detection(
            session,
            source_rgba,
            source_width,
            source_height,
            target_size.max(1),
            score_threshold,
            iou_threshold,
        ),
        OnnxTask::Segmentation => run_segmentation(
            session,
            source_rgba,
            source_width,
            source_height,
            target_size.max(1),
        ),
        OnnxTask::StyleTransfer | OnnxTask::Denoising | OnnxTask::Generic => {
            run_generic(session, source_rgba, source_width, source_height)
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn run_generic(
    session: &mut OnnxSession,
    rgba: &[u8],
    width: u32,
    height: u32,
) -> Result<NativeOnnxImageOutput, String> {
    let tensor = rgba_to_chw(rgba, width, height)?;
    let output = session.run_f32(tensor, vec![1, 3, height as i64, width as i64])?;
    let (output_width, output_height) = output_size(&output.shape, width, height);
    let rgba = tensor_to_rgba(&output.data, output_width, output_height)?;
    Ok(NativeOnnxImageOutput {
        rgba,
        width: output_width,
        height: output_height,
        data: None,
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn run_super_resolution(
    session: &mut OnnxSession,
    model_id: &str,
    rgba: &[u8],
    width: u32,
    height: u32,
) -> Result<NativeOnnxImageOutput, String> {
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
        let (model_width, model_height) = output_size(&output.shape, INPUT * 3, INPUT * 3);
        let mut model_rgba = vec![0; (model_width * model_height * 4) as usize];
        for output_y in 0..model_height {
            for output_x in 0..model_width {
                let output_index = (output_y * model_width + output_x) as usize;
                let source_x = (output_x * INPUT / model_width).min(INPUT - 1);
                let source_y = (output_y * INPUT / model_height).min(INPUT - 1);
                let chroma_index = (source_y * INPUT + source_x) as usize;
                let luminance = output.data.get(output_index).copied().unwrap_or_default();
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
        return Ok(NativeOnnxImageOutput {
            rgba: resize_rgba(
                &model_rgba,
                model_width,
                model_height,
                output_width,
                output_height,
            )?,
            width: output_width,
            height: output_height,
            data: None,
        });
    }

    let tensor = rgba_to_chw(rgba, width, height)?;
    let output = session.run_f32(tensor, vec![1, 3, height as i64, width as i64])?;
    let (output_width, output_height) = output_size(
        &output.shape,
        width.saturating_mul(4),
        height.saturating_mul(4),
    );
    Ok(NativeOnnxImageOutput {
        rgba: rgb_output_to_rgba(&output.data, output_width, output_height)?,
        width: output_width,
        height: output_height,
        data: None,
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn run_background_removal(
    session: &mut OnnxSession,
    model_id: &str,
    rgba: &[u8],
    width: u32,
    height: u32,
) -> Result<NativeOnnxImageOutput, String> {
    let input_size = if model_id == "u2netp" { 320 } else { 512 };
    let resized = resize_rgba(rgba, width, height, input_size, input_size)?;
    let tensor = rgba_to_chw(&resized, input_size, input_size)?;
    let output = session.run_f32(tensor, vec![1, 3, input_size as i64, input_size as i64])?;
    let (mask_width, mask_height) = output_size(&output.shape, input_size, input_size);
    let mask_rgba = apply_alpha_mask(
        &resize_rgba(&resized, input_size, input_size, mask_width, mask_height)?,
        &output.data,
        mask_width,
        mask_height,
    )?;
    Ok(NativeOnnxImageOutput {
        rgba: resize_rgba(&mask_rgba, mask_width, mask_height, width, height)?,
        width,
        height,
        data: None,
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn run_depth(
    session: &mut OnnxSession,
    rgba: &[u8],
    width: u32,
    height: u32,
) -> Result<NativeOnnxImageOutput, String> {
    const INPUT: u32 = 256;
    let resized = resize_rgba(rgba, width, height, INPUT, INPUT)?;
    let tensor = midas_preprocess(&resized, INPUT, INPUT)?;
    let output = session.run_f32(tensor, vec![1, 3, INPUT as i64, INPUT as i64])?;
    let (depth_width, depth_height) = output_size(&output.shape, INPUT, INPUT);
    let count = (depth_width * depth_height) as usize;
    if output.data.len() < count {
        return Err("depth output tensor is smaller than its declared dimensions".to_owned());
    }
    let depth = &output.data[..count];
    let minimum = depth.iter().copied().fold(f32::INFINITY, f32::min);
    let maximum = depth.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let range = (maximum - minimum).max(f32::EPSILON);
    let mut depth_rgba = vec![0; count * 4];
    for (index, value) in depth.iter().copied().enumerate() {
        let gray = to_byte((value - minimum) / range);
        depth_rgba[index * 4..index * 4 + 4].copy_from_slice(&[gray, gray, gray, 255]);
    }
    Ok(NativeOnnxImageOutput {
        rgba: resize_rgba(&depth_rgba, depth_width, depth_height, width, height)?,
        width,
        height,
        data: None,
    })
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_arguments)]
fn run_detection(
    session: &mut OnnxSession,
    rgba: &[u8],
    width: u32,
    height: u32,
    target_size: u32,
    score_threshold: f32,
    iou_threshold: f32,
) -> Result<NativeOnnxImageOutput, String> {
    let input = letterbox_preprocess(rgba, width, height, target_size)?;
    let output = session.run_f32(
        input.tensor,
        input
            .shape
            .iter()
            .map(|dimension| *dimension as i64)
            .collect(),
    )?;
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
        draw_box(
            &mut overlay,
            width,
            height,
            detection.bbox,
            [255, 59, 48, 255],
        );
    }
    Ok(NativeOnnxImageOutput {
        rgba: overlay,
        width,
        height,
        data: Some(json!(detections)),
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn run_segmentation(
    session: &mut OnnxSession,
    rgba: &[u8],
    width: u32,
    height: u32,
    target_size: u32,
) -> Result<NativeOnnxImageOutput, String> {
    let input = letterbox_preprocess(rgba, width, height, target_size)?;
    let output = session.run_f32(
        input.tensor,
        input
            .shape
            .iter()
            .map(|dimension| *dimension as i64)
            .collect(),
    )?;
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
    Ok(NativeOnnxImageOutput {
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
fn tensor_to_rgba(data: &[f32], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let pixels = (width * height) as usize;
    if data.len() >= pixels * 3 {
        return rgb_output_to_rgba(data, width, height);
    }
    if data.len() < pixels {
        return Err("ONNX output tensor does not contain an image".to_owned());
    }
    let mut rgba = vec![0; pixels * 4];
    for index in 0..pixels {
        let value = to_byte(data[index]);
        rgba[index * 4..index * 4 + 4].copy_from_slice(&[value, value, value, 255]);
    }
    Ok(rgba)
}

#[cfg(not(target_arch = "wasm32"))]
fn output_size(shape: &[i64], fallback_width: u32, fallback_height: u32) -> (u32, u32) {
    if shape.len() >= 2 {
        let height = shape[shape.len() - 2];
        let width = shape[shape.len() - 1];
        if width > 0 && height > 0 {
            return (width as u32, height as u32);
        }
    }
    (fallback_width, fallback_height)
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
