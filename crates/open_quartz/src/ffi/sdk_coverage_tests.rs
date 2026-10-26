use super::*;
use serde_json::Value;

#[test]
fn aggregate_bindings_cover_projects_graph_commands_nodes_and_errors() {
    let sdk = OpenQuartzBinding::default();
    let mut project = sdk.create_project("Binding");
    assert_eq!(project.name(), "Binding");
    project.set_name("Renamed".to_owned());
    assert_eq!(project.name(), "Renamed");
    let project_json = project.to_json().unwrap();
    assert!(sdk.open_project("{").is_err());
    assert!(sdk.normalize_project("{").is_err());
    assert!(sdk.screen_saver_export_project("{", "renderer").is_err());
    let reopened = sdk.open_project(&project_json).unwrap();
    assert_eq!(reopened.name(), "Renamed");

    let graph = project.graph();
    assert_eq!(graph.revision(), 0);
    assert_eq!(graph.snapshot_json().unwrap(), r#"{"nodes":[],"edges":[]}"#);
    assert!(graph.initialize("{").is_err());
    graph.initialize(r#"{"nodes":[],"edges":[]}"#).unwrap();
    let created: Value = serde_json::from_str(
        &graph
            .create_node(r#"{"kind":"constant","position":null}"#, 0)
            .unwrap(),
    )
    .unwrap();
    assert!(graph.initialize(r#"{"nodes":[],"edges":[]}"#).is_err());
    assert!(graph.replace("{", 0).is_err());
    assert!(graph.apply("{", 0).is_err());
    assert!(graph.create_node("{", 0).is_err());

    let node_id = created["node"]["id"].as_str().unwrap();
    assert!(graph.node_json(node_id).is_some());
    assert!(graph.node_json("missing").is_none());
    assert!(graph.can_connect("missing", "out", node_id, "in").is_err());

    let command = format!(
        r#"{{"kind":"setNodePosition","nodeId":"{node_id}","position":{{"x":2.0,"y":3.0}}}}"#
    );
    assert!(graph
        .apply(&command, graph.revision())
        .unwrap()
        .contains("changedNodes"));
    let revision = graph.revision();
    assert!(graph
        .replace(r#"{"nodes":[],"edges":[]}"#, revision)
        .is_ok());
    assert!(graph.rollback(graph.revision()).is_ok());
    assert!(graph.redo(graph.revision()).is_ok());
    assert!(graph.rollback(0).is_err());
    assert!(graph.redo(graph.revision()).is_err());
    assert!(graph
        .apply(
            r#"{"kind":"setNodePosition","nodeId":"missing","position":{"x":0.0,"y":0.0}}"#,
            graph.revision(),
        )
        .is_err());
    assert!(graph
        .create_node(
            r#"{"kind":"math","position":null,"op":"unknown"}"#,
            graph.revision(),
        )
        .is_err());
    assert!(graph.replace(r#"{"nodes":[],"edges":[]}"#, 0).is_err());

    assert!(project.screen_saver_graph("missing", 1, 1).is_err());
    let valid_project = sdk.create_project("Screen Saver");
    let valid_graph = valid_project.graph();
    let screen_graph = r#"{"nodes":[{"id":"source","type":"shader","position":{"x":0.0,"y":0.0},"data":{"type":"shader","label":"Source","shaderCode":"@fragment fn main() -> @location(0) vec4f { return vec4f(1.0); }","inputs":[],"outputs":[{"id":"out","label":"out","dataType":"sampler2D","direction":"output"}],"uniforms":{}}},{"id":"renderer","type":"renderer","position":{"x":1.0,"y":0.0},"data":{"type":"renderer","label":"Renderer","shaderCode":"","inputs":[{"id":"in","label":"in","dataType":"sampler2D","direction":"input"}],"outputs":[],"uniforms":{}}}],"edges":[{"id":"edge","source":"source","sourceHandle":"out","target":"renderer","targetHandle":"in"}]}"#;
    valid_graph.initialize(screen_graph).unwrap();
    assert!(valid_project
        .screen_saver_graph("renderer", 1920, 1080)
        .unwrap()
        .contains("__screen_saver_output_resample"));
    assert!(sdk
        .screen_saver_export_project(&project.to_json().unwrap(), "missing")
        .is_err());
}

#[test]
fn player_binding_covers_lifecycle_outputs_and_disposed_errors() {
    let sdk = OpenQuartzBinding::new();
    let project = sdk.create_project("Player");
    let mut player = project.create_player().unwrap();
    assert_eq!(player.graph_revision(), 1);
    assert_eq!(player.outputs_json(), "[]");
    assert!(player.pause().is_err());
    player.play().unwrap();
    player.pause().unwrap();
    player.resume().unwrap();
    player.stop().unwrap();
    player.close().unwrap();
    assert!(player.play().is_err());
    assert!(player.pause().is_err());
    assert!(player.resume().is_err());
    assert!(player.stop().is_err());
    player.close().unwrap();

    let output_project = sdk.create_project("Outputs");
    let output_graph = output_project.graph();
    output_graph
        .create_node(
            r#"{"kind":"shader","position":null,"code":"@fragment fn main() -> @location(0) vec4f { return vec4f(1.0); }","label":"Output","templateName":null,"shaderTemplateId":null}"#,
            0,
        )
        .unwrap();
    let output_player = output_project.create_player().unwrap();
    assert!(output_player.outputs_json().contains("shader_1"));

    let change = crate::GraphChange::from_changed_nodes([crate::NodeId::new("node")]);
    assert!(change_json(2, &change).contains("node"));
}
