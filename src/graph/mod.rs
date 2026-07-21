//! Dependency graph module for cross-file and symbol-level analysis.
//!
//! This module provides graph data structures and algorithms for analyzing
//! dependencies between source files and their contained symbols. It supports:
//!
//! - Building a directed graph from extracted symbols
//! - Computing strongly connected components (SCCs)
//! - Providing deployability hints based on cycle detection
//!
//! # Design as stated in RFC 0008
//!
//! The graph layer sits between the extraction layer and the output layer:
//! - `node.rs` defines `FileNode` and `SymbolNode` types
//! - `edge.rs` defines `EdgeKind` (Ownership, Import, Reference)
//! - `builder.rs` provides `GraphBuilder` for incremental construction
//! - `scc.rs` provides SCC analysis via Tarjan's algorithm
//!
//! # Example
//!
//! ```
//! use meta_ast::graph::{GraphBuilder, SccAnalysis, CodeGraph};
//! use meta_ast::model::SnapshotId;
//!
//! // Create builder and add files/symbols
//! let mut builder = GraphBuilder::new(SnapshotId(1));
//! // ... add nodes and edges ...
//! let graph = builder.build();
//!
//! // Run SCC analysis
//! let scc = SccAnalysis::analyze(&graph.graph);
//! ```

pub mod builder;
pub mod edge;
pub mod node;
pub mod resolver;
pub mod scc;

use std::collections::HashMap;

pub use builder::GraphBuilder;
pub use edge::{EdgeData, EdgeKind};
pub use node::{ExternalClassification, ExternalNode, FileNode, NodeData, SymbolNode};
pub use scc::{DeployabilityHint, Scc, SccAnalysis};

use crate::language::LangId;
use crate::model::{FileId, SnapshotId, SymbolId};
use petgraph::graph::{DiGraph, NodeIndex};

/// The canonical dependency graph for a codebase snapshot.
#[derive(Debug, Clone)]
pub struct CodeGraph {
    /// The underlying petgraph with our node/edge data types.
    pub graph: DiGraph<NodeData, EdgeData>,

    /// Map from FileId to graph node index for O(1) lookup.
    pub(crate) file_to_index: HashMap<FileId, NodeIndex>,

    /// Map from SymbolId to graph node index for O(1) lookup.
    pub(crate) symbol_to_index: HashMap<SymbolId, NodeIndex>,

    /// Map from external raw path to graph node index for O(1) lookup.
    pub(crate) external_index: HashMap<String, NodeIndex>,

    /// Snapshot identifier for this graph as discussed before.
    pub snapshot_id: SnapshotId,
}

