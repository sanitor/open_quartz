use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OnnxTask {
    SuperResolution,
    BackgroundRemoval,
    Detection,
    Segmentation,
    StyleTransfer,
    Denoising,
    DepthEstimation,
    Generic,
}
