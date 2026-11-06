pub(crate) mod model;
pub mod catalog;
mod history;
mod layout;
mod node_factory;
mod screen_saver;

pub use model::*;
pub use layout::GraphLayout;

use crate::resources::ResourceCatalog;

#[derive(Clone, Debug)]
pub struct Project {
    version: String,
    name: String,
    created_at: String,
    updated_at: String,
    graph: Graph,
    graph_revision: u32,
    graph_history: Vec<Graph>,
    graph_redo: Vec<Graph>,
    layout: GraphLayout,
    resources: ResourceCatalog,
}

impl Project {
    pub(crate) fn new(name: impl Into<String>) -> Self {
        Self {
            version: PROJECT_FILE_VERSION.to_owned(),
            name: name.into(),
            created_at: String::new(),
            updated_at: String::new(),
            graph: Graph::default(),
            graph_revision: 0,
            graph_history: Vec::new(),
            graph_redo: Vec::new(),
            layout: GraphLayout::default(),
            resources: ResourceCatalog::default(),
        }
    }

    pub(crate) fn retain_upstream_graph(&mut self, renderer_node_id: &str) -> Result<(), SdkError> {
        self.graph = self.graph.upstream_subgraph(&NodeId::new(renderer_node_id))?;
        Ok(())
    }

    pub fn try_from_file(file: ProjectFile) -> Result<Self, SdkError> {
        if file.version != PROJECT_FILE_VERSION {
            return Err(
                SdkError::new(SdkErrorCode::InvalidState, "Incompatible project version")
                    .with_details(format!(
                        "expected {PROJECT_FILE_VERSION}, got {}",
                        file.version
                    )),
            );
        }
        file.graph.validate()?;
        Ok(Self::from_file(file))
    }

    pub fn from_file(file: ProjectFile) -> Self {
        let layout = GraphLayout::from_graph(&file.graph);
        Self {
            version: file.version,
            name: file.name,
            created_at: file.created_at,
            updated_at: file.updated_at,
            graph: file.graph,
            graph_revision: 0,
            graph_history: Vec::new(),
            graph_redo: Vec::new(),
            layout,
            resources: ResourceCatalog::default(),
        }
    }

    pub fn to_file(&self) -> ProjectFile {
        let mut graph = self.graph.clone();
        for node in &mut graph.nodes {
            if let Some(position) = self.layout.position(&NodeId::new(&node.id)) {
                node.position = position;
            }
        }
        ProjectFile {
            version: self.version.clone(),
            name: self.name.clone(),
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
            graph,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
    }

    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    pub fn graph_revision(&self) -> u32 {
        self.graph_revision
    }

    pub fn graph_snapshot(&self) -> Graph {
        self.graph.clone()
    }
    pub fn graph_mut(&mut self) -> &mut Graph {
        &mut self.graph
    }

    pub fn layout(&self) -> &GraphLayout {
        &self.layout
    }

    pub fn layout_mut(&mut self) -> &mut GraphLayout {
        &mut self.layout
    }

    pub fn resources(&self) -> &ResourceCatalog {
        &self.resources
    }

    pub fn resources_mut(&mut self) -> &mut ResourceCatalog {
        &mut self.resources
    }
}

#[cfg(test)]
#[path = "tests/support.rs"]
mod test_support;
#[cfg(test)]
mod coverage_tests;