impl CodeGraph {
    /// Creates an empty CodeGraph for the given snapshot.
    pub fn new(snapshot_id: SnapshotId) -> Self {
        Self {
            graph: DiGraph::new(),
            file_to_index: HashMap::new(),
            symbol_to_index: HashMap::new(),
            external_index: HashMap::new(),
            snapshot_id,
        }
    }
    pub fn file_node_index(&self, file_id: FileId) -> Option<NodeIndex> {
        self.file_to_index.get(&file_id).copied()
    }
    pub fn symbol_node_index(&self, symbol_id: SymbolId) -> Option<NodeIndex> {
        self.symbol_to_index.get(&symbol_id).copied()
    }
    pub fn file_node(&self, file_id: FileId) -> Option<&FileNode> {
        let idx = self.file_node_index(file_id)?;
        self.graph.node_weight(idx).and_then(|data| data.as_file())
    }
    pub fn symbol_node(&self, symbol_id: SymbolId) -> Option<&SymbolNode> {
        let idx = self.symbol_node_index(symbol_id)?;
        self.graph
            .node_weight(idx)
            .and_then(|data| data.as_symbol())
    }
    pub fn file_count(&self) -> usize {
        self.file_to_index.len()
    }
    pub fn symbol_count(&self) -> usize {
        self.symbol_to_index.len()
    }
    pub fn external_count(&self) -> usize {
        self.external_index.len()
    }
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }
    /// Resolves or creates the `External` node for a raw unresolved path, keeping
    /// `external_index` consistent so repeated loads reuse a single node.
    pub fn get_or_create_external_node(&mut self, raw_path: String, language: LangId) -> NodeIndex {
        if let Some(&idx) = self.external_index.get(&raw_path) {
            return idx;
        }
        let node = NodeData::External(ExternalNode {
            raw_path: raw_path.clone(),
            language,
            classification: None,
        });
        let idx = self.graph.add_node(node);
        self.external_index.insert(raw_path, idx);
        idx
    }
    /// Adds an edge, normalizing duplicate `(src, dst, kind)` triples by max-merging
    /// confidence so injected edges obey the same invariant as builder-constructed ones.
    pub fn add_edge_normalized(
        &mut self,
        source: NodeIndex,
        target: NodeIndex,
        kind: EdgeKind,
        confidence: f32,
    ) {
        let mut edge_idx = self.graph.first_edge(source, petgraph::Direction::Outgoing);
        while let Some(e) = edge_idx {
            let (_src, dst) = self.graph.edge_endpoints(e).unwrap();
            if dst == target && self.graph[e].kind == kind {
                self.graph[e].confidence = self.graph[e].confidence.max(confidence);
                return;
            }
            edge_idx = self.graph.next_edge(e, petgraph::Direction::Outgoing);
        }
        self.graph
            .add_edge(source, target, EdgeData { kind, confidence });
    }
    pub fn files(&self) -> impl Iterator<Item = (FileId, &FileNode)> + '_ {
        self.file_to_index.iter().filter_map(|(file_id, &idx)| {
            self.graph
                .node_weight(idx)
                .and_then(|data| data.as_file().map(|f| (*file_id, f)))
        })
    }
    pub fn symbols(&self) -> impl Iterator<Item = (SymbolId, &SymbolNode)> + '_ {
        self.symbol_to_index.iter().filter_map(|(symbol_id, &idx)| {
            self.graph
                .node_weight(idx)
                .and_then(|data| data.as_symbol().map(|s| (*symbol_id, s)))
        })
    }
    pub fn edges_of_kind(
        &self,
        kind: EdgeKind,
    ) -> impl Iterator<Item = (NodeIndex, NodeIndex)> + '_ {
        self.graph.edge_indices().filter_map(move |edge_idx| {
            let (source, target) = self.graph.edge_endpoints(edge_idx)?;
            let weight = self.graph.edge_weight(edge_idx)?;
            if weight.kind == kind {
                Some((source, target))
            } else {
                None
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::LangId;
    use crate::model::{LineColumn, SourceRange, Symbol, SymbolKind, Visibility, ids::SnapshotId};
    use std::path::PathBuf;

    fn test_range() -> SourceRange {
        SourceRange {
            byte_start: 0,
            byte_end: 10,
            start: LineColumn { line: 1, column: 0 },
            end: LineColumn {
                line: 1,
                column: 10,
            },
        }
    }

    fn test_symbol(id: u32, name: &str) -> Symbol {
        Symbol {
            id: SymbolId(id),
            name: name.to_string(),
            kind: SymbolKind::Function,
            language: LangId::Rust,
            file_path: PathBuf::from("test.rs"),
            source_range: test_range(),
            visibility: Some(Visibility::Public),
            signature: None,
            docstring: None,
            is_async: false,
        }
    }

    #[test]
    fn code_graph_new_empty() {
        let graph = CodeGraph::new(SnapshotId(1));
        assert_eq!(graph.node_count(), 0);
        assert_eq!(graph.edge_count(), 0);
        assert_eq!(graph.snapshot_id.0, 1);
    }

    #[test]
    fn code_graph_file_lookup_returns_none_for_missing() {
        let graph = CodeGraph::new(SnapshotId(1));
        assert!(graph.file_node(FileId(0)).is_none());
        assert!(graph.file_node_index(FileId(0)).is_none());
    }

    #[test]
    fn code_graph_symbol_lookup_returns_none_for_missing() {
        let graph = CodeGraph::new(SnapshotId(1));
        assert!(graph.symbol_node(SymbolId(0)).is_none());
        assert!(graph.symbol_node_index(SymbolId(0)).is_none());
    }

    #[test]
    fn builder_produces_valid_code_graph() {
        let mut builder = GraphBuilder::new(SnapshotId(1));
        let file_id = builder.add_file(PathBuf::from("test.rs"), LangId::Rust);
        let symbol = test_symbol(1, "main");
        let _sym_idx = builder.add_symbol(&symbol).unwrap();

        let graph = builder.build();

        assert_eq!(graph.file_count(), 1);
        assert_eq!(graph.symbol_count(), 1);
        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 1); // ownership edge

        // Test lookups
        let file_lookup = graph.file_node(file_id);
        assert!(file_lookup.is_some());
        assert_eq!(file_lookup.unwrap().language, LangId::Rust);

        let sym_lookup = graph.symbol_node(SymbolId(1));
        assert!(sym_lookup.is_some());
        assert_eq!(sym_lookup.unwrap().name, "main");
    }

    #[test]
    fn code_graph_iteration_over_files() {
        let mut builder = GraphBuilder::new(SnapshotId(1));
        builder.add_file(PathBuf::from("a.rs"), LangId::Rust);
        builder.add_file(PathBuf::from("b.py"), LangId::Python);

        let graph = builder.build();
        let files: Vec<_> = graph.files().collect();

        assert_eq!(files.len(), 2);
    }

    #[test]
    fn code_graph_iteration_over_symbols() {
        let mut builder = GraphBuilder::new(SnapshotId(1));
        let _file_id = builder.add_file(PathBuf::from("test.rs"), LangId::Rust);
        let sym1 = test_symbol(1, "func_a");
        let sym2 = test_symbol(2, "func_b");
        builder.add_symbol(&sym1).unwrap();
        builder.add_symbol(&sym2).unwrap();

        let graph = builder.build();
        let symbols: Vec<_> = graph.symbols().collect();

        assert_eq!(symbols.len(), 2);
    }

    #[test]
    fn code_graph_edges_of_kind_filtering() {
        let mut builder = GraphBuilder::new(SnapshotId(1));
        let file1 = builder.add_file(PathBuf::from("a.rs"), LangId::Rust);
        let _file2 = builder.add_file(PathBuf::from("b.rs"), LangId::Rust);

        // Add import edge
        builder.add_import(file1, PathBuf::from("b.rs"));

        let graph = builder.build();

        let ownership_edges: Vec<_> = graph.edges_of_kind(EdgeKind::Ownership).collect();
        let import_edges: Vec<_> = graph.edges_of_kind(EdgeKind::Import).collect();

        assert_eq!(ownership_edges.len(), 0); // No symbols added
        assert_eq!(import_edges.len(), 1);
    }

    #[test]
    fn add_edge_normalized_handles_multiple_edge_kinds_between_same_nodes() {
        let mut graph = CodeGraph::new(SnapshotId(1));
        let n1 = graph.graph.add_node(NodeData::File(FileNode {
            id: FileId(1),
            path: PathBuf::from("a.rs"),
            language: LangId::Rust,
            snapshot_id: SnapshotId(1),
        }));
        let n2 = graph.graph.add_node(NodeData::File(FileNode {
            id: FileId(2),
            path: PathBuf::from("b.rs"),
            language: LangId::Rust,
            snapshot_id: SnapshotId(1),
        }));

        // 1. Add Reference edge with confidence 0.7
        graph.add_edge_normalized(n1, n2, EdgeKind::Reference, 0.7);
        // 2. Add Import edge with confidence 0.5 (pushed to head of edge list)
        graph.add_edge_normalized(n1, n2, EdgeKind::Import, 0.5);
        // 3. Add Reference edge again with confidence 0.9 (should max-merge into existing Reference edge)
        graph.add_edge_normalized(n1, n2, EdgeKind::Reference, 0.9);

        let ref_count = graph
            .graph
            .edges_connecting(n1, n2)
            .filter(|e| e.weight().kind == EdgeKind::Reference)
            .count();
        assert_eq!(
            ref_count, 1,
            "Expected 1 Reference edge, but found {ref_count}"
        );
        assert_eq!(graph.edge_count(), 2);
    }
}
