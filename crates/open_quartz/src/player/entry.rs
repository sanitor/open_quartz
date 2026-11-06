use crate::project::{Graph, Project, ProjectFile, SdkError, SdkErrorCode};

use super::{Environment, PlayerBuilder};

#[derive(Clone, Debug)]
pub struct OpenQuartz {
    environment: Environment,
}

impl OpenQuartz {
    pub fn new(environment: Environment) -> Self {
        Self { environment }
    }

    pub fn create_project(&self, name: impl Into<String>) -> Project {
        Project::new(name)
    }

    pub fn open_project_json(&self, project_json: &str) -> Result<Project, SdkError> {
        let file: ProjectFile = serde_json::from_str(project_json).map_err(|error| {
            SdkError::new(SdkErrorCode::InvalidState, "Cannot decode project")
                .with_details(error.to_string())
        })?;
        Project::try_from_file(file)
    }

    pub fn normalize_project_json(&self, project_json: &str) -> Result<String, SdkError> {
        let project = self.open_project_json(project_json)?;
        Ok(serde_json::to_string(&project.to_file())
            .expect("ProjectFile serialization should be infallible"))
    }

    pub fn screen_saver_export_project_json(
        &self,
        project_json: &str,
        renderer_node_id: &str,
    ) -> Result<String, SdkError> {
        let mut project = self.open_project_json(project_json)?;
        project.retain_upstream_graph(renderer_node_id)?;
        Ok(serde_json::to_string(&project.to_file())
            .expect("screen saver ProjectFile serialization should be infallible"))
    }

    pub fn player(&self, graph: &Graph) -> PlayerBuilder {
        PlayerBuilder::new(self.environment.clone(), graph.clone())
    }
}

#[cfg(test)]
#[path = "entry_coverage_tests.rs"]
mod coverage_tests;
