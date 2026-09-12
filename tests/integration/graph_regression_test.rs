//! Regression coverage for graph resolution contracts.

use std::path::{Path, PathBuf};

use meta_ast::graph::{EdgeKind, NodeData};
use meta_ast::model::SnapshotId;
use petgraph::visit::EdgeRef;

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mixed/client_call_mesh")
}

fn analyze() -> meta_ast::pipeline::GraphAnalysis {
    let (analysis, _diags) =
        meta_ast::pipeline::analyze_graph(&fixture_root(), SnapshotId::new(1).unwrap(), None)
            .unwrap();
    analysis
}

/// The deployment path needs the calling file as the edge source, because a
/// top-level call has no enclosing symbol.
#[cfg(feature = "metacall-deploy")]
#[test]
fn client_calls_edge_from_the_calling_file() {
    let analysis = analyze();
    let graph = analysis.graph.graph();
    let from_file = graph
        .edge_references()
        .filter(|edge| {
            edge.weight().kind == EdgeKind::Reference
                && matches!(graph[edge.source()], NodeData::File(_))
                && matches!(graph[edge.target()], NodeData::Symbol(_))
        })
        .count();
    assert_eq!(
        from_file, 1,
        "the resolved metacall call must produce one file to symbol reference edge"
    );
}

/// The language server navigates by symbol, so the symbol projection stays.
#[cfg(feature = "metacall-deploy")]
#[test]
fn client_calls_keep_a_symbol_projection() {
    let analysis = analyze();
    let graph = analysis.graph.graph();
    let from_symbol = graph
        .edge_references()
        .filter(|edge| {
            edge.weight().kind == EdgeKind::Reference
                && matches!(graph[edge.source()], NodeData::Symbol(_))
                && matches!(graph[edge.target()], NodeData::Symbol(_))
                && edge.source() != edge.target()
        })
        .count();
    assert!(
        from_symbol >= 1,
        "a call inside a symbol must stay visible to symbol keyed consumers"
    );
}
