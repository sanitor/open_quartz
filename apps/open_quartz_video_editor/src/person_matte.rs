use open_quartz::inference::{BackendPreference, ExecutionPolicy, InferenceConfig, PreparedInferenceModel};
use open_quartz::project::catalog::{inference_models, InferenceModelDescriptor};
use open_quartz::project::{Graph, InferenceSource, NodeFactoryRequest, NodeType, Position, ProjectNode};
use open_quartz::resources::models::ModelStore;
use std::path::PathBuf;
use std::sync::LazyLock;

pub(crate) const LABEL: &str = "Person Matte";
const MODEL: &str = "u2netp";

pub(crate) fn policy() -> ExecutionPolicy {
    ExecutionPolicy { backend: BackendPreference::Auto, ..ExecutionPolicy::resident() }
}

static DESCRIPTOR: LazyLock<InferenceModelDescriptor> = LazyLock::new(|| {
    inference_models().into_iter().find(|entry| entry.id == MODEL)
        .expect("Person Matte is backed by a built-in catalog model")
});

pub(crate) fn factory(position: Position) -> NodeFactoryRequest {
    NodeFactoryRequest::Inference {
        position: Some(position),
        label: LABEL.into(),
        template_name: Some(LABEL.into()),
        model_id: Some(MODEL.into()),
        catalog_id: Some(MODEL.into()),
        inputs: DESCRIPTOR.expected_io.inputs.clone(),
        outputs: DESCRIPTOR.expected_io.outputs.clone(),
    }
}

pub(crate) fn is_effect(node: &ProjectNode) -> bool {
    let data = &node.data;
    node.node_type == NodeType::Inference
        && data.inference_source == Some(InferenceSource::Catalog)
        && data.inference_model_id.as_deref() == Some(MODEL)
        && data.inference_catalog_id.as_deref() == Some(MODEL)
        && data.inference_policy.as_ref() == Some(&policy())
        && data.inference_params.is_none()
        && data.inference_model_path.is_none()
        && data.shader_code.is_empty()
        && crate::clip_pipeline::ports_match(&data.inputs, &DESCRIPTOR.expected_io.inputs)
        && crate::clip_pipeline::ports_match(&data.outputs, &DESCRIPTOR.expected_io.outputs)
}

fn store() -> Result<ModelStore, String> {
    if let Some(path) = std::env::var_os("OPEN_QUARTZ_STUDIO_MODEL_DIR") {
        return Ok(ModelStore::new(path.into()));
    }
    let directory = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("HOME").map(|path| PathBuf::from(path).join("Library/Application Support"))
    }.ok_or("Cannot locate the Studio model cache")?;
    Ok(ModelStore::new(directory.join("OpenQuartz Studio").join("Models")))
}

pub(crate) fn stage(
    prepared: &mut open_quartz::execution::PreparedNativeGraph,
    previous: Option<&Graph>,
    check: impl Fn() -> Result<(), String>,
) -> Result<(), String> {
    let pending: Vec<_> = prepared.graph().nodes.iter().filter(|node| node.node_type == NodeType::Inference)
        .filter(|node| !previous.is_some_and(|old| old.nodes.iter().any(|prior| prior.id == node.id && is_effect(prior))))
        .map(|node| node.id.clone()).collect();
    if pending.is_empty() {
        return Ok(());
    }
    let store = store()?;
    store.download(MODEL, &DESCRIPTOR.download_url, DESCRIPTOR.file_size, &DESCRIPTOR.sha256, |_, _| check())?;
    for node in pending {
        check()?;
        let model: PreparedInferenceModel = store.open_cached(MODEL, InferenceConfig {
            model_id: MODEL.into(), task: DESCRIPTOR.task, target_size: 320, policy: policy(), ..InferenceConfig::default()
        })?;
        prepared.stage_inference(&node, model)?;
    }
    Ok(())
}
