//! Shard name portability and edge ownership.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use meta_ast::Severity;
use meta_ast::graph::{CodeGraph, EdgeKind, GraphBuilder};
use meta_ast::model::{IdGenerator, SnapshotId, SymbolId};
use meta_ast::output::shard::{
    INDEX_DIR_NAME, IndexLoadOptions, ShardEdge, ShardEdgeKind, ShardError, ShardFile, ShardHeader,
    ShardManifestRecord, collision_key, is_writable_name, load_index, plan_shard_file_names,
    read_shard, restore_shard_edges, write_header, write_manifest, write_shard,
};
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

fn planned_names(paths: &[&str]) -> Vec<String> {
    let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    plan_shard_file_names(&paths).unwrap().names
}

/// Write one shard per file under the given names, plus the header and the
/// manifest that reference them.
fn write_index(root: &Path, analysis: &GraphAnalysis, names: &[String]) {
    let dir = root.join(INDEX_DIR_NAME);
    fs::create_dir_all(dir.join("shards")).unwrap();

    let mut header = Vec::new();
    write_header(&mut header, &ShardHeader::new("2026-01-01T00:00:00Z")).unwrap();
    fs::write(dir.join("header.json"), header).unwrap();

    let mut records = Vec::new();
    for (file, name) in analysis.extractions.iter().zip(names) {
        let bytes = fs::read(&file.path).unwrap();
        records.push(ShardManifestRecord::from_file_bytes(
            file.path.clone(),
            &bytes,
            0,
            name.clone(),
        ));
        let shard = ShardFile::from_extraction(file, &analysis.graph).unwrap();
        let mut payload = Vec::new();
        write_shard(&mut payload, std::slice::from_ref(&shard)).unwrap();
        fs::write(dir.join(name), payload).unwrap();
    }

    let mut manifest = Vec::new();
    write_manifest(&mut manifest, &records).unwrap();
    fs::write(dir.join("manifest.jsonl"), manifest).unwrap();
}

/// A shard is a file name, so two source names that one platform folds into
/// one name must not share a shard.
#[test]
fn portable_names_of_equivalent_file_names_stay_distinct() {
    const PAIRS: [(&str, &str); 14] = [
        ("a\\b.py", "a/b.py"),
        ("a%2Fb.py", "a%2fb.py"),
        ("caf\u{e9}.py", "cafe\u{301}.py"),
        ("A.py", "a.py"),
        ("name.", "name"),
        ("name ", "name"),
        ("CON", "CON.py"),
        ("a-b.py", "a_b.py"),
        ("a b.py", "a_b.py"),
        ("x%20y.py", "x y.py"),
        ("a+b.py", "a b.py"),
        ("a#b.py", "a?b.py"),
        ("NUL.py", "nul.py"),
        ("a..py", "a.py"),
    ];

    for (first, second) in PAIRS {
        let names = planned_names(&[first, second]);
        assert_eq!(names.len(), 2);
        assert_ne!(
            names[0], names[1],
            "{first:?} and {second:?} need their own shard"
        );
        for name in &names {
            assert!(
                is_writable_name(name),
                "{name} must be writable for {first:?}/{second:?}"
            );
        }
    }
}

/// A path long enough to exceed the common file name limit still gets a
/// writable shard name.
#[test]
fn a_long_path_becomes_a_bounded_name() {
    let long = format!("{}leaf.py", "d/".repeat(120));
    let names = planned_names(&[long.as_str(), "a.py"]);

    assert_ne!(names[0], names[1]);
    for name in &names {
        assert!(
            name.len() <= 255,
            "{name} exceeds the common file name limit ({} bytes)",
            name.len()
        );
        assert!(is_writable_name(name), "{name} must be writable");
    }
}

#[test]
fn collision_key_folds_case_and_trailing_marks() {
    assert_eq!(collision_key("A.py"), collision_key("a.py"));
    assert_ne!(collision_key("A.py"), collision_key("B.py"));
    assert_eq!(collision_key("name."), collision_key("name"));
    assert_eq!(collision_key("name "), collision_key("name"));
    assert_eq!(collision_key("name. ."), "name");
}

#[test]
fn device_names_and_trailing_marks_are_refused() {
    for name in [
        "shards/CON.jsonl",
        "shards/con.jsonl",
        "shards/CON",
        "shards/NUL.py.jsonl",
        "shards/lpt1.jsonl",
        "shards/name.",
        "shards/name ",
        "000.jsonl",
        "shards/../secret.jsonl",
        "/etc/passwd",
    ] {
        assert!(!is_writable_name(name), "{name} must be refused");
    }

    for name in [
        "shards/000.jsonl",
        "shards/a.py.jsonl",
        "shards/%43ON.py.jsonl",
        "shards/CON2.jsonl",
        "shards/console.jsonl",
    ] {
        assert!(is_writable_name(name), "{name} must be accepted");
    }
}

