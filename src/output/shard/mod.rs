//! `.metast` v2 JSONL shard and index serialization.
//!
//! Provides the persistence model for `.meta-ast/` index directories:
//! - `shards/<n>.jsonl`: Per-file AST symbols, unresolved items, and stable-name graph edges.
//! - `manifest.jsonl`: File metadata with BLAKE3 content hashes.
//! - `header.json`: Index schema and versioning metadata.
//!
//! Shards persist stable language-scoped qualified names instead of run-local graph identifiers.
//! Loading regenerates symbol identifiers through the caller's `IdGenerator`.

pub mod edge;
pub mod error;
pub mod file;
pub mod header;
pub mod manifest;
pub(crate) mod name;

pub use edge::{ShardEdge, ShardEdgeKind, ShardFlowKind, restore_shard_edges};
pub use error::ShardError;
pub use file::{
    LoadedShard, SHARD_SCHEMA_VERSION, ShardFile, ShardSymbol, read_shard, write_shard,
};
pub use header::{ShardHeader, read_header, write_header};
pub use manifest::{ShardManifestRecord, read_manifest, write_manifest};

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::graph::{EdgeKind, GraphBuilder};
    use crate::language::LangId;
    use crate::model::{
        FileExtraction, IdGenerator, LineColumn, SnapshotId, SourceRange, Symbol, SymbolId,
        SymbolKind, Visibility,
    };

    fn range() -> SourceRange {
        SourceRange {
            byte_start: 0,
            byte_end: 12,
            start: LineColumn { line: 0, column: 0 },
            end: LineColumn {
                line: 0,
                column: 12,
            },
        }
    }

    fn extraction() -> FileExtraction {
        let path = PathBuf::from("src/example.py");
        FileExtraction {
            path: path.clone(),
            lang: LangId::Python,
            symbols: vec![Symbol {
                id: SymbolId::new(91).unwrap(),
                name: "encrypt".to_string(),
                kind: SymbolKind::Function,
                language: LangId::Python,
                file_path: path,
                source_range: range(),
                visibility: Some(Visibility::Public),
                signature: Some("def encrypt(value: str)".to_string()),
                docstring: Some("Encrypt a value.".to_string()),
                is_async: false,
            }],
            imports: Vec::new(),
            references: Vec::new(),
            diagnostics: Vec::new(),
            ast_node_count: 7,
            #[cfg(feature = "metacall-deploy")]
            call_sites: Vec::new(),
            #[cfg(feature = "dataflow")]
            data_nodes: Vec::new(),
            #[cfg(feature = "dataflow")]
            flow_edges: Vec::new(),
        }
    }

    #[test]
    fn shard_round_trip_regenerates_symbol_ids() {
        let extraction = extraction();
        let mut diagnostics = Vec::new();
        let (graph, _) = GraphBuilder::from_extractions(
            std::slice::from_ref(&extraction),
            Path::new("."),
            SnapshotId::new(1).unwrap(),
            &mut diagnostics,
        );
        let shard = ShardFile::from_extraction(&extraction, &graph).unwrap();
        let mut bytes = Vec::new();
        write_shard(&mut bytes, &[shard]).unwrap();
        let decoded = read_shard(Cursor::new(bytes)).unwrap();
        let loaded = decoded
            .into_iter()
            .next()
            .unwrap()
            .load(&IdGenerator::with_start(500))
            .unwrap();

        assert_eq!(loaded.file.symbols[0].id, SymbolId::new(500).unwrap());
        assert_eq!(loaded.file.symbols[0].name, "encrypt");
        assert_eq!(loaded.file.symbols[0].source_range, range());
        assert_eq!(loaded.edges.len(), 1);
    }

    #[test]
    fn shard_json_omits_numeric_graph_ids() {
        let extraction = extraction();
        let mut diagnostics = Vec::new();
        let (graph, _) = GraphBuilder::from_extractions(
            std::slice::from_ref(&extraction),
            Path::new("."),
            SnapshotId::new(1).unwrap(),
            &mut diagnostics,
        );
        let shard = ShardFile::from_extraction(&extraction, &graph).unwrap();
        let json = serde_json::to_string(&shard).unwrap();

        assert!(!json.contains("\"id\""));
        assert!(json.contains("python src%2Fexample.py . encrypt#function!0 ."));
        assert!(json.contains("\"kind\":\"ownership\""));
    }

    #[test]
    fn stable_symbol_name_ignores_source_offset_changes() {
        let first = extraction();
        let mut shifted = first.clone();
        shifted.symbols[0].source_range.byte_start = 100;
        shifted.symbols[0].source_range.byte_end = 112;
        let mut diagnostics = Vec::new();
        let (first_graph, _) = GraphBuilder::from_extractions(
            std::slice::from_ref(&first),
            Path::new("."),
            SnapshotId::new(1).unwrap(),
            &mut diagnostics,
        );
        let (shifted_graph, _) = GraphBuilder::from_extractions(
            std::slice::from_ref(&shifted),
            Path::new("."),
            SnapshotId::new(2).unwrap(),
            &mut diagnostics,
        );
        let first_shard = ShardFile::from_extraction(&first, &first_graph).unwrap();
        let shifted_shard = ShardFile::from_extraction(&shifted, &shifted_graph).unwrap();

        assert_eq!(
            first_shard.edges[0].target_name,
            shifted_shard.edges[0].target_name
        );
    }

    #[test]
    fn restored_edges_use_graph_normalization() {
        let extraction = extraction();
        let mut diagnostics = Vec::new();
        let (mut graph, _) = GraphBuilder::from_extractions(
            std::slice::from_ref(&extraction),
            Path::new("."),
            SnapshotId::new(1).unwrap(),
            &mut diagnostics,
        );
        let file_index = graph
            .files()
            .next()
            .and_then(|(id, _)| graph.file_node_index(id))
            .unwrap();
        let symbol_index = graph
            .symbols()
            .next()
            .and_then(|(id, _)| graph.symbol_node_index(id))
            .unwrap();
        graph.add_edge_normalized(file_index, symbol_index, EdgeKind::Reference, 0.4);
        let shard = ShardFile::from_extraction(&extraction, &graph).unwrap();
        let loaded = shard.load(&IdGenerator::with_start(500)).unwrap();
        let (mut rebuilt, _) = GraphBuilder::from_extractions(
            std::slice::from_ref(&loaded.file),
            Path::new("."),
            SnapshotId::new(2).unwrap(),
            &mut diagnostics,
        );

        restore_shard_edges(&mut rebuilt, &loaded.edges).unwrap();

        let references = rebuilt.edges_of_kind(EdgeKind::Reference).count();
        assert_eq!(references, 1);
    }

    #[test]
    fn reader_reports_line_for_invalid_json() {
        let error = read_shard(Cursor::new("\n{invalid}\n")).unwrap_err();
        assert!(matches!(error, ShardError::Decode { line: 2, .. }));
    }

    #[cfg(unix)]
    #[test]
    fn unix_backslash_and_separator_paths_stay_distinct() {
        let backslash = name::normalized_path(Path::new("a\\b.py")).unwrap();
        let separator = name::normalized_path(Path::new("a/b.py")).unwrap();
        assert_ne!(backslash, separator);
    }

    #[test]
    fn reader_rejects_invalid_edge_metadata() {
        let file = ShardFile {
            schema_version: SHARD_SCHEMA_VERSION,
            path: PathBuf::from("a.py"),
            language: LangId::Python,
            symbols: Vec::new(),
            imports: Vec::new(),
            references: Vec::new(),
            diagnostics: Vec::new(),
            ast_node_count: 0,
            edges: vec![ShardEdge {
                source_name: "python file a.py".to_string(),
                target_name: "python file b.py".to_string(),
                kind: ShardEdgeKind::Import,
                confidence: 1.5,
                flow_kind: None,
            }],
        };
        let mut output = Vec::new();
        let write_error = write_shard(&mut output, std::slice::from_ref(&file)).unwrap_err();
        assert!(matches!(
            write_error,
            ShardError::InvalidEdge { line: 1, .. }
        ));

        let input = format!("{}\n", serde_json::to_string(&file).unwrap());
        let read_error = read_shard(Cursor::new(input)).unwrap_err();
        assert!(matches!(
            read_error,
            ShardError::InvalidEdge { line: 1, .. }
        ));
    }

    #[test]
    fn reader_rejects_other_schema_versions() {
        let mut value = serde_json::to_value(ShardFile {
            schema_version: SHARD_SCHEMA_VERSION,
            path: PathBuf::from("a.py"),
            language: LangId::Python,
            symbols: Vec::new(),
            imports: Vec::new(),
            references: Vec::new(),
            diagnostics: Vec::new(),
            ast_node_count: 0,
            edges: Vec::new(),
        })
        .unwrap();
        value["schema_version"] = serde_json::json!(99);
        let input = format!("{}\n", serde_json::to_string(&value).unwrap());

        let error = read_shard(Cursor::new(input)).unwrap_err();
        assert!(matches!(
            error,
            ShardError::SchemaVersion {
                line: 1,
                found: 99,
                ..
            }
        ));
    }

    #[test]
    fn header_round_trip() {
        let header = ShardHeader::new("2026-08-29T12:00:00Z");
        assert_eq!(header.schema_version, SHARD_SCHEMA_VERSION);
        assert_eq!(header.tool_version, env!("CARGO_PKG_VERSION"));

        let mut bytes = Vec::new();
        write_header(&mut bytes, &header).unwrap();
        let loaded = read_header(Cursor::new(bytes)).unwrap();
        assert_eq!(header, loaded);
    }

    #[test]
    fn manifest_round_trip() {
        let record = ShardManifestRecord::from_file_bytes(
            PathBuf::from("src/main.py"),
            b"def main(): pass\n",
            1724932800,
            "shards/0.jsonl".to_string(),
        );
        assert_eq!(record.schema_version, SHARD_SCHEMA_VERSION);
        assert_eq!(record.size, 17);
        assert_eq!(
            record.content_hash,
            blake3::hash(b"def main(): pass\n").to_hex().to_string()
        );

        let mut bytes = Vec::new();
        write_manifest(&mut bytes, std::slice::from_ref(&record)).unwrap();
        let decoded = read_manifest(Cursor::new(bytes)).unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0], record);
    }
}
