//! Golden document for the exported shard set.
//!
//! The exporter writes a persisted format that another process reads. Any
//! change to a field, a name or an ordering must show up as a diff here and
//! force a schema decision, instead of reaching a reader by accident.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use meta_ast::model::{FileExtraction, SnapshotId};
use meta_ast::output::shard::{
    ShardFile, ShardHeader, ShardManifestRecord, write_header, write_manifest, write_shard,
};

/// Fixed in the document, so the golden is independent of the clock.
const HEADER_TIMESTAMP: &str = "2026-01-01T00:00:00Z";
const SHARD_NAME: &str = "shards/000.jsonl";

const REGENERATE: &str =
    "cargo test --all-features --test integration regenerate_the_golden_document -- --ignored";

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Relative to the crate root, so every exported path stays project relative
/// and the golden is the same on any checkout.
fn fixture() -> PathBuf {
    PathBuf::from("tests/fixtures/python")
}

fn golden_path() -> PathBuf {
    repository_root().join("tests/fixtures/shard/golden.json")
}

/// Replaces the checkout prefix, because the exporter records the path it was
/// handed and the golden must be the same on any machine.
fn portable(text: &str) -> String {
    let root = repository_root();
    let root = root.to_string_lossy();
    text.replace(root.as_ref(), "<root>")
}

/// Exports the fixture and returns every artifact as text, keyed by name.
fn export(root: &Path) -> BTreeMap<String, String> {
    let snapshot = SnapshotId::new(1);
    assert!(snapshot.is_some(), "1 is a valid snapshot id");
    let (analysis, _diagnostics) =
        meta_ast::pipeline::analyze_graph(root, snapshot.unwrap(), None).unwrap();

    let mut extractions: Vec<&FileExtraction> = analysis
        .extractions
        .iter()
        .map(|file| file.as_ref())
        .collect();
    extractions.sort_by(|left, right| left.path.cmp(&right.path));
    assert!(!extractions.is_empty(), "the fixture must contain files");

    let mut header = Vec::new();
    write_header(&mut header, &ShardHeader::new(HEADER_TIMESTAMP)).unwrap();

    let mut records = Vec::new();
    for file in &extractions {
        let bytes = std::fs::read(&file.path).unwrap();
        records.push(ShardManifestRecord::from_file_bytes(
            file.path.clone(),
            &bytes,
            0,
            SHARD_NAME.to_string(),
        ));
    }
    let mut manifest = Vec::new();
    write_manifest(&mut manifest, &records).unwrap();

    let mut shards: Vec<(PathBuf, ShardFile)> = analysis
        .extractions
        .iter()
        .map(|file| {
            let shard = ShardFile::from_extraction(file, &analysis.graph).unwrap();
            (file.path.clone(), shard)
        })
        .collect();
    shards.sort_by(|left, right| left.0.cmp(&right.0));
    let shards: Vec<ShardFile> = shards.into_iter().map(|(_, shard)| shard).collect();
    let mut payload = Vec::new();
    write_shard(&mut payload, &shards).unwrap();

    let mut document = BTreeMap::new();
    document.insert(
        "header.json".to_string(),
        portable(&String::from_utf8(header).unwrap()),
    );
    document.insert(
        "manifest.jsonl".to_string(),
        portable(&String::from_utf8(manifest).unwrap()),
    );
    document.insert(
        SHARD_NAME.to_string(),
        portable(&String::from_utf8(payload).unwrap()),
    );
    document
}

#[test]
fn the_shard_export_matches_the_golden_document() {
    let produced = export(&fixture());
    let path = golden_path();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "cannot read {}: {error}. Regenerate it with: {REGENERATE}",
            path.display()
        )
    });
    let golden: BTreeMap<String, String> = serde_json::from_str(&text).unwrap();

    let produced_names: Vec<&String> = produced.keys().collect();
    let golden_names: Vec<&String> = golden.keys().collect();
    assert_eq!(
        produced_names, golden_names,
        "the exported artifact set changed. Regenerate with: {REGENERATE}"
    );

    for (name, expected) in &golden {
        let actual = produced.get(name).unwrap();
        assert_eq!(
            actual, expected,
            "the artifact {name} changed. Regenerate with: {REGENERATE}"
        );
    }
}

/// Rewrites the golden document from the current exporter. Run deliberately:
/// the diff is the review.
#[test]
#[ignore = "rewrites the committed golden document"]
fn regenerate_the_golden_document() {
    let produced = export(&fixture());
    let path = golden_path();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let text = serde_json::to_string_pretty(&produced).unwrap();
    std::fs::write(&path, format!("{text}\n")).unwrap();
}
