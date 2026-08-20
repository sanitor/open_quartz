use serde::{Deserialize, Serialize};
use serde_json::json;

use open_quartz_schema::{DataType, OnnxTask, Port, PortDirection};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSnapshot {
    pub math_categories: Vec<MathCategory>,
    pub math_ops: Vec<MathDescriptor>,
    pub onnx_categories: Vec<String>,
    pub onnx_models: Vec<OnnxModelDescriptor>,
    pub shader_groups: Vec<ShaderGroupDescriptor>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MathCategory {
    pub category: String,
    pub ops: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MathDescriptor {
    pub id: String,
    pub label: String,
    pub category: String,
    pub input_count: u8,
    pub formula: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamDescriptor {
    #[serde(rename = "type")]
    pub value_type: String,
    pub default: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnnxIoDescriptor {
    pub inputs: Vec<Port>,
    pub outputs: Vec<Port>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnnxModelDescriptor {
    pub id: String,
    pub label: String,
    pub task: OnnxTask,
    pub category: String,
    pub download_url: String,
    pub file_size: u64,
    pub sha256: String,
    #[serde(rename = "expectedIO")]
    pub expected_io: OnnxIoDescriptor,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_params: Option<std::collections::BTreeMap<String, ParamDescriptor>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShaderGroupDescriptor {
    pub category: String,
    pub items: Vec<ShaderTemplateDescriptor>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShaderTemplateDescriptor {
    pub id: String,
    pub label: String,
    pub inputs: Vec<ShaderPortDescriptor>,
    pub outputs: Vec<ShaderPortDescriptor>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShaderPortDescriptor {
    pub label: String,
    pub data_type: DataType,
}

pub fn catalog_snapshot() -> CatalogSnapshot {
    let onnx_models = onnx_models();
    let mut onnx_categories = onnx_models
        .iter()
        .map(|entry| entry.category.clone())
        .collect::<Vec<_>>();
    onnx_categories.sort();
    onnx_categories.dedup();
    CatalogSnapshot {
        math_categories: math_categories(),
        math_ops: math_ops(),
        onnx_categories,
        onnx_models,
        shader_groups: shader_groups(),
    }
}

pub fn math_categories() -> Vec<MathCategory> {
    [
        ("Arithmetic", &["add", "subtract", "multiply", "divide", "negate", "modulo"][..]),
        ("Range", &["min", "max", "clamp", "saturate", "step", "smoothstep", "abs", "sign"][..]),
        ("Trigonometry", &["sin", "cos", "tan", "asin", "acos", "atan"][..]),
        ("Exponential", &["pow", "sqrt", "exp", "log"][..]),
        ("Interpolation", &["mix"][..]),
        ("Rounding", &["floor", "ceil", "round", "fract"][..]),
    ]
    .into_iter()
    .map(|(category, ops)| MathCategory {
        category: category.to_owned(),
        ops: ops.iter().map(|op| (*op).to_owned()).collect(),
    })
    .collect()
}

pub fn math_ops() -> Vec<MathDescriptor> {
    [
        ("add", "Add", "Arithmetic", 2, "a + b"),
        ("subtract", "Subtract", "Arithmetic", 2, "a - b"),
        ("multiply", "Multiply", "Arithmetic", 2, "a * b"),
        ("divide", "Divide", "Arithmetic", 2, "b == 0 ? 0 : a / b"),
        ("negate", "Negate", "Arithmetic", 1, "-a"),
        ("modulo", "Modulo", "Arithmetic", 2, "b == 0 ? 0 : a % b"),
        ("min", "Min", "Range", 2, "min(a, b)"),
        ("max", "Max", "Range", 2, "max(a, b)"),
        ("clamp", "Clamp", "Range", 3, "min(max(a, b), c)"),
        ("saturate", "Saturate", "Range", 1, "clamp(a, 0, 1)"),
        ("step", "Step", "Range", 2, "b >= a ? 1 : 0"),
        ("smoothstep", "Smoothstep", "Range", 3, "t*t*(3-2*t), t=clamp((c-a)/(b-a),0,1)"),
        ("abs", "Abs", "Range", 1, "abs(a)"),
        ("sign", "Sign", "Range", 1, "a == 0 ? a : sign(a)"),
        ("sin", "Sin", "Trigonometry", 1, "sin(a)"),
        ("cos", "Cos", "Trigonometry", 1, "cos(a)"),
        ("tan", "Tan", "Trigonometry", 1, "tan(a)"),
        ("asin", "Asin", "Trigonometry", 1, "asin(a)"),
        ("acos", "Acos", "Trigonometry", 1, "acos(a)"),
        ("atan", "Atan", "Trigonometry", 1, "atan(a)"),
        ("pow", "Pow", "Exponential", 2, "pow(a, b)"),
        ("sqrt", "Sqrt", "Exponential", 1, "sqrt(a)"),
        ("exp", "Exp", "Exponential", 1, "exp(a)"),
        ("log", "Log", "Exponential", 1, "ln(a)"),
        ("mix", "Mix", "Interpolation", 3, "a * (1 - c) + b * c"),
        ("floor", "Floor", "Rounding", 1, "floor(a)"),
        ("ceil", "Ceil", "Rounding", 1, "ceil(a)"),
        ("round", "Round", "Rounding", 1, "a.fract() == -0.5 ? ceil(a) : floor(a + 0.5)"),
        ("fract", "Fract", "Rounding", 1, "a - floor(a)"),
    ]
    .into_iter()
    .map(|(id, label, category, input_count, formula)| MathDescriptor {
        id: id.to_owned(),
        label: label.to_owned(),
        category: category.to_owned(),
        input_count,
        formula: formula.to_owned(),
    })
    .collect()
}

pub fn evaluate_math(operation: &str, input: &[f64]) -> f64 {
    let a = input.first().copied().unwrap_or(0.0);
    let b = input.get(1).copied().unwrap_or(0.0);
    let c = input.get(2).copied().unwrap_or(0.0);
    match operation {
        "add" => a + b,
        "subtract" => a - b,
        "multiply" => a * b,
        "divide" => safe_divide(a, b),
        "negate" => -a,
        "modulo" => safe_modulo(a, b),
        "min" => a.min(b),
        "max" => a.max(b),
        "clamp" => a.max(b).min(c),
        "saturate" => a.clamp(0.0, 1.0),
        "step" => f64::from(b >= a),
        "smoothstep" => {
            let t = safe_divide(c - a, b - a).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        }
        "abs" => a.abs(),
        "sign" => {
            if a == 0.0 {
                a
            } else {
                a.signum()
            }
        }
        "sin" => a.sin(),
        "cos" => a.cos(),
        "tan" => a.tan(),
        "asin" => a.asin(),
        "acos" => a.acos(),
        "atan" => a.atan(),
        "pow" => a.powf(b),
        "sqrt" => a.sqrt(),
        "exp" => a.exp(),
        "log" => a.ln(),
        "mix" => a * (1.0 - c) + b * c,
        "floor" => a.floor(),
        "ceil" => a.ceil(),
        "round" => {
            if a.fract() == -0.5 {
                a.ceil()
            } else {
                (a + 0.5).floor()
            }
        }
        "fract" => a - a.floor(),
        _ => 0.0,
    }
}

pub fn onnx_models() -> Vec<OnnxModelDescriptor> {
    vec![
        onnx_model::<2>(
            "yolov8n",
            "YOLOv8n Detector",
            OnnxTask::Detection,
            "Detection",
            "https://raw.githubusercontent.com/caozisheng/rimeflow-yolov8n/main/models/yolov8n.onnx",
            12_851_098,
            "",
            vec![port("onnx_in_image", "image", DataType::Sampler2d, PortDirection::Input)],
            vec![
                port("onnx_out_detections", "detections", DataType::Roi, PortDirection::Output),
                port("onnx_out_overlay", "overlay", DataType::Sampler2d, PortDirection::Output),
            ],
            Some([
                ("scoreThreshold", float_param(0.25, 0.0, 1.0, 0.05, "Score Threshold")),
                ("iouThreshold", float_param(0.45, 0.0, 1.0, 0.05, "IoU Threshold")),
            ]),
        ),
        onnx_model::<0>(
            "super-resolution-3x",
            "Super Resolution 3×",
            OnnxTask::SuperResolution,
            "Super-Resolution",
            "https://media.githubusercontent.com/media/onnx/models/main/validated/vision/super_resolution/sub_pixel_cnn_2016/model/super-resolution-10.onnx",
            240_078,
            "",
            vec![port("onnx_in_image", "image", DataType::Sampler2d, PortDirection::Input)],
            vec![port("onnx_out_upscaled", "upscaled", DataType::Sampler2d, PortDirection::Output)],
            None,
        ),
        onnx_model::<0>(
            "realesrgan-x4",
            "Real-ESRGAN 4×",
            OnnxTask::SuperResolution,
            "Super-Resolution",
            "https://huggingface.co/Samo629/real-esrgan-onnx/resolve/main/realesr-general-x4v3.onnx",
            4_866_421,
            "",
            vec![port("onnx_in_image", "image", DataType::Sampler2d, PortDirection::Input)],
            vec![port("onnx_out_upscaled", "upscaled", DataType::Sampler2d, PortDirection::Output)],
            None,
        ),
        onnx_model::<0>(
            "u2netp",
            "U²Net-P (Background)",
            OnnxTask::BackgroundRemoval,
            "Background Removal",
            "https://github.com/danielgatis/rembg/releases/download/v0.0.0/u2netp.onnx",
            4_574_861,
            "",
            vec![port("onnx_in_image", "image", DataType::Sampler2d, PortDirection::Input)],
            vec![port("onnx_out_foreground", "foreground", DataType::Sampler2d, PortDirection::Output)],
            None,
        ),
        onnx_model::<0>(
            "modnet",
            "MODNet (Portrait)",
            OnnxTask::BackgroundRemoval,
            "Background Removal",
            "https://huggingface.co/onnx-community/modnet-webnn/resolve/main/onnx/model.onnx",
            25_888_640,
            "",
            vec![port("onnx_in_image", "image", DataType::Sampler2d, PortDirection::Input)],
            vec![port("onnx_out_foreground", "foreground", DataType::Sampler2d, PortDirection::Output)],
            None,
        ),
        onnx_model::<0>(
            "midas-small",
            "MiDaS v2.1 Small (Depth)",
            OnnxTask::DepthEstimation,
            "Depth Estimation",
            "https://huggingface.co/Heliosoph/midas-small-onnx/resolve/main/midas_v21_small_256.onnx",
            66_389_153,
            "",
            vec![port("onnx_in_image", "image", DataType::Sampler2d, PortDirection::Input)],
            vec![port("onnx_out_depth", "depth", DataType::Sampler2d, PortDirection::Output)],
            None,
        ),
        onnx_model::<2>(
            "yolo26n-sem",
            "YOLO26n Semantic Seg",
            OnnxTask::Segmentation,
            "Segmentation",
            "https://github.com/caozisheng/rimeflow-yolo26n-sem/raw/refs/heads/master/models/yolo26n-sem.onnx",
            6_284_385,
            "",
            vec![port("onnx_in_image", "image", DataType::Sampler2d, PortDirection::Input)],
            vec![port("onnx_out_overlay", "overlay", DataType::Sampler2d, PortDirection::Output)],
            None,
        ),
    ]
}

pub fn shader_groups() -> Vec<ShaderGroupDescriptor> {
    vec![
        shader_group("FILTER", &[
            ("Resample", &[("inputImage", DataType::Sampler2d)], &[("fragColor", DataType::Vec4)]),
            ("Sobel Edge Detection", &[("inputImage", DataType::Sampler2d), ("intensity", DataType::Float)], &[("fragColor", DataType::Vec4)]),
            ("Gaussian Blur 3x3", &[("inputImage", DataType::Sampler2d)], &[("fragColor", DataType::Vec4)]),
            ("Box Blur", &[("inputImage", DataType::Sampler2d)], &[("fragColor", DataType::Vec4)]),
            ("Sharpen", &[("inputImage", DataType::Sampler2d), ("strength", DataType::Float)], &[("fragColor", DataType::Vec4)]),
            ("Emboss", &[("inputImage", DataType::Sampler2d)], &[("fragColor", DataType::Vec4)]),
            ("Pixelate", &[("inputImage", DataType::Sampler2d), ("blockSize", DataType::Vec2)], &[("fragColor", DataType::Vec4)]),
        ]),
        shader_group("COLOR", &[
            ("Invert", &[("inputImage", DataType::Sampler2d)], &[("fragColor", DataType::Vec4)]),
            ("Grayscale", &[("inputImage", DataType::Sampler2d)], &[("fragColor", DataType::Vec4)]),
            ("Brightness/Contrast", &[("inputImage", DataType::Sampler2d), ("brightness", DataType::Float), ("contrast", DataType::Float)], &[("fragColor", DataType::Vec4)]),
            ("Hue Rotate", &[("inputImage", DataType::Sampler2d), ("angle", DataType::Float)], &[("fragColor", DataType::Vec4)]),
            ("Threshold", &[("inputImage", DataType::Sampler2d), ("threshold", DataType::Float)], &[("fragColor", DataType::Vec4)]),
            ("Sepia", &[("inputImage", DataType::Sampler2d)], &[("fragColor", DataType::Vec4)]),
            ("Field Color Map", &[("inputImage", DataType::Sampler2d)], &[("fragColor", DataType::Vec4)]),
        ]),
        shader_group("GENERATOR", &[
            ("Solid Color", &[("color", DataType::Vec4)], &[("fragColor", DataType::Vec4)]),
            ("Gradient", &[("colorA", DataType::Vec4), ("colorB", DataType::Vec4)], &[("fragColor", DataType::Vec4)]),
            ("Checkerboard", &[("gridSize", DataType::Vec2), ("color1", DataType::Vec4), ("color2", DataType::Vec4)], &[("fragColor", DataType::Vec4)]),
            ("Noise", &[("scale", DataType::Float)], &[("fragColor", DataType::Vec4)]),
            ("Circle", &[("circle", DataType::Vec4)], &[("fragColor", DataType::Vec4)]),
        ]),
        shader_group("BLEND", &[
            ("Add", &[("inputA", DataType::Sampler2d), ("inputB", DataType::Sampler2d)], &[("fragColor", DataType::Vec4)]),
            ("Multiply", &[("inputA", DataType::Sampler2d), ("inputB", DataType::Sampler2d)], &[("fragColor", DataType::Vec4)]),
            ("Screen", &[("inputA", DataType::Sampler2d), ("inputB", DataType::Sampler2d)], &[("fragColor", DataType::Vec4)]),
            ("Overlay", &[("inputA", DataType::Sampler2d), ("inputB", DataType::Sampler2d)], &[("fragColor", DataType::Vec4)]),
            ("Difference", &[("inputA", DataType::Sampler2d), ("inputB", DataType::Sampler2d)], &[("fragColor", DataType::Vec4)]),
            ("Exclusion", &[("inputA", DataType::Sampler2d), ("inputB", DataType::Sampler2d)], &[("fragColor", DataType::Vec4)]),
            ("Soft Light", &[("inputA", DataType::Sampler2d), ("inputB", DataType::Sampler2d)], &[("fragColor", DataType::Vec4)]),
        ]),
        shader_group("DISTORTION", &[
            ("Twirl", &[("inputImage", DataType::Sampler2d), ("radius", DataType::Float), ("angle", DataType::Float)], &[("fragColor", DataType::Vec4)]),
            ("Ripple", &[("inputImage", DataType::Sampler2d), ("frequency", DataType::Float), ("amplitude", DataType::Float)], &[("fragColor", DataType::Vec4)]),
            ("Displacement", &[("displaceMap", DataType::Sampler2d), ("inputImage", DataType::Sampler2d), ("strength", DataType::Float)], &[("fragColor", DataType::Vec4)]),
            ("Barrel", &[("inputImage", DataType::Sampler2d), ("k1", DataType::Float), ("k2", DataType::Float)], &[("fragColor", DataType::Vec4)]),
            ("Pinch", &[("inputImage", DataType::Sampler2d), ("radius", DataType::Float), ("strength", DataType::Float)], &[("fragColor", DataType::Vec4)]),
        ]),
        shader_group("FEEDBACK", &[
            ("Gray-Scott Reaction-Diffusion", &[("dA", DataType::Float), ("dB", DataType::Float), ("feedRate", DataType::Float), ("killRate", DataType::Float), ("timestep", DataType::Float)], &[("fragColor", DataType::Vec4)]),
        ]),
    ]
}

fn safe_divide(a: f64, b: f64) -> f64 {
    if b == 0.0 {
        0.0
    } else {
        a / b
    }
}

fn safe_modulo(a: f64, b: f64) -> f64 {
    if b == 0.0 {
        0.0
    } else {
        a % b
    }
}

fn port(id: &str, label: &str, data_type: DataType, direction: PortDirection) -> Port {
    Port {
        id: id.to_owned(),
        label: label.to_owned(),
        data_type,
        direction,
        default_value: None,
        description: None,
    }
}

fn float_param(default: f64, min: f64, max: f64, step: f64, label: &str) -> ParamDescriptor {
    ParamDescriptor {
        value_type: "float".to_owned(),
        default: json!(default),
        min: Some(min),
        max: Some(max),
        step: Some(step),
        label: label.to_owned(),
    }
}

fn onnx_model<const N: usize>(
    id: &str,
    label: &str,
    task: OnnxTask,
    category: &str,
    download_url: &str,
    file_size: u64,
    sha256: &str,
    inputs: Vec<Port>,
    outputs: Vec<Port>,
    default_params: Option<[(&str, ParamDescriptor); N]>,
) -> OnnxModelDescriptor {
    OnnxModelDescriptor {
        id: id.to_owned(),
        label: label.to_owned(),
        task,
        category: category.to_owned(),
        download_url: download_url.to_owned(),
        file_size,
        sha256: sha256.to_owned(),
        expected_io: OnnxIoDescriptor { inputs, outputs },
        default_params: default_params.map(|params| {
            params
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value))
                .collect()
        }),
    }
}

fn shader_group(
    category: &str,
    templates: &[(&str, &[(&str, DataType)], &[(&str, DataType)])],
) -> ShaderGroupDescriptor {
    ShaderGroupDescriptor {
        category: category.to_owned(),
        items: templates
            .iter()
            .map(|(label, inputs, outputs)| ShaderTemplateDescriptor {
                id: label.to_lowercase().replace(' ', "-"),
                label: (*label).to_owned(),
                inputs: shader_ports(inputs),
                outputs: shader_ports(outputs),
            })
            .collect(),
    }
}

fn shader_ports(ports: &[(&str, DataType)]) -> Vec<ShaderPortDescriptor> {
    ports
        .iter()
        .map(|(label, data_type)| ShaderPortDescriptor {
            label: (*label).to_owned(),
            data_type: *data_type,
        })
        .collect()
}
