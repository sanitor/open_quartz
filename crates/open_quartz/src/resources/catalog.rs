use serde::{Deserialize, Serialize};

use crate::project::{ResourceId, SdkError, SdkErrorCode};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResourceKind {
    Image,
    Video,
    Camera,
    Model,
    Bytes,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "kebab-case")]
pub enum ResourceSource {
    Path(String),
    Url(String),
    Device(String),
    Embedded(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resource {
    id: ResourceId,
    kind: ResourceKind,
    source: ResourceSource,
}

impl Resource {
    pub fn new(id: ResourceId, kind: ResourceKind, source: ResourceSource) -> Self {
        Self { id, kind, source }
    }

    pub fn id(&self) -> &ResourceId {
        &self.id
    }

    pub fn kind(&self) -> ResourceKind {
        self.kind
    }

    pub fn source(&self) -> &ResourceSource {
        &self.source
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResourceCatalog {
    resources: Vec<Resource>,
}

impl ResourceCatalog {
    pub fn add(&mut self, resource: Resource) -> Result<(), SdkError> {
        if self
            .resources
            .iter()
            .any(|current| current.id == resource.id)
        {
            return Err(SdkError::new(
                SdkErrorCode::InvalidResource,
                "Project resource ID is already registered",
            )
            .with_details(resource.id.to_string()));
        }
        self.resources.push(resource);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.resources.len()
    }

    pub fn is_empty(&self) -> bool {
        self.resources.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Resource> {
        self.resources.iter()
    }
}

#[cfg(test)]
#[path = "catalog_coverage_tests.rs"]
mod coverage_tests;