/// The whole writer path on a tree that holds a case colliding pair and a
/// reserved device name: every file gets its own readable shard, the reader
/// takes them back, and the export is byte stable.
#[test]
fn export_disambiguates_equivalent_names_end_to_end() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("A.py"), "def upper():\n    return 1\n").unwrap();
    fs::write(root.join("a.py"), "def lower():\n    return 2\n").unwrap();
    fs::write(root.join("CON.py"), "def device():\n    return 3\n").unwrap();

    let (analysis, _diagnostics) =
        meta_ast::pipeline::analyze_graph(root, SnapshotId::new(1).unwrap(), None).unwrap();
    let relative: Vec<PathBuf> = analysis
        .extractions
        .iter()
        .map(|file| {
            file.path
                .strip_prefix(root)
                .unwrap_or(file.path.as_path())
                .to_path_buf()
        })
        .collect();
    assert_eq!(relative.len(), 3, "the fixture tree holds three files");

    let plan = plan_shard_file_names(&relative).unwrap();
    assert_eq!(plan.names.len(), 3);

    let collision = plan
        .warnings
        .iter()
        .find(|warning| warning.message.contains("A.py") && warning.message.contains("a.py"))
        .expect("one warning names both paths that fold into one name");
    assert_eq!(collision.severity, Severity::Warning);
    assert!(
        collision.message.contains("case"),
        "the reason is named: {}",
        collision.message
    );

    let device = plan
        .warnings
        .iter()
        .find(|warning| warning.message.contains("CON.py"))
        .expect("one warning names the reserved device path");
    assert!(
        device.message.contains("device"),
        "the reason is named: {}",
        device.message
    );

    let distinct: BTreeSet<&String> = plan.names.iter().collect();
    assert_eq!(distinct.len(), 3, "one shard per file: {:?}", plan.names);
    assert!(
        plan.names.iter().all(|name| is_writable_name(name)),
        "every written name is portable: {:?}",
        plan.names
    );

    write_index(root, &analysis, &plan.names);
    for name in &plan.names {
        assert!(
            root.join(INDEX_DIR_NAME).join(name).exists(),
            "{name} was written"
        );
    }

    let loaded = load_index(
        root,
        &IdGenerator::<SymbolId>::new(),
        &IndexLoadOptions::default(),
    )
    .unwrap();
    assert_eq!(loaded.stats.loaded, 3);
    assert_eq!(loaded.stats.skipped, 0);

    let payload = |name: &String| fs::read(root.join(INDEX_DIR_NAME).join(name)).unwrap();
    let before: Vec<Vec<u8>> = plan.names.iter().map(payload).collect();
    write_index(root, &analysis, &plan.names);
    let after: Vec<Vec<u8>> = plan.names.iter().map(payload).collect();

    assert_eq!(before, after, "a second export writes the same bytes");
}

/// A name a portable writer never creates is refused instead of opening a
/// device file on the platform that reserves it.
#[test]
fn reader_refuses_a_name_that_no_platform_can_write() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("a.py"), "def a():\n    return 1\n").unwrap();

    let (analysis, _diagnostics) =
        meta_ast::pipeline::analyze_graph(root, SnapshotId::new(1).unwrap(), None).unwrap();
    write_index(root, &analysis, &["shards/CON.jsonl".to_string()]);

    let error = load_index(
        root,
        &IdGenerator::<SymbolId>::new(),
        &IndexLoadOptions::default(),
    )
    .unwrap_err();

    assert!(
        matches!(error, ShardError::UnwritableShardName { .. }),
        "got {error:?}"
    );
}

/// The loaded totals must account for every persisted edge, per kind.
#[test]
fn loaded_index_reports_edges_by_kind() {
    let (dir, analysis, _shards) = analyzed_project();
    let root = dir.path();
    let names: Vec<String> = (0..analysis.extractions.len())
        .map(|index| format!("shards/{index}.jsonl"))
        .collect();
    write_index(root, &analysis, &names);

    let loaded = load_index(
        root,
        &IdGenerator::<SymbolId>::new(),
        &IndexLoadOptions::default(),
    )
    .unwrap();

    let mut payload_counts: BTreeMap<ShardEdgeKind, usize> = BTreeMap::new();
    for name in &names {
        let file = File::open(root.join(INDEX_DIR_NAME).join(name)).unwrap();
        for record in read_shard(BufReader::new(file)).unwrap() {
            for edge in record.edges {
                *payload_counts.entry(edge.kind).or_default() += 1;
            }
        }
    }

    assert!(!payload_counts.is_empty(), "the fixture stores edges");
    assert_eq!(payload_counts.values().sum::<usize>(), loaded.edges.len());
    assert_eq!(loaded.edge_counts_by_kind(), payload_counts);
}
