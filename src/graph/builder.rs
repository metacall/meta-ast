// Graph builder for incremental construction from extraction results.
//!
//! Construction proceeds in two phases:
//! 1. Ownership graph: files and their symbols
//! 2. Dependency graph: imports and cross-file references
//!
//! The builder maintains bidirectional mappings between domain IDs
//! (FileId, SymbolId) and petgraph NodeIndex for efficient lookups.

use std::collections::HashMap;
use std::path::PathBuf;

use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;

use crate::graph::CodeGraph;
use crate::graph::edge::{EdgeData, EdgeKind};
use crate::graph::node::{FileNode, NodeData, SymbolNode};
use crate::language::LangId;
use crate::model::{FileId, IdGenerator, SnapshotId, Symbol, SymbolId};

/// Builder for incremental graph construction from extraction results.
#[derive(Debug)]
pub struct GraphBuilder {
    /// Underlying graph being constructed
    graph: DiGraph<NodeData, EdgeData>,

    /// Map from FileId to graph node index
    file_to_index: HashMap<FileId, NodeIndex>,

    /// Map from SymbolId to graph node index
    symbol_to_index: HashMap<SymbolId, NodeIndex>,

    /// Map from file path to FileId (for resolving import targets)
    path_to_file: HashMap<PathBuf, FileId>,

    /// ID generator for FileIds
    file_id_gen: IdGenerator<FileId>,

    /// Snapshot ID for this analysis
    snapshot_id: SnapshotId,

    /// Counter for edge deduplication
    edge_normalizer: EdgeNormalizer,
}

/// Tracks seen edges to prevent duplicates.
#[derive(Debug, Default)]
struct EdgeNormalizer {
    seen: std::collections::HashSet<(NodeIndex, NodeIndex, EdgeKind)>,
}

impl EdgeNormalizer {
    fn is_new(&mut self, source: NodeIndex, target: NodeIndex, kind: EdgeKind) -> bool {
        self.seen.insert((source, target, kind))
    }
}

impl GraphBuilder {
    /// Creates a new GraphBuilder for the given snapshot.
    pub fn new(snapshot_id: SnapshotId) -> Self {
        Self {
            graph: DiGraph::new(),
            file_to_index: HashMap::new(),
            symbol_to_index: HashMap::new(),
            path_to_file: HashMap::new(),
            file_id_gen: IdGenerator::new(),
            snapshot_id,
            edge_normalizer: EdgeNormalizer::default(),
        }
    }

    /// Adds a file node to the graph.
    ///
    /// If the file already exists (determined by path), returns the existing FileId.
    pub fn add_file(&mut self, path: PathBuf, language: LangId) -> FileId {
        // Check if file already exists
        if let Some(&existing_id) = self.path_to_file.get(&path) {
            return existing_id;
        }

        let id = self.file_id_gen.next();

        let node = FileNode {
            id,
            path: path.clone(),
            language,
            snapshot_id: self.snapshot_id,
        };

        let idx = self.graph.add_node(NodeData::File(node));

        self.file_to_index.insert(id, idx);
        self.path_to_file.insert(path, id);

        id
    }

    /// Adds a symbol node and its ownership edge to the containing file.
    ///
    /// The file must already exist in the builder; this is enforced by the
    /// FileId parameter. The symbol's file_id must match a previously added file.
    ///
    /// Returns the NodeIndex for the symbol node.
    pub fn add_symbol(&mut self, symbol: &Symbol) -> NodeIndex {
        // Check if symbol already exists
        if let Some(&existing) = self.symbol_to_index.get(&symbol.id) {
            return existing;
        }

        // Look up the file by path
        let file_id = *self
            .path_to_file
            .get(&symbol.file_path)
            .expect("file must be added before its symbols");

        let file_idx = *self
            .file_to_index
            .get(&file_id)
            .expect("file index must exist");

        let node = SymbolNode {
            id: symbol.id,
            name: symbol.name.clone(),
            kind: symbol.kind,
            file_id,
            visibility: symbol.visibility,
            source_range: symbol.source_range.clone(),
        };

        let sym_idx = self.graph.add_node(NodeData::Symbol(node));
        self.symbol_to_index.insert(symbol.id, sym_idx);

        // Add ownership edge: file -> symbol
        self.add_edge_internal(file_idx, sym_idx, EdgeKind::Ownership);

        sym_idx
    }

    /// Adds an import edge from one file to another.
    ///
    /// Both source and target files must exist in the builder.
    /// If the target file doesn't exist, this is a no-op (external dependency).
    pub fn add_import(&mut self, from: FileId, to: PathBuf) {
        let Some(&from_idx) = self.file_to_index.get(&from) else {
            return; // Source not in graph
        };

        // Resolve target path to file ID if it exists in our graph
        let Some(&to_id) = self.path_to_file.get(&to) else {
            // Target not in project - external dependency, skip for now
            return;
        };

        let Some(&to_idx) = self.file_to_index.get(&to_id) else {
            return; // Should not happen if path_to_file is consistent
        };

        self.add_edge_internal(from_idx, to_idx, EdgeKind::Import);
    }

    /// Adds a reference edge between two symbols.
    ///
    /// Both symbols must exist in the builder.
    pub fn add_reference(&mut self, from: SymbolId, to: SymbolId) {
        let Some(&from_idx) = self.symbol_to_index.get(&from) else {
            return;
        };
        let Some(&to_idx) = self.symbol_to_index.get(&to) else {
            return;
        };

        self.add_edge_internal(from_idx, to_idx, EdgeKind::Reference);
    }

