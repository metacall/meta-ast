//! Shard name portability and edge ownership.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use meta_ast::graph::{CodeGraph, EdgeKind, GraphBuilder};
use meta_ast::model::SnapshotId;
use meta_ast::output::shard::{ShardEdge, ShardEdgeKind, ShardFile, restore_shard_edges};
use meta_ast::pipeline::GraphAnalysis;
use tempfile::tempdir;

fn two_file_project() -> tempfile::TempDir {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("a.py"),
        "import b\n\ndef alpha():\n    return b.beta()\n",
    )
    .unwrap();
    fs::write(dir.path().join("b.py"), "def beta():\n    return 1\n").unwrap();
    dir
}

type Analyzed = (tempfile::TempDir, GraphAnalysis, Vec<ShardFile>);

fn analyzed_project() -> Analyzed {
    let dir = two_file_project();
    let (analysis, _diagnostics) =
        meta_ast::pipeline::analyze_graph(dir.path(), SnapshotId::new(1).unwrap(), None).unwrap();
    let shards = analysis
        .extractions
        .iter()
        .map(|file| ShardFile::from_extraction(file, &analysis.graph).unwrap())
        .collect();
    (dir, analysis, shards)
}

fn edge_counts(graph: &CodeGraph) -> BTreeMap<EdgeKind, usize> {
    [
        EdgeKind::Ownership,
        EdgeKind::Import,
        EdgeKind::Reference,
        EdgeKind::Flow,
    ]
    .into_iter()
    .map(|kind| (kind, graph.edges_of_kind(kind).count()))
    .collect()
}

/// The same edge reached two shards, so the index stored cross-file edges
/// twice and the payload grew with every import.
#[test]
fn a_cross_file_edge_is_stored_once() {
    let (_dir, _analysis, shards) = analyzed_project();

    let mut occurrences: BTreeMap<(String, String, ShardEdgeKind), usize> = BTreeMap::new();
    for shard in &shards {
        for edge in &shard.edges {
            *occurrences
                .entry((
                    edge.source_name.clone(),
                    edge.target_name.clone(),
                    edge.kind,
                ))
                .or_default() += 1;
        }
    }

    assert!(!occurrences.is_empty(), "the fixture produces edges");
    let duplicated: Vec<_> = occurrences
        .iter()
        .filter(|(_key, count)| **count > 1)
        .map(|(key, count)| (key, *count))
        .collect();
    assert!(
        duplicated.is_empty(),
        "one shard writes each edge: {duplicated:?}"
    );

    let import_is_stored = occurrences.iter().any(|((source, target, kind), count)| {
        *kind == ShardEdgeKind::Import
            && *count == 1
            && source.contains("a.py")
            && target.contains("b.py")
    });
    assert!(
        import_is_stored,
        "the import edge between the two files is stored once: {occurrences:?}"
    );
}

/// Restore relies on the graph merge rule, so a repeated restore must not
/// change a count. The rule is the only reason the duplicate above stayed
/// invisible.
#[test]
fn restoring_the_same_edges_twice_changes_nothing() {
    let (_dir, analysis, shards) = analyzed_project();
    let edges: Vec<ShardEdge> = shards
        .iter()
        .flat_map(|shard| shard.edges.clone())
        .collect();
    assert!(!edges.is_empty(), "the fixture produces edges");

    let mut rebuilt = rebuild(&analysis);
    let before = edge_counts(&rebuilt);
    restore_shard_edges(&mut rebuilt, &edges).unwrap();
    let once = edge_counts(&rebuilt);
    restore_shard_edges(&mut rebuilt, &edges).unwrap();
    let twice = edge_counts(&rebuilt);

    assert_eq!(once, twice, "a repeated restore merges into the same edges");
    assert!(
        once.values().sum::<usize>() > before.values().sum::<usize>(),
        "the restore added the persisted edges: before {before:?}, after {once:?}"
    );
}

fn rebuild(analysis: &GraphAnalysis) -> CodeGraph {
    let mut diagnostics = Vec::new();
    let (graph, _scc) = GraphBuilder::from_extractions(
        &analysis.extractions,
        Path::new("."),
        SnapshotId::new(2).unwrap(),
        &mut diagnostics,
    );
    graph
}
