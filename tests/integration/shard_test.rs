//! Integration tests for .metast v2 shard and index persistence.

use std::fs;
use std::path::Path;

use meta_ast::graph::GraphBuilder;
use meta_ast::model::{IdGenerator, SnapshotId, SymbolId};
use meta_ast::output::shard::{
    SHARD_SCHEMA_VERSION, ShardFile, ShardHeader, ShardManifestRecord, read_header, read_manifest,
    read_shard, restore_shard_edges, write_header, write_manifest, write_shard,
};

#[test]
fn shard_full_pipeline_roundtrip_and_graph_restoration() {
    let root = Path::new("tests/fixtures/python");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    assert!(!files.is_empty(), "fixture files must be discovered");

    let initial_extraction = meta_ast::extractor::extract(&files);
    let mut initial_diags = Vec::new();
    let (initial_graph, _) = GraphBuilder::from_extractions(
        &initial_extraction.files,
        root,
        SnapshotId::new(1).unwrap(),
        &mut initial_diags,
    );

    let temp_dir = std::env::temp_dir().join("meta_ast_shard_integration_test");
    if temp_dir.exists() {
        let _ = fs::remove_dir_all(&temp_dir);
    }
    fs::create_dir_all(&temp_dir).unwrap();

    let shards_dir = temp_dir.join("shards");
    fs::create_dir_all(&shards_dir).unwrap();

    // 1. Convert extractions to ShardFiles and collect edges
    let shard_files: Vec<ShardFile> = initial_extraction
        .files
        .iter()
        .map(|file| ShardFile::from_extraction(file, &initial_graph).unwrap())
        .collect();

    // 2. Write header.json
    let header = ShardHeader::new("2026-08-29T12:00:00Z");
    let header_path = temp_dir.join("header.json");
    let header_file = fs::File::create(&header_path).unwrap();
    write_header(header_file, &header).unwrap();

    // 3. Write shards/0.jsonl
    let shard_path = shards_dir.join("0.jsonl");
    let shard_out = fs::File::create(&shard_path).unwrap();
    write_shard(shard_out, &shard_files).unwrap();

    // 4. Write manifest.jsonl
    let mut manifest_records = Vec::new();
    for (idx, (path, _lang)) in files.iter().enumerate() {
        let content = fs::read(path).unwrap();
        manifest_records.push(ShardManifestRecord::from_file_bytes(
            path.clone(),
            &content,
            1724932800 + idx as u64,
            "shards/0.jsonl".to_string(),
        ));
    }
    let manifest_path = temp_dir.join("manifest.jsonl");
    let manifest_out = fs::File::create(&manifest_path).unwrap();
    write_manifest(manifest_out, &manifest_records).unwrap();

    // 5. Read back header and manifest
    let header_in = fs::File::open(&header_path).unwrap();
    let loaded_header = read_header(header_in).unwrap();
    assert_eq!(loaded_header.schema_version, SHARD_SCHEMA_VERSION);
    assert_eq!(loaded_header.created_at, "2026-08-29T12:00:00Z");

    let manifest_in = std::io::BufReader::new(fs::File::open(&manifest_path).unwrap());
    let loaded_manifest = read_manifest(manifest_in).unwrap();
    assert_eq!(loaded_manifest.len(), manifest_records.len());
    for (orig, loaded) in manifest_records.iter().zip(&loaded_manifest) {
        assert_eq!(orig.path, loaded.path);
        assert_eq!(orig.content_hash, loaded.content_hash);
        assert_eq!(orig.size, loaded.size);
    }

    // 6. Read back shards and regenerate IDs with new generator
    let shard_in = std::io::BufReader::new(fs::File::open(&shard_path).unwrap());
    let loaded_shard_files = read_shard(shard_in).unwrap();
    assert_eq!(loaded_shard_files.len(), shard_files.len());

    let id_gen = IdGenerator::<SymbolId>::with_start(10_000);
    let mut loaded_extractions = Vec::new();
    let mut all_shard_edges = Vec::new();

    for shard in loaded_shard_files {
        let loaded = shard.load(&id_gen).unwrap();
        all_shard_edges.extend(loaded.edges);
        loaded_extractions.push(loaded.file);
    }

    // Symbol IDs must be >= 10000
    for file in &loaded_extractions {
        for sym in &file.symbols {
            assert!(sym.id.to_raw() >= 10_000);
        }
    }

    // 7. Rebuild graph from loaded extractions and restore edges
    let mut rebuild_diags = Vec::new();
    let (mut rebuilt_graph, _) = GraphBuilder::from_extractions(
        &loaded_extractions,
        root,
        SnapshotId::new(2).unwrap(),
        &mut rebuild_diags,
    );

    restore_shard_edges(&mut rebuilt_graph, &all_shard_edges).unwrap();

    use meta_ast::graph::edge::EdgeKind;

    // Verify rebuilt graph topology matches initial graph for AST nodes and edges
    assert_eq!(rebuilt_graph.files().count(), initial_graph.files().count());
    assert_eq!(
        rebuilt_graph.symbols().count(),
        initial_graph.symbols().count()
    );
    assert_eq!(
        rebuilt_graph.edges_of_kind(EdgeKind::Ownership).count(),
        initial_graph.edges_of_kind(EdgeKind::Ownership).count()
    );
    assert_eq!(
        rebuilt_graph.edges_of_kind(EdgeKind::Import).count(),
        initial_graph.edges_of_kind(EdgeKind::Import).count()
    );
    assert_eq!(
        rebuilt_graph.edges_of_kind(EdgeKind::Reference).count(),
        initial_graph.edges_of_kind(EdgeKind::Reference).count()
    );

    let _ = fs::remove_dir_all(&temp_dir);
}
