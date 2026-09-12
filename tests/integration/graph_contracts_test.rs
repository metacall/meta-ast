//! Graph invariants: one edge merge rule, one naming authority, one path index.
//!
//! The merge rule and the naming authority are checked structurally, because a
//! second copy of either is the defect. The scan is narrow: it looks for the
//! exact expression each rule is written with.

use std::path::PathBuf;

use meta_ast::LangId;
use meta_ast::graph::{CodeGraph, EdgeKind};
use meta_ast::model::SnapshotId;

fn read_source(relative: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

fn occurrences(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

/// Returns the body of the first function whose signature contains `signature`.
fn function_body(source: &str, signature: &str) -> String {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("{signature} is not in the source"));
    let rest = &source[start..];
    let open = rest.find('{').expect("a function has a body");
    let mut depth = 0usize;
    for (offset, character) in rest[open..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return rest[..open + offset + 1].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced body for {signature}");
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn snapshot_id() -> SnapshotId {
    let id = SnapshotId::new(1);
    assert!(id.is_some(), "1 is a valid snapshot id");
    id.unwrap()
}

/// The duplicate rule must be written once. A second copy drifts: one keeps the
/// first flow kind, the other overwrites it.
#[test]
fn the_edge_merge_rule_exists_once() {
    let surfaces = [
        read_source("src/graph/mod.rs"),
        read_source("src/graph/builder.rs"),
        read_source("src/graph/edge.rs"),
    ];
    let copies: usize = surfaces
        .iter()
        .map(|source| occurrences(source, "confidence.max("))
        .sum();
    assert_eq!(
        copies, 1,
        "the duplicate-edge merge rule must live in one place, found {copies} copies"
    );
}

/// Both entry points must agree: the builder and the graph keep the stronger
/// confidence and never lower it.
#[test]
fn repeated_edges_keep_the_stronger_confidence() {
    let mut graph = CodeGraph::new(snapshot_id());
    let left = graph.get_or_create_external_node("react".to_string(), LangId::JavaScript);
    let right = graph.get_or_create_external_node("node:fs".to_string(), LangId::JavaScript);

    graph.add_edge_normalized(left, right, EdgeKind::Import, 0.4);
    graph.add_edge_normalized(left, right, EdgeKind::Import, 0.9);
    graph.add_edge_normalized(left, right, EdgeKind::Import, 0.5);

    let edges: Vec<_> = graph
        .graph()
        .edge_indices()
        .filter(|&index| {
            graph
                .graph()
                .edge_weight(index)
                .is_some_and(|weight| weight.kind == EdgeKind::Import)
        })
        .collect();
    assert_eq!(edges.len(), 1, "a repeated triple is one edge");
    let weight = graph.graph().edge_weight(edges[0]).unwrap();
    assert_eq!(weight.confidence, 0.9, "the stronger confidence wins");
}

/// The first flow kind wins: a later duplicate must not overwrite it.
#[test]
fn the_first_flow_kind_is_kept() {
    use meta_ast::model::FlowKind;

    let mut graph = CodeGraph::new(snapshot_id());
    let left = graph.get_or_create_external_node("a".to_string(), LangId::Python);
    let right = graph.get_or_create_external_node("b".to_string(), LangId::Python);

    graph.add_edge_normalized_with_flow(left, right, EdgeKind::Flow, 0.1, Some(FlowKind::Argument));
    graph.add_edge_normalized_with_flow(left, right, EdgeKind::Flow, 0.8, Some(FlowKind::Return));

    let edges: Vec<_> = graph
        .graph()
        .edge_indices()
        .filter(|&index| {
            graph
                .graph()
                .edge_weight(index)
                .is_some_and(|weight| weight.kind == EdgeKind::Flow)
        })
        .collect();
    assert_eq!(edges.len(), 1);
    let weight = graph.graph().edge_weight(edges[0]).unwrap();
    assert_eq!(weight.confidence, 0.8);
    assert_eq!(weight.flow_kind, Some(FlowKind::Argument));
}

/// The kind name of a node comes from the graph layer, not from the writer.
#[test]
fn node_names_come_from_one_authority() {
    let graph_output = read_source("src/output/graph.rs");
    assert!(
        graph_output.contains("naming::node_kind_name("),
        "the graph output must take the node kind name from graph::naming"
    );
    assert!(
        graph_output.contains("naming::node_display_name("),
        "the graph output must take the node display name from graph::naming"
    );

    let naming = read_source("src/graph/naming.rs");
    for function in [
        "pub fn node_kind_name(",
        "pub fn node_display_name(",
        "pub fn node_language(",
    ] {
        assert!(
            naming.contains(function),
            "graph::naming must expose {function}"
        );
    }
}

/// The graph output is a pure function of the graph.
#[test]
fn graph_output_is_deterministic() {
    let root = fixture("mixed");
    let (analysis, _) = meta_ast::pipeline::analyze_graph(&root, snapshot_id(), None).unwrap();

    let first =
        meta_ast::output::graph::GraphOutput::from_graph(&analysis.graph, Some(&analysis.scc), 7);
    let second =
        meta_ast::output::graph::GraphOutput::from_graph(&analysis.graph, Some(&analysis.scc), 7);

    let first = serde_json::to_string(&first).unwrap();
    let second = serde_json::to_string(&second).unwrap();
    assert_eq!(first, second);
    assert!(!first.is_empty());
}

/// Call sites are resolved against an extraction index, never a scan per site.
#[test]
fn client_call_resolution_indexes_extractions_by_path() {
    let source = read_source("src/deploy/client_call.rs");
    let enclosing = function_body(&source, "fn enclosing_symbol(");
    assert!(
        !enclosing.contains(".find(|file| file.path"),
        "enclosing_symbol must take its file from an index built once, not a per-call scan"
    );
    assert!(
        source.contains("path_to_extraction"),
        "resolve_client_call_projections must build a path to extraction index"
    );
}

/// The caller symbol projection of a client call reaches the graph.
#[cfg(feature = "metacall-deploy")]
#[test]
fn client_call_carries_a_symbol_projection() {
    let root = fixture("mixed/client_call_mesh");
    let (analysis, _) = meta_ast::pipeline::analyze_graph(&root, snapshot_id(), None).unwrap();

    let mut cross_language_targets = Vec::new();
    for (source, target, _confidence) in analysis.graph.reference_edges() {
        let source_symbol = analysis.graph.symbol_node(source).unwrap();
        let target_symbol = analysis.graph.symbol_node(target).unwrap();
        let source_language = analysis
            .graph
            .file_node(source_symbol.file_id)
            .map(|file| file.language);
        let target_language = analysis
            .graph
            .file_node(target_symbol.file_id)
            .map(|file| file.language);
        if source_language != target_language {
            cross_language_targets.push(target_symbol.name.clone());
        }
    }
    assert!(
        cross_language_targets.iter().any(|name| name == "multiply"),
        "the resolved client call must reach the Node export, found {cross_language_targets:?}"
    );
}
