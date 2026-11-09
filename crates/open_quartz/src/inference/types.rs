pub use crate::project::model::InferenceTask;

#[derive(Clone)]
pub struct InferenceConfig {
    pub model_id: String,
    pub task: InferenceTask,
    pub target_size: u32,
    pub score_threshold: f32,
    pub iou_threshold: f32,
    pub policy: oq_inference::ExecutionPolicy,
}

impl Default for InferenceConfig {
    fn default() -> Self {
        Self {
            model_id: String::new(),
            task: InferenceTask::Generic,
            target_size: 640,
            score_threshold: 0.25,
            iou_threshold: 0.45,
            policy: oq_inference::ExecutionPolicy::resident(),
        }
    }
}