    /// Internal method to add an edge with deduplication.
    fn add_edge_internal(&mut self, source: NodeIndex, target: NodeIndex, kind: EdgeKind) {
        if !self.edge_normalizer.is_new(source, target, kind) {
            return; // Duplicate edge, skip
        }

        let edge_data = EdgeData {
            kind,
            confidence: 1.0, // Strong confidence for explicit imports
        };

        self.graph.add_edge(source, target, edge_data);
    }

    /// Returns the FileId for a given file path, if registered.
    pub fn file_id_for_path(&self, path: &std::path::PathBuf) -> Option<FileId> {
        self.path_to_file.get(path).copied()
    }

    // TODO(MVP): Handle ExternalImportFlag - some imports should add a
    //             placeholder external node instead of being silently dropped.

    /// Builds and returns an adjacency map from FileId to the FileIds it imports.
    ///
    /// Walks all Import edges in the graph to produce the relationship.
    pub fn import_adjacency(&self) -> HashMap<FileId, Vec<FileId>> {
        let mut adjacency: HashMap<FileId, Vec<FileId>> = HashMap::new();
        let mut index_to_file: HashMap<NodeIndex, FileId> = HashMap::new();
        for (&file_id, &idx) in &self.file_to_index {
            index_to_file.insert(idx, file_id);
        }
        for edge in self.graph.edge_references() {
            if edge.weight().kind == EdgeKind::Import
                && let (Some(&from_id), Some(&to_id)) = (
                    index_to_file.get(&edge.source()),
                    index_to_file.get(&edge.target()),
                )
            {
                adjacency.entry(from_id).or_default().push(to_id);
            }
        }
        adjacency
    }

    /// Finalizes the graph and returns the constructed CodeGraph.
    pub fn build(self) -> CodeGraph {
        CodeGraph {
            graph: self.graph,
            file_to_index: self.file_to_index,
            symbol_to_index: self.symbol_to_index,
            snapshot_id: self.snapshot_id,
        }
    }

    /// Returns the number of nodes in the graph so far.
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Returns the number of edges in the graph so far.
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{LineColumn, SourceRange, SymbolKind, Visibility};

    fn test_source_range() -> SourceRange {
        SourceRange {
            byte_start: 0,
            byte_end: 10,
            start: LineColumn { line: 0, column: 0 },
            end: LineColumn {
                line: 0,
                column: 10,
            },
        }
    }

    fn test_symbol(id: u32, name: &str, _file_id: u32) -> Symbol {
        Symbol {
            id: SymbolId(id),
            name: name.to_string(),
            kind: SymbolKind::Function,
            language: LangId::Python,
            file_path: PathBuf::from("test.py"),
            source_range: test_source_range(),
            visibility: Some(Visibility::Public),
            signature: None,
            docstring: None,
            is_async: false,
        }
    }

    #[test]
    fn builder_creates_file_nodes() {
        let mut builder = GraphBuilder::new(SnapshotId(1));
        let path = PathBuf::from("src/main.py");

        let id1 = builder.add_file(path.clone(), LangId::Python);
        let id2 = builder.add_file(path, LangId::Python);

        assert_eq!(id1, id2, "same path should return same FileId");
        assert_eq!(builder.node_count(), 1);
    }

    #[test]
    fn builder_creates_symbol_with_ownership() {
        let mut builder = GraphBuilder::new(SnapshotId(1));
        let path = PathBuf::from("test.py");

        let file_id = builder.add_file(path.clone(), LangId::Python);
        let symbol = test_symbol(1, "hello", file_id.0);

        let _sym_idx = builder.add_symbol(&symbol);

        assert_eq!(builder.node_count(), 2); // file + symbol
        assert_eq!(builder.edge_count(), 1); // ownership edge
    }

    #[test]
    fn builder_deduplicates_edges() {
        let mut builder = GraphBuilder::new(SnapshotId(1));
        let path1 = PathBuf::from("a.py");
        let path2 = PathBuf::from("b.py");

        let file1 = builder.add_file(path1, LangId::Python);
        let _file2 = builder.add_file(path2, LangId::Python);

        // Add same import twice
        builder.add_import(file1, PathBuf::from("b.py"));
        builder.add_import(file1, PathBuf::from("b.py"));

        assert_eq!(
            builder.edge_count(),
            1,
            "duplicate edges should be deduplicated"
        );
    }

    #[test]
    fn builder_skips_external_imports() {
        let mut builder = GraphBuilder::new(SnapshotId(1));
        let path = PathBuf::from("main.py");

        let file_id = builder.add_file(path, LangId::Python);

        // Import of external module not in our project
        builder.add_import(file_id, PathBuf::from("external_module.py"));

        assert_eq!(
            builder.edge_count(),
            0,
            "external imports should not create edges"
        );
    }

    #[test]
    fn builder_tracks_node_mappings() {
        let mut builder = GraphBuilder::new(SnapshotId(1));
        let path = PathBuf::from("test.py");

        let file_id = builder.add_file(path, LangId::Python);
        let symbol = test_symbol(42, "func", file_id.0);

        builder.add_symbol(&symbol);

        // Verify mappings exist
        assert!(builder.file_to_index.contains_key(&file_id));
        assert!(builder.symbol_to_index.contains_key(&SymbolId(42)));
    }
}
