//! Hardened `.meta-ast` index loading.

use std::fs;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use meta_ast::model::{IdGenerator, SnapshotId, SymbolId};
use meta_ast::output::shard::{
    INDEX_DIR_NAME, IndexLoadOptions, ShardError, ShardFile, ShardHeader, ShardManifestRecord,
    is_safe_shard_name, load_index, read_manifest, write_header, write_manifest, write_shard,
};
use tempfile::tempdir;

fn project() -> tempfile::TempDir {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("a.py"), "import b\ndef alpha(): pass\n").unwrap();
    fs::write(dir.path().join("b.py"), "def beta(): pass\n").unwrap();
    dir
}

fn write_index(root: &Path) -> usize {
    let snapshot = SnapshotId::new(1).unwrap();
    let (analysis, _diagnostics) = meta_ast::pipeline::analyze_graph(root, snapshot, None).unwrap();
    let dir = root.join(INDEX_DIR_NAME);
    fs::create_dir_all(dir.join("shards")).unwrap();

    let mut header = Vec::new();
    write_header(&mut header, &ShardHeader::new("2026-01-01T00:00:00Z")).unwrap();
    fs::write(dir.join("header.json"), header).unwrap();

    let mut records = Vec::new();
    let mut shards = Vec::new();
    for file in &analysis.extractions {
        let bytes = fs::read(&file.path).unwrap();
        records.push(ShardManifestRecord::from_file_bytes(
            file.path.clone(),
            &bytes,
            0,
            "shards/000.jsonl".to_string(),
        ));
        shards.push(ShardFile::from_extraction(file, &analysis.graph).unwrap());
    }

    let mut manifest = Vec::new();
    write_manifest(&mut manifest, &records).unwrap();
    fs::write(dir.join("manifest.jsonl"), manifest).unwrap();

    let mut shard_bytes = Vec::new();
    write_shard(&mut shard_bytes, &shards).unwrap();
    fs::write(dir.join("shards/000.jsonl"), shard_bytes).unwrap();

    records.len()
}

#[test]
fn load_index_round_trips_extractions_and_edges() {
    let dir = project();
    let expected = write_index(dir.path());

    let id_gen = IdGenerator::<SymbolId>::new();
    let loaded = load_index(dir.path(), &id_gen, &IndexLoadOptions::default()).unwrap();

    assert_eq!(loaded.stats.loaded, expected);
    assert_eq!(loaded.stats.skipped, 0);
    assert_eq!(loaded.extractions.len(), expected);

    let names: Vec<&str> = loaded
        .extractions
        .iter()
        .flat_map(|file| file.symbols.iter().map(|symbol| symbol.name.as_str()))
        .collect();
    assert!(names.contains(&"alpha"), "names: {names:?}");
    assert!(names.contains(&"beta"), "names: {names:?}");
    assert!(!loaded.edges.is_empty());
}

#[test]
fn tampered_content_is_skipped() {
    let dir = project();
    write_index(dir.path());
    fs::write(dir.path().join("a.py"), "def replaced(): pass\n").unwrap();

    let id_gen = IdGenerator::<SymbolId>::new();
    let loaded = load_index(dir.path(), &id_gen, &IndexLoadOptions::default()).unwrap();

    assert_eq!(loaded.stats.loaded, 1);
    assert_eq!(loaded.stats.skipped, 1);
    assert!(
        loaded
            .extractions
            .iter()
            .flat_map(|file| file.symbols.iter())
            .all(|symbol| symbol.name != "alpha")
    );
}

#[test]
fn unsafe_shard_name_is_rejected() {
    let dir = project();
    write_index(dir.path());
    let manifest_path = dir.path().join(INDEX_DIR_NAME).join("manifest.jsonl");
    let mut records = read_manifest(BufReader::new(File::open(&manifest_path).unwrap())).unwrap();
    for record in &mut records {
        record.shard = "shards/../secret.jsonl".to_string();
    }
    let mut manifest = Vec::new();
    write_manifest(&mut manifest, &records).unwrap();
    fs::write(&manifest_path, manifest).unwrap();

    let id_gen = IdGenerator::<SymbolId>::new();
    let error = load_index(dir.path(), &id_gen, &IndexLoadOptions::default()).unwrap_err();
    assert!(matches!(error, ShardError::UnsafeShardName { .. }));
}

#[test]
fn tool_version_mismatch_is_rejected() {
    let dir = project();
    write_index(dir.path());
    let header_path = dir.path().join(INDEX_DIR_NAME).join("header.json");
    let mut header = Vec::new();
    write_header(
        &mut header,
        &ShardHeader::with_tool_version("0.0.0", "2026-01-01T00:00:00Z"),
    )
    .unwrap();
    fs::write(&header_path, header).unwrap();

    let id_gen = IdGenerator::<SymbolId>::new();
    let error = load_index(dir.path(), &id_gen, &IndexLoadOptions::default()).unwrap_err();
    assert!(matches!(error, ShardError::ToolVersionMismatch { .. }));
}

#[test]
fn path_outside_root_is_rejected() {
    let dir = project();
    write_index(dir.path());
    let outside = dir.path().parent().unwrap().join("outside.py");
    fs::write(&outside, "def outside(): pass\n").unwrap();

    let manifest_path = dir.path().join(INDEX_DIR_NAME).join("manifest.jsonl");
    let mut records = read_manifest(BufReader::new(File::open(&manifest_path).unwrap())).unwrap();
    let original = dir.path().join("a.py");
    for record in &mut records {
        if record.path == original {
            record.path = outside.clone();
        }
    }
    let mut manifest = Vec::new();
    write_manifest(&mut manifest, &records).unwrap();
    fs::write(&manifest_path, manifest).unwrap();

    let id_gen = IdGenerator::<SymbolId>::new();
    let error = load_index(dir.path(), &id_gen, &IndexLoadOptions::default()).unwrap_err();
    assert!(matches!(error, ShardError::PathOutsideRoot { .. }));
}

#[test]
fn ids_are_assigned_in_manifest_order() {
    let dir = project();
    write_index(dir.path());

    let collect = |loaded: &meta_ast::LoadedIndex| -> Vec<(String, u32)> {
        loaded
            .extractions
            .iter()
            .flat_map(|file| {
                file.symbols
                    .iter()
                    .map(|symbol| (symbol.name.clone(), symbol.id.to_raw()))
            })
            .collect()
    };

    let first_gen = IdGenerator::<SymbolId>::new();
    let first = load_index(dir.path(), &first_gen, &IndexLoadOptions::default()).unwrap();
    let second_gen = IdGenerator::<SymbolId>::new();
    let second = load_index(dir.path(), &second_gen, &IndexLoadOptions::default()).unwrap();

    assert_eq!(collect(&first), collect(&second));
}

#[test]
fn shard_name_safety() {
    assert!(is_safe_shard_name("shards/000.jsonl"));
    assert!(!is_safe_shard_name("shards/../secret"));
    assert!(!is_safe_shard_name("/etc/passwd"));
    assert!(!is_safe_shard_name("000.jsonl"));
}
