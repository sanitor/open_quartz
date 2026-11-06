use open_quartz::execution::schedule::{topological_sort, DirtySet, GraphEdge, GraphNode};

fn nodes(ids: &[&str]) -> Vec<GraphNode> {
    ids.iter()
        .map(|id| GraphNode {
            id: (*id).to_owned(),
        })
        .collect()
}

fn edges(pairs: &[(&str, &str)]) -> Vec<GraphEdge> {
    pairs
        .iter()
        .map(|(source, target)| GraphEdge {
            source: (*source).to_owned(),
            target: (*target).to_owned(),
        })
        .collect()
}

#[test]
fn topological_sort_matches_typescript_linear_and_diamond_behavior() {
    assert_eq!(
        topological_sort(&nodes(&["A", "B", "C"]), &edges(&[("A", "B"), ("B", "C")])),
        ["A", "B", "C"]
    );
    assert_eq!(
        topological_sort(
            &nodes(&["A", "B", "C", "D"]),
            &edges(&[("A", "B"), ("A", "C"), ("B", "D"), ("C", "D")]),
        ),
        ["A", "B", "C", "D"]
    );
}

#[test]
fn topological_sort_ignores_unknown_endpoints_and_returns_partial_cycles() {
    assert_eq!(
        topological_sort(&nodes(&["A"]), &edges(&[("unknown", "A")])),
        ["A"]
    );
    assert_eq!(
        topological_sort(
            &nodes(&["S", "A", "B"]),
            &edges(&[("S", "A"), ("A", "B"), ("B", "A")]),
        ),
        ["S"]
    );
}

#[test]
fn dirty_set_propagates_to_all_downstream_nodes() {
    let graph_edges = edges(&[("A", "B"), ("A", "C"), ("B", "D"), ("C", "D")]);
    let mut dirty = DirtySet::new(&graph_edges);
    dirty.mark_dirty("A");
    assert_eq!(dirty.len(), 4);
    assert!(dirty.contains("D"));

    let order = vec!["A", "B", "C", "D"]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(dirty.take_in_order(&order), order);
    assert!(dirty.is_empty());
}

#[test]
fn dirty_set_is_cycle_safe_and_edge_updates_replace_dependents() {
    let mut dirty = DirtySet::new(&edges(&[("A", "B"), ("B", "A")]));
    dirty.mark_dirty("A");
    assert_eq!(dirty.len(), 2);

    dirty.set_edges(&edges(&[("X", "Y")]));
    dirty.mark_dirty("X");
    assert!(dirty.contains("Y"));
}
