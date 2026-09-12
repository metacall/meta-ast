//! The one-shot and the incremental analysis paths must produce the same graph.
//!
//! Both entry points share the graph assembly stage. A divergence in files,
//! symbols, edges, hints or diagnostics is therefore a defect in the shared
//! stage, not in the caller. The comparison is by path and name, never by raw
//! identifier, because identifier numbering depends on the call order.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use meta_ast::graph::{CodeGraph, NodeData};
use meta_ast::model::SnapshotId;
use meta_ast::{GraphAnalysis, WatchState, incremental_reanalyze, pipeline};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn describe(node: Option<&NodeData>) -> String {
    match node {
        Some(NodeData::File(file)) => format!("file:{}", file.path.display()),
        Some(NodeData::Symbol(symbol)) => format!(
            "symbol:{}:{}:{:?}",
            symbol.name,
            symbol.kind.as_str(),
            symbol.visibility
        ),
        Some(NodeData::External(external)) => {
            format!("external:{}:{}", external.raw_path, external.language)
        }
        Some(NodeData::Data(data)) => {
            format!("data:{}", data.name.as_deref().unwrap_or("<anonymous>"))
        }
        Some(other) => format!("other:{}", other.kind_str()),
        None => "missing".to_string(),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Observation {
    files: BTreeSet<String>,
    symbols: BTreeSet<String>,
    edges: BTreeSet<String>,
    hints: Vec<String>,
    diagnostics: Vec<String>,
    counts: (usize, usize, usize),
}

fn observe(analysis: &GraphAnalysis, diagnostics: &[meta_ast::Diagnostic]) -> Observation {
    let graph: &CodeGraph = &analysis.graph;

    let files = graph
        .files()
        .map(|(_, file)| format!("{}|{}", file.path.display(), file.language))
        .collect();

    let symbols = graph
        .symbols()
        .map(|(_, symbol)| {
            let file = graph
                .file_node(symbol.file_id)
                .map_or_else(String::new, |node| node.path.display().to_string());
            format!("{file}|{}|{}", symbol.name, symbol.kind.as_str())
        })
        .collect();

    let mut edges = BTreeSet::new();
    let raw = graph.graph();
    for edge in raw.edge_indices() {
        let Some(weight) = raw.edge_weight(edge) else {
            continue;
        };
        let Some((source, target)) = raw.edge_endpoints(edge) else {
            continue;
        };
        edges.insert(format!(
            "{:?}|{}|{}|{:.3}|{:?}",
            weight.kind,
            describe(raw.node_weight(source)),
            describe(raw.node_weight(target)),
            weight.confidence,
            weight.flow_kind
        ));
    }

    let mut hints: Vec<String> = analysis
        .scc
        .hint_counts()
        .into_iter()
        .map(|(hint, count)| format!("{hint}:{count}"))
        .collect();
    hints.sort();

    let mut oneshot_diagnostics: Vec<String> = diagnostics
        .iter()
        .map(|diagnostic| {
            format!(
                "{:?}|{}|{}",
                diagnostic.severity,
                diagnostic.path.display(),
                diagnostic.message
            )
        })
        .collect();
    oneshot_diagnostics.sort();

    Observation {
        files,
        symbols,
        edges,
        hints,
        diagnostics: oneshot_diagnostics,
        counts: (graph.file_count(), graph.symbol_count(), graph.edge_count()),
    }
}

fn assert_paths_agree(root: &Path) {
    let snapshot_id = SnapshotId::new(1);
    assert!(snapshot_id.is_some(), "1 is a valid snapshot id");
    let snapshot_id = snapshot_id.unwrap();
    let (one_shot, one_shot_diagnostics) = pipeline::analyze_graph(root, snapshot_id, None)
        .unwrap_or_else(|error| panic!("one-shot analysis of {} failed: {error}", root.display()));

    let mut state = WatchState::new();
    let (incremental, change_set, incremental_diagnostics) =
        incremental_reanalyze(root, None, &mut state).unwrap_or_else(|error| {
            panic!("incremental analysis of {} failed: {error}", root.display())
        });

    let expected = observe(&one_shot, &one_shot_diagnostics);
    let actual = observe(&incremental, &incremental_diagnostics);

    assert_eq!(
        expected.counts,
        actual.counts,
        "counts (files, symbols, edges) differ for {}",
        root.display()
    );
    assert_eq!(
        expected.files,
        actual.files,
        "files differ for {}",
        root.display()
    );
    assert_eq!(
        expected.symbols,
        actual.symbols,
        "symbols differ for {}",
        root.display()
    );
    assert_eq!(
        expected.edges,
        actual.edges,
        "edges differ for {}",
        root.display()
    );
    assert_eq!(
        expected.hints,
        actual.hints,
        "hints differ for {}",
        root.display()
    );
    assert_eq!(
        expected.diagnostics,
        actual.diagnostics,
        "diagnostics differ for {}",
        root.display()
    );

    let discovered = expected.counts.0;
    assert_eq!(
        change_set.files_added, discovered,
        "a cold incremental pass must add every discovered file"
    );
    assert_eq!(change_set.files_unchanged, 0);
}

#[test]
fn one_shot_and_incremental_agree_on_the_multi_language_fixture() {
    assert_paths_agree(&fixture("multi"));
}

#[test]
fn one_shot_and_incremental_agree_on_the_cross_language_fixture() {
    assert_paths_agree(&fixture("mixed"));
}

#[test]
fn one_shot_and_incremental_agree_on_a_circular_pair() {
    assert_paths_agree(&fixture("multi/edge_circular"));
}

#[cfg(feature = "metacall-deploy")]
#[test]
fn one_shot_and_incremental_agree_on_client_calls() {
    assert_paths_agree(&fixture("mixed/client_call_mesh"));
}
