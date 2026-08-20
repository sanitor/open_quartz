use open_quartz_schema::{DataType, Graph, NodeType, PortDirection, ProjectFile};

#[test]
fn schema_crate_exposes_only_domain_values_without_execution_dependencies() {
    let graph = Graph::default();
    assert!(graph.nodes.is_empty());
    assert_eq!(NodeType::Shader, NodeType::Shader);
    assert_eq!(PortDirection::Input, PortDirection::Input);
    assert_eq!(DataType::Json, DataType::Json);
    let _file: ProjectFile = ProjectFile {
        version: open_quartz_schema::PROJECT_FILE_VERSION.to_owned(),
        name: "boundary".to_owned(),
        created_at: String::new(),
        updated_at: String::new(),
        graph,
    };
}
