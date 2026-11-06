use super::*;
use crate::project::test_support::*;

use crate::resources::{Resource, ResourceKind, ResourceSource};

#[test]
fn project_files_reject_invalid_graphs_and_versions() {
    let invalid_file = project_file(Graph {
        nodes: vec![
            node("duplicate", NodeType::Input, vec![], vec![]),
            node("duplicate", NodeType::Input, vec![], vec![]),
        ],
        edges: vec![],
    });
    assert_eq!(
        Project::try_from_file(invalid_file).unwrap_err().code,
        SdkErrorCode::InvalidGraph
    );
    let incompatible = ProjectFile {
        version: "0.3.0".to_owned(),
        name: "Old".to_owned(),
        created_at: String::new(),
        updated_at: String::new(),
        graph: Graph::default(),
    };
    assert_eq!(
        Project::try_from_file(incompatible).unwrap_err().code,
        SdkErrorCode::InvalidState
    );
}

#[test]
fn project_owns_its_resource_catalog() {
    let mut project = Project::try_from_file(project_file(renderer_graph())).unwrap();
    assert_eq!(project.resources().len(), 0);
    project
        .resources_mut()
        .add(Resource::new(
            ResourceId::new("embedded"),
            ResourceKind::Bytes,
            ResourceSource::Embedded("payload".to_owned()),
        ))
        .unwrap();
    assert_eq!(project.resources().len(), 1);
}
