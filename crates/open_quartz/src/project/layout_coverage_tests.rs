use super::*;
use crate::player::{Environment, OpenQuartz};
use crate::project::test_support::*;
use crate::project::NodeType;

#[test]
fn project_layout_and_initialization_contracts_cover_failures() {
    let sdk = OpenQuartz::new(Environment::headless());
    let graph = renderer_graph();
    let mut project = sdk.create_project("Initial");
    assert_eq!(project.name(), "Initial");
    project.set_name("Renamed");
    assert_eq!(project.name(), "Renamed");
    assert!(project.layout().position(&NodeId::new("missing")).is_none());
    assert_eq!(
        project
            .layout_mut()
            .set_position(NodeId::new("missing"), Position { x: 1.0, y: 2.0 })
            .unwrap_err()
            .code,
        SdkErrorCode::UnknownNode
    );
    project.initialize_graph(graph.clone()).unwrap();
    project
        .graph_mut()
        .nodes
        .push(node("without-layout", NodeType::Input, vec![], vec![]));
    assert_eq!(
        project
            .to_file()
            .graph
            .nodes
            .iter()
            .find(|node| node.id == "without-layout")
            .unwrap()
            .position,
        Position { x: 0.0, y: 0.0 }
    );
    assert_eq!(
        project
            .replace_graph(project.graph_snapshot(), 99)
            .unwrap_err()
            .code,
        SdkErrorCode::StaleRevision
    );
    assert!(project.layout().position(&NodeId::new("source")).is_some());
    assert_eq!(
        project.initialize_graph(graph.clone()).unwrap_err().code,
        SdkErrorCode::InvalidState
    );
}

#[test]
fn layout_updates_cover_project_positions() {
    let mut project = Project::try_from_file(project_file(renderer_graph())).unwrap();
    project
        .layout_mut()
        .set_position(NodeId::new("source"), Position { x: 10.0, y: 20.0 })
        .unwrap();
    assert_eq!(
        project.layout().position(&NodeId::new("source")),
        Some(Position { x: 10.0, y: 20.0 })
    );
}
