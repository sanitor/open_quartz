use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::runtime::{AsyncCompletionEnvelope, ContentStamp, FrameStamp, OutputKey, OutputPayload};
use open_quartz_schema::DataType;

use super::{
    apply_alpha_mask, detect_postprocess, letterbox_preprocess, midas_preprocess,
    rgb_output_to_rgba, rgba_to_chw, segment_postprocess, OnnxTask, SegmentationResult,
    CITYSCAPES_CLASSES, COCO_CLASSES,
};

const INITIAL_TILE: u32 = 64;
const MIN_TILE: u32 = 16;
const TILE_PAD: u32 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OnnxModelFamily {
    SuperResolutionRgb,
    SuperResolutionYcbcr,
    BackgroundRemoval,
    DepthEstimation,
    Detection,
    Segmentation,
    GenericRgb,
    GenericYcbcr,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TensorDescriptor {
    pub dtype: String,
    pub layout: String,
    pub shape: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThresholdDescriptor {
    pub score: f32,
    pub iou: f32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TileDescriptor {
    pub tile_x: u32,
    pub tile_y: u32,
    pub tile_width: u32,
    pub tile_height: u32,
    pub patch_x: u32,
    pub patch_y: u32,
    pub patch_width: u32,
    pub patch_height: u32,
    pub pad_left: u32,
    pub pad_top: u32,
    pub pad_right: u32,
    pub pad_bottom: u32,
    pub input: TensorDescriptor,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "kebab-case")]
pub enum OnnxExecutionPlan {
    Single {
        input: TensorDescriptor,
        output: TensorDescriptor,
    },
    Tiled {
        output_width: u32,
        output_height: u32,
        scale: u32,
        channels: usize,
        tile_size: u32,
        min_tile_size: u32,
        tile_pad: u32,
        fixed_size: Option<u32>,
        tiles: Vec<TileDescriptor>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserOnnxTaskPlan {
    pub model_id: String,
    pub task: OnnxTask,
    pub family: OnnxModelFamily,
    pub source_width: u32,
    pub source_height: u32,
    pub target_size: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thresholds: Option<ThresholdDescriptor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_labels: Option<Vec<String>>,
    pub execution: OnnxExecutionPlan,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserOnnxTaskPlanRequest {
    pub model_id: Option<String>,
    pub task: Option<OnnxTask>,
    pub source_width: u32,
    pub source_height: u32,
    #[serde(default)]
    pub target_size: Option<u32>,
    #[serde(default)]
    pub tile_size: Option<u32>,
    #[serde(default)]
    pub params: Value,
    #[serde(default)]
    pub input_shape: Vec<Value>,
    #[serde(default)]
    pub output_shape: Vec<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserOnnxTensor {
    pub tensor: Vec<f32>,
    pub descriptor: TensorDescriptor,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pad_x: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pad_y: Option<f32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserOnnxTensorRequest {
    pub plan: BrowserOnnxTaskPlan,
    #[serde(default)]
    pub tile_index: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserOnnxDecodedOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rgba: Option<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dst_x: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dst_y: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserOnnxOutputRequest {
    pub plan: BrowserOnnxTaskPlan,
    #[serde(default)]
    pub tile_index: Option<usize>,
    #[serde(default)]
    pub output_shape: Vec<Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserOnnxCompletionPort {
    pub id: String,
    pub data_type: DataType,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserOnnxCompletionRequest {
    pub node_id: String,
    pub graph_revision: u32,
    pub node_generation: u32,
    pub input_stamp: FrameStamp,
    #[serde(default)]
    pub data: Value,
    pub outputs: Vec<BrowserOnnxCompletionPort>,
}

pub fn plan_browser_onnx_task(
    request: BrowserOnnxTaskPlanRequest,
) -> Result<BrowserOnnxTaskPlan, String> {
    if request.source_width == 0 || request.source_height == 0 {
        return Err("source image dimensions must be positive".to_owned());
    }
    let model_id = request.model_id.unwrap_or_else(|| "custom".to_owned());
    let task = request.task.unwrap_or(OnnxTask::Generic);
    let family = model_family(&model_id, task, &request.input_shape);
    let target_size = match family {
        OnnxModelFamily::Detection | OnnxModelFamily::Segmentation => {
            Some(request.target_size.unwrap_or(640).max(1))
        }
        _ => request.target_size,
    };
    let thresholds = (family == OnnxModelFamily::Detection).then(|| ThresholdDescriptor {
        score: number_param(&request.params, "scoreThreshold", 0.25),
        iou: number_param(&request.params, "iouThreshold", 0.45),
    });
    let execution = match family {
        OnnxModelFamily::Detection | OnnxModelFamily::Segmentation => {
            let target = target_size.unwrap_or(640);
            OnnxExecutionPlan::Single {
                input: tensor_descriptor(3, target, target),
                output: TensorDescriptor {
                    dtype: "float32".to_owned(),
                    layout: "model-defined".to_owned(),
                    shape: Vec::new(),
                },
            }
        }
        OnnxModelFamily::GenericRgb | OnnxModelFamily::GenericYcbcr => {
            let channels = if family == OnnxModelFamily::GenericYcbcr { 1 } else { 3 };
            let fixed_size = fixed_square(&request.input_shape);
            let scale = model_scale(&request.input_shape, &request.output_shape).max(1);
            tiled_execution(
                request.source_width,
                request.source_height,
                scale,
                channels,
                fixed_size,
                request.tile_size,
            )
        }
        OnnxModelFamily::SuperResolutionRgb => tiled_execution(
            request.source_width,
            request.source_height,
            4,
            3,
            None,
            request.tile_size,
        ),
        OnnxModelFamily::SuperResolutionYcbcr => tiled_execution(
            request.source_width,
            request.source_height,
            3,
            1,
            Some(224),
            request.tile_size,
        ),
        OnnxModelFamily::BackgroundRemoval => {
            let fixed = if model_id == "u2netp" { 320 } else { 512 };
            tiled_execution(
                request.source_width,
                request.source_height,
                1,
                3,
                Some(fixed),
                request.tile_size,
            )
        }
        OnnxModelFamily::DepthEstimation => tiled_execution(
            request.source_width,
            request.source_height,
            1,
            3,
            Some(256),
            request.tile_size,
        ),
    };
    Ok(BrowserOnnxTaskPlan {
        model_id,
        task,
        family,
        source_width: request.source_width,
        source_height: request.source_height,
        target_size,
        thresholds,
        class_labels: match family {
            OnnxModelFamily::Detection => Some(COCO_CLASSES.iter().map(|label| (*label).to_owned()).collect()),
            OnnxModelFamily::Segmentation => Some(CITYSCAPES_CLASSES.iter().map(|label| (*label).to_owned()).collect()),
            _ => None,
        },
        execution,
    })
}

pub fn encode_browser_onnx_input(
    rgba: &[u8],
    request: BrowserOnnxTensorRequest,
) -> Result<BrowserOnnxTensor, String> {
    match &request.plan.execution {
        OnnxExecutionPlan::Single { input, .. } => encode_single(rgba, &request.plan, input),
        OnnxExecutionPlan::Tiled { tiles, .. } => {
            let index = request
                .tile_index
                .ok_or_else(|| "tile_index is required for tiled ONNX input".to_owned())?;
            let tile = tiles
                .get(index)
                .ok_or_else(|| format!("tile_index {index} is outside the tile plan"))?;
            encode_tile(rgba, &request.plan, tile)
        }
    }
}

pub fn decode_browser_onnx_output(
    source_rgba: &[u8],
    raw: &[f32],
    request: BrowserOnnxOutputRequest,
) -> Result<BrowserOnnxDecodedOutput, String> {
    match &request.plan.execution {
        OnnxExecutionPlan::Single { .. } => decode_single(source_rgba, raw, &request),
        OnnxExecutionPlan::Tiled { tiles, .. } => {
            let index = request
                .tile_index
                .ok_or_else(|| "tile_index is required for tiled ONNX output".to_owned())?;
            let tile = tiles
                .get(index)
                .ok_or_else(|| format!("tile_index {index} is outside the tile plan"))?;
            decode_tile(source_rgba, raw, &request.plan, tile)
        }
    }
}

pub fn build_browser_onnx_completion(
    request: BrowserOnnxCompletionRequest,
) -> AsyncCompletionEnvelope {
    let content_stamp = ContentStamp {
        epoch: request.input_stamp.epoch,
        timeline_ns: request.input_stamp.timeline_ns,
        media_pts_ns: None,
    };
    let outputs = request
        .outputs
        .into_iter()
        .map(|port| {
            let payload = if is_sampler(port.data_type) {
                OutputPayload::Resource {
                    handle: request.node_generation as u64,
                }
            } else {
                OutputPayload::Json(request.data.clone())
            };
            (OutputKey::new(&request.node_id, port.id), payload)
        })
        .collect();
    AsyncCompletionEnvelope {
        node_id: request.node_id,
        graph_revision: request.graph_revision,
        node_generation: request.node_generation,
        input_stamp: request.input_stamp,
        content_stamp,
        outputs,
    }
}

fn model_family(model_id: &str, task: OnnxTask, input_shape: &[Value]) -> OnnxModelFamily {
    match task {
        OnnxTask::SuperResolution if model_id == "super-resolution-3x" => {
            OnnxModelFamily::SuperResolutionYcbcr
        }
        OnnxTask::SuperResolution => OnnxModelFamily::SuperResolutionRgb,
        OnnxTask::BackgroundRemoval => OnnxModelFamily::BackgroundRemoval,
        OnnxTask::DepthEstimation => OnnxModelFamily::DepthEstimation,
        OnnxTask::Detection => OnnxModelFamily::Detection,
        OnnxTask::Segmentation => OnnxModelFamily::Segmentation,
        OnnxTask::StyleTransfer | OnnxTask::Denoising | OnnxTask::Generic => {
            if shape_dim(input_shape, 1).unwrap_or(3) == 1 {
                OnnxModelFamily::GenericYcbcr
            } else {
                OnnxModelFamily::GenericRgb
            }
        }
    }
}

fn encode_single(
    rgba: &[u8],
    plan: &BrowserOnnxTaskPlan,
    descriptor: &TensorDescriptor,
) -> Result<BrowserOnnxTensor, String> {
    if matches!(
        plan.family,
        OnnxModelFamily::Detection | OnnxModelFamily::Segmentation
    ) {
        let tensor = letterbox_preprocess(
            rgba,
            plan.source_width,
            plan.source_height,
            plan.target_size.unwrap_or(640),
        )?;
        return Ok(BrowserOnnxTensor {
            tensor: tensor.tensor,
            descriptor: TensorDescriptor {
                dtype: "float32".to_owned(),
                layout: "nchw-rgb".to_owned(),
                shape: tensor.shape.to_vec(),
            },
            scale: Some(tensor.scale),
            pad_x: Some(tensor.pad_x),
            pad_y: Some(tensor.pad_y),
        });
    }
    Ok(BrowserOnnxTensor {
        tensor: rgba_to_chw(rgba, plan.source_width, plan.source_height)?,
        descriptor: descriptor.clone(),
        scale: None,
        pad_x: None,
        pad_y: None,
    })
}

fn decode_single(
    _source_rgba: &[u8],
    raw: &[f32],
    request: &BrowserOnnxOutputRequest,
) -> Result<BrowserOnnxDecodedOutput, String> {
    let scale = request_scale(&request.plan)?;
    let pad_x = request_pad_x(&request.plan)?;
    let pad_y = request_pad_y(&request.plan)?;
    match request.plan.family {
        OnnxModelFamily::Detection => {
            let thresholds = request
                .plan
                .thresholds
                .clone()
                .unwrap_or(ThresholdDescriptor { score: 0.25, iou: 0.45 });
            let detections = detect_postprocess(
                raw,
                request.plan.source_width,
                request.plan.source_height,
                scale,
                pad_x,
                pad_y,
                thresholds.score,
                thresholds.iou,
            );
            Ok(BrowserOnnxDecodedOutput {
                rgba: None,
                width: request.plan.source_width,
                height: request.plan.source_height,
                dst_x: None,
                dst_y: None,
                data: Some(
                    serde_json::to_value(detections)
                        .map_err(|error| format!("Cannot serialize detections: {error}"))?,
                ),
            })
        }
        OnnxModelFamily::Segmentation => {
            let segmentation = segment_postprocess(
                raw,
                request.plan.source_width as usize,
                request.plan.source_height as usize,
                scale,
                pad_x,
                pad_y,
            )?;
            Ok(BrowserOnnxDecodedOutput {
                rgba: Some(segmentation.mask_rgba),
                width: segmentation.mask_width as u32,
                height: segmentation.mask_height as u32,
                dst_x: None,
                dst_y: None,
                data: None,
            })
        }
        _ => {
            let (width, height) = output_size(
                &request.output_shape,
                request.plan.source_width,
                request.plan.source_height,
            );
            Ok(BrowserOnnxDecodedOutput {
                rgba: Some(rgb_output_to_rgba(raw, width, height)?),
                width,
                height,
                dst_x: None,
                dst_y: None,
                data: None,
            })
        }
    }
}

fn request_scale(plan: &BrowserOnnxTaskPlan) -> Result<f32, String> {
    let target = plan.target_size.unwrap_or(640) as f32;
    Ok((target / plan.source_width as f32).min(target / plan.source_height as f32))
}

fn request_pad_x(plan: &BrowserOnnxTaskPlan) -> Result<f32, String> {
    let target = plan.target_size.unwrap_or(640);
    let scale = request_scale(plan)?;
    let resized_width = (plan.source_width as f32 * scale).round() as u32;
    Ok((target - resized_width) as f32 / 2.0)
}

fn request_pad_y(plan: &BrowserOnnxTaskPlan) -> Result<f32, String> {
    let target = plan.target_size.unwrap_or(640);
    let scale = request_scale(plan)?;
    let resized_height = (plan.source_height as f32 * scale).round() as u32;
    Ok((target - resized_height) as f32 / 2.0)
}

fn encode_tile(
    rgba: &[u8],
    plan: &BrowserOnnxTaskPlan,
    tile: &TileDescriptor,
) -> Result<BrowserOnnxTensor, String> {
    validate_rgba(rgba, plan.source_width, plan.source_height)?;
    let tensor = match plan.family {
        OnnxModelFamily::SuperResolutionYcbcr | OnnxModelFamily::GenericYcbcr => {
            encode_ycbcr_tile(rgba, plan.source_width, tile)?
        }
        OnnxModelFamily::DepthEstimation => encode_depth_tile(rgba, plan.source_width, tile)?,
        _ => encode_rgb_tile(rgba, plan.source_width, tile, tile.input.shape[2], tile.input.shape[3])?,
    };
    Ok(BrowserOnnxTensor {
        tensor,
        descriptor: tile.input.clone(),
        scale: None,
        pad_x: None,
        pad_y: None,
    })
}

fn decode_tile(
    source_rgba: &[u8],
    raw: &[f32],
    plan: &BrowserOnnxTaskPlan,
    tile: &TileDescriptor,
) -> Result<BrowserOnnxDecodedOutput, String> {
    validate_rgba(source_rgba, plan.source_width, plan.source_height)?;
    let (scale, model_width, model_height) = match &plan.execution {
        OnnxExecutionPlan::Tiled {
            scale,
            fixed_size,
            ..
        } => {
            let model_width = fixed_size.unwrap_or(tile.patch_width);
            let model_height = fixed_size.unwrap_or(tile.patch_height);
            (*scale, model_width, model_height)
        }
        _ => return Err("decode_tile requires a tiled plan".to_owned()),
    };
    let rgba = match plan.family {
        OnnxModelFamily::SuperResolutionYcbcr | OnnxModelFamily::GenericYcbcr => {
            decode_ycbcr_tile(source_rgba, raw, plan.source_width, plan.source_height, tile, scale, model_width)?
        }
        OnnxModelFamily::BackgroundRemoval => {
            decode_alpha_tile(source_rgba, raw, plan.source_width, tile, model_width)?
        }
        OnnxModelFamily::DepthEstimation => decode_depth_tile(raw, tile, model_width)?,
        _ => decode_rgb_tile(raw, tile, scale, model_width, model_height)?,
    };
    Ok(BrowserOnnxDecodedOutput {
        width: tile.tile_width * scale,
        height: tile.tile_height * scale,
        dst_x: Some(tile.tile_x * scale),
        dst_y: Some(tile.tile_y * scale),
        rgba: Some(rgba),
        data: None,
    })
}

fn tiled_execution(
    source_width: u32,
    source_height: u32,
    scale: u32,
    channels: usize,
    fixed_size: Option<u32>,
    requested_tile_size: Option<u32>,
) -> OnnxExecutionPlan {
    let mut tile_size = requested_tile_size.unwrap_or(INITIAL_TILE).max(1);
    if fixed_size.is_some() {
        tile_size = fixed_size.unwrap().saturating_sub(2 * TILE_PAD).max(1);
    }
    let mut tiles = Vec::new();
    let mut tile_y = 0;
    while tile_y < source_height {
        let mut tile_x = 0;
        while tile_x < source_width {
            let tile_width = tile_size.min(source_width - tile_x);
            let tile_height = tile_size.min(source_height - tile_y);
            let pad_left = TILE_PAD.min(tile_x);
            let pad_top = TILE_PAD.min(tile_y);
            let pad_right = TILE_PAD.min(source_width - tile_x - tile_width);
            let pad_bottom = TILE_PAD.min(source_height - tile_y - tile_height);
            let patch_x = tile_x - pad_left;
            let patch_y = tile_y - pad_top;
            let patch_width = tile_width + pad_left + pad_right;
            let patch_height = tile_height + pad_top + pad_bottom;
            let model_width = fixed_size.unwrap_or(patch_width);
            let model_height = fixed_size.unwrap_or(patch_height);
            tiles.push(TileDescriptor {
                tile_x,
                tile_y,
                tile_width,
                tile_height,
                patch_x,
                patch_y,
                patch_width,
                patch_height,
                pad_left,
                pad_top,
                pad_right,
                pad_bottom,
                input: tensor_descriptor(channels, model_height, model_width),
            });
            tile_x += tile_size;
        }
        tile_y += tile_size;
    }
    OnnxExecutionPlan::Tiled {
        output_width: source_width * scale,
        output_height: source_height * scale,
        scale,
        channels,
        tile_size,
        min_tile_size: MIN_TILE,
        tile_pad: TILE_PAD,
        fixed_size,
        tiles,
    }
}

fn tensor_descriptor(channels: usize, height: u32, width: u32) -> TensorDescriptor {
    TensorDescriptor {
        dtype: "float32".to_owned(),
        layout: "nchw-rgb".to_owned(),
        shape: vec![1, channels, height as usize, width as usize],
    }
}

fn encode_rgb_tile(
    rgba: &[u8],
    source_width: u32,
    tile: &TileDescriptor,
    model_height: usize,
    model_width: usize,
) -> Result<Vec<f32>, String> {
    let pixels = model_width * model_height;
    let mut tensor = vec![0.0; pixels * 3];
    for row in 0..tile.patch_height as usize {
        for col in 0..tile.patch_width as usize {
            let source = ((tile.patch_y as usize + row) * source_width as usize
                + tile.patch_x as usize
                + col)
                * 4;
            let destination = row * model_width + col;
            tensor[destination] = rgba[source] as f32 / 255.0;
            tensor[pixels + destination] = rgba[source + 1] as f32 / 255.0;
            tensor[pixels * 2 + destination] = rgba[source + 2] as f32 / 255.0;
        }
    }
    Ok(tensor)
}

fn encode_ycbcr_tile(
    rgba: &[u8],
    source_width: u32,
    tile: &TileDescriptor,
) -> Result<Vec<f32>, String> {
    let model_width = tile.input.shape[3];
    let model_height = tile.input.shape[2];
    let mut tensor = vec![0.0; model_width * model_height];
    for row in 0..tile.patch_height as usize {
        for col in 0..tile.patch_width as usize {
            let source = ((tile.patch_y as usize + row) * source_width as usize
                + tile.patch_x as usize
                + col)
                * 4;
            let red = rgba[source] as f32 / 255.0;
            let green = rgba[source + 1] as f32 / 255.0;
            let blue = rgba[source + 2] as f32 / 255.0;
            tensor[row * model_width + col] = 0.299 * red + 0.587 * green + 0.114 * blue;
        }
    }
    Ok(tensor)
}

fn encode_depth_tile(
    rgba: &[u8],
    source_width: u32,
    tile: &TileDescriptor,
) -> Result<Vec<f32>, String> {
    let patch = extract_patch_rgba(rgba, source_width, tile)?;
    midas_preprocess(&patch, tile.input.shape[3] as u32, tile.input.shape[2] as u32)
}

fn decode_rgb_tile(
    raw: &[f32],
    tile: &TileDescriptor,
    scale: u32,
    model_width: u32,
    model_height: u32,
) -> Result<Vec<u8>, String> {
    let out_patch_width = model_width * scale;
    let out_patch_height = model_height * scale;
    let patch = rgb_output_to_rgba(raw, out_patch_width, out_patch_height)?;
    crop_rgba(
        &patch,
        out_patch_width,
        tile.pad_left * scale,
        tile.pad_top * scale,
        tile.tile_width * scale,
        tile.tile_height * scale,
    )
}

fn decode_ycbcr_tile(
    source_rgba: &[u8],
    raw: &[f32],
    source_width: u32,
    source_height: u32,
    tile: &TileDescriptor,
    scale: u32,
    model_width: u32,
) -> Result<Vec<u8>, String> {
    let crop_width = tile.tile_width * scale;
    let crop_height = tile.tile_height * scale;
    let out_patch_width = model_width * scale;
    if raw.len() < (out_patch_width * (tile.input.shape[2] as u32 * scale)) as usize {
        return Err("YCbCr output tensor is smaller than its declared dimensions".to_owned());
    }
    let mut output = vec![0; (crop_width * crop_height * 4) as usize];
    for row in 0..crop_height {
        for col in 0..crop_width {
            let source_index =
                ((tile.pad_top * scale + row) * out_patch_width + tile.pad_left * scale + col)
                    as usize;
            let orig_x = (tile.tile_x + col / scale).min(source_width - 1);
            let orig_y = (tile.tile_y + row / scale).min(source_height - 1);
            let chroma = (orig_y * source_width + orig_x) as usize * 4;
            let red = source_rgba[chroma] as f32 / 255.0;
            let green = source_rgba[chroma + 1] as f32 / 255.0;
            let blue = source_rgba[chroma + 2] as f32 / 255.0;
            let cb = -0.169 * red - 0.331 * green + 0.500 * blue;
            let cr = 0.500 * red - 0.419 * green - 0.081 * blue;
            let y = raw[source_index];
            let destination = (row * crop_width + col) as usize * 4;
            output[destination] = float_to_byte(y + 1.402 * cr);
            output[destination + 1] = float_to_byte(y - 0.344 * cb - 0.714 * cr);
            output[destination + 2] = float_to_byte(y + 1.772 * cb);
            output[destination + 3] = 255;
        }
    }
    Ok(output)
}

fn decode_alpha_tile(
    source_rgba: &[u8],
    raw: &[f32],
    source_width: u32,
    tile: &TileDescriptor,
    model_width: u32,
) -> Result<Vec<u8>, String> {
    let patch = extract_patch_rgba(source_rgba, source_width, tile)?;
    let masked = apply_alpha_mask(&patch, raw, model_width, tile.input.shape[2] as u32)?;
    crop_rgba(
        &masked,
        model_width,
        tile.pad_left,
        tile.pad_top,
        tile.tile_width,
        tile.tile_height,
    )
}

fn decode_depth_tile(
    raw: &[f32],
    tile: &TileDescriptor,
    model_width: u32,
) -> Result<Vec<u8>, String> {
    let model_height = tile.input.shape[2] as u32;
    if raw.len() < (model_width * model_height) as usize {
        return Err("depth output tensor is smaller than its declared dimensions".to_owned());
    }
    let crop_width = tile.tile_width;
    let crop_height = tile.tile_height;
    let mut minimum = f32::INFINITY;
    let mut maximum = f32::NEG_INFINITY;
    for row in 0..crop_height {
        for col in 0..crop_width {
            let value = raw[((tile.pad_top + row) * model_width + tile.pad_left + col) as usize];
            minimum = minimum.min(value);
            maximum = maximum.max(value);
        }
    }
    let range = (maximum - minimum).max(f32::EPSILON);
    let mut output = vec![0; (crop_width * crop_height * 4) as usize];
    for row in 0..crop_height {
        for col in 0..crop_width {
            let value = raw[((tile.pad_top + row) * model_width + tile.pad_left + col) as usize];
            let gray = float_to_byte((value - minimum) / range);
            let destination = (row * crop_width + col) as usize * 4;
            output[destination..destination + 4].copy_from_slice(&[gray, gray, gray, 255]);
        }
    }
    Ok(output)
}

fn extract_patch_rgba(
    rgba: &[u8],
    source_width: u32,
    tile: &TileDescriptor,
) -> Result<Vec<u8>, String> {
    let model_width = tile.input.shape[3] as u32;
    let model_height = tile.input.shape[2] as u32;
    let mut patch = vec![0; (model_width * model_height * 4) as usize];
    for row in 0..tile.patch_height {
        for col in 0..tile.patch_width {
            let source =
                ((tile.patch_y + row) * source_width + tile.patch_x + col) as usize * 4;
            let destination = (row * model_width + col) as usize * 4;
            patch[destination..destination + 4].copy_from_slice(&rgba[source..source + 4]);
        }
    }
    Ok(patch)
}

fn crop_rgba(
    rgba: &[u8],
    source_width: u32,
    crop_x: u32,
    crop_y: u32,
    crop_width: u32,
    crop_height: u32,
) -> Result<Vec<u8>, String> {
    let mut output = vec![0; (crop_width * crop_height * 4) as usize];
    for row in 0..crop_height {
        let source = ((crop_y + row) * source_width + crop_x) as usize * 4;
        let destination = (row * crop_width) as usize * 4;
        let len = crop_width as usize * 4;
        if source + len > rgba.len() {
            return Err("RGBA crop exceeds source pixels".to_owned());
        }
        output[destination..destination + len].copy_from_slice(&rgba[source..source + len]);
    }
    Ok(output)
}

fn number_param(params: &Value, key: &str, fallback: f32) -> f32 {
    params
        .get(key)
        .and_then(Value::as_f64)
        .map(|value| value as f32)
        .unwrap_or(fallback)
}

fn fixed_square(shape: &[Value]) -> Option<u32> {
    let height = shape_dim(shape, 2)?;
    let width = shape_dim(shape, 3)?;
    (height == width && height > 0).then_some(height as u32)
}

fn model_scale(input_shape: &[Value], output_shape: &[Value]) -> u32 {
    let input = shape_dim(input_shape, 2).unwrap_or(1);
    let output = shape_dim(output_shape, 2).unwrap_or(input);
    (output / input.max(1)).max(1) as u32
}

fn output_size(shape: &[Value], fallback_width: u32, fallback_height: u32) -> (u32, u32) {
    let height = shape_dim(shape, 2).unwrap_or(fallback_height as usize) as u32;
    let width = shape_dim(shape, 3).unwrap_or(fallback_width as usize) as u32;
    (width, height)
}

fn shape_dim(shape: &[Value], index: usize) -> Option<usize> {
    let value = shape.get(index)?;
    match value {
        Value::Number(number) => number.as_u64().map(|value| value as usize),
        _ => None,
    }
}

fn validate_rgba(rgba: &[u8], width: u32, height: u32) -> Result<(), String> {
    let expected = width as usize * height as usize * 4;
    if width == 0 || height == 0 || rgba.len() != expected {
        Err(format!(
            "RGBA byte length {} does not match {width}x{height} image",
            rgba.len()
        ))
    } else {
        Ok(())
    }
}

fn is_sampler(data_type: DataType) -> bool {
    matches!(data_type, DataType::Sampler2d | DataType::SamplerCube)
}

fn float_to_byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[allow(dead_code)]
fn _assert_segmentation_result_is_serializable(_: &SegmentationResult) {}
