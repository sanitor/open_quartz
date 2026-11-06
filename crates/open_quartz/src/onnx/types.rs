pub use crate::domain::OnnxTask;

#[derive(Clone)]
pub struct NativeOnnxConfig {
    pub model_id: String,
    pub task: OnnxTask,
    pub target_size: u32,
    pub score_threshold: f32,
    pub iou_threshold: f32,
}
