//! The same shader source and creation metadata used by the React shader menu.

use std::sync::LazyLock;

use serde_json::Value;

const CATALOG_JSON: &str = include_str!("../../../src/shaders/catalog.json");

#[derive(Debug)]
pub(crate) struct ShaderPreset {
    pub category: &'static str,
    pub label: &'static str,
    pub code: &'static str,
    pub template_name: &'static str,
    /// Built-ins retain their template identity; custom samples remain editable.
    pub shader_template_id: Option<&'static str>,
}

static CATALOG: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(CATALOG_JSON).expect("embedded shader catalog must be valid JSON")
});

static PRESETS: LazyLock<Vec<ShaderPreset>> = LazyLock::new(|| {
    let catalog: &'static Value = &CATALOG;
    catalog["groups"]
        .as_array()
        .expect("shader catalog groups must be an array")
        .iter()
        .flat_map(|group| {
            let category = group["category"]
                .as_str()
                .expect("shader category must be a string");
            group["items"]
                .as_array()
                .expect("shader category items must be an array")
                .iter()
                .map(move |item| ShaderPreset {
                    category,
                    label: item["label"].as_str().expect("shader label must be a string"),
                    code: item["code"].as_str().expect("shader code must be a string"),
                    template_name: item["templateName"]
                        .as_str()
                        .expect("shader template name must be a string"),
                    shader_template_id: match &item["shaderTemplateId"] {
                        Value::Null => None,
                        Value::String(id) => Some(id.as_str()),
                        _ => panic!("shader template identity must be a string or null"),
                    },
                })
        })
        .collect()
});

/// Preserves menu category and item order, with the editable custom samples first.
pub(crate) fn presets() -> &'static [ShaderPreset] {
    PRESETS.as_slice()
}

/// Resolve menu metadata using the exact node label from a shader factory request.
pub(crate) fn find(label: &str) -> Option<&'static ShaderPreset> {
    presets().iter().find(|preset| preset.label == label)
}
