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
use crate::graph::node::{ExternalNode, FileNode, NodeData, SymbolNode};
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

    /// Map from external raw path to graph node index
    external_index: HashMap<String, NodeIndex>,
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
            external_index: HashMap::new(),
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
    pub fn add_symbol(&mut self, symbol: &Symbol) -> Result<NodeIndex, crate::Error> {
        // Check if symbol already exists
        if let Some(&existing) = self.symbol_to_index.get(&symbol.id) {
            return Ok(existing);
        }

        // Look up the file by path
        let file_id = *self
            .path_to_file
            .get(&symbol.file_path)
            .ok_or_else(|| crate::Error::Graph("file must be added before its symbols".into()))?;

        let file_idx = *self
            .file_to_index
            .get(&file_id)
            .ok_or_else(|| crate::Error::Graph("file index must exist".into()))?;

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

        Ok(sym_idx)
    }

    /// Adds an import edge from one file to another.
    ///
    /// If the target file exists in the project, creates an import edge.
    /// If the target is external (not in the project), creates an ExternalNode
    /// placeholder and an import edge to it.
    pub fn add_import(&mut self, from: FileId, to: PathBuf) {
        let Some(&from_idx) = self.file_to_index.get(&from) else {
            return; // Source not in graph
        };

        // Resolve target path to file ID if it exists in our graph
        if let Some(&to_id) = self.path_to_file.get(&to) {
            if let Some(&to_idx) = self.file_to_index.get(&to_id) {
                self.add_edge_internal(from_idx, to_idx, EdgeKind::Import);
            }
            return;
        }

        // External dependency: create or reuse external node
        let raw_path = to.to_string_lossy().to_string();
        let to_idx = if let Some(&idx) = self.external_index.get(&raw_path) {
            idx
        } else {
            // Determine language from source file
            let language = self
                .path_to_file
                .iter()
                .find(|(_, id)| **id == from)
                .map(|(p, _)| {
                    crate::input::detect_language(p).unwrap_or(crate::language::LangId::Python)
                })
                .unwrap_or(crate::language::LangId::Python);

            let node = ExternalNode {
                raw_path: raw_path.clone(),
                language,
            };
            let idx = self.graph.add_node(NodeData::External(node));
            self.external_index.insert(raw_path, idx);
            idx
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
            external_index: self.external_index,
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

    /// Returns the number of external dependency nodes.
    pub fn external_count(&self) -> usize {
        self.external_index.len()
    }

    /// Assemble a complete CodeGraph + SCC from parallel extraction results.
    ///
    /// Replaces the manual multi-step wiring in pipeline.rs. Handles:
    /// - File and symbol node registration
    /// - Import edge resolution (via language-specific resolvers)
    /// - Cross-file reference resolution via FlattenedScopeCache
    /// - SCC analysis
    ///
    /// Errors during symbol addition are non-fatal and appended to `diagnostics`.
    pub fn from_extractions(
        extractions: &[crate::model::FileExtraction],
        root: &std::path::Path,
        snapshot_id: crate::model::SnapshotId,
        diagnostics: &mut Vec<crate::error::Diagnostic>,
    ) -> (CodeGraph, crate::graph::SccAnalysis) {
        let mut builder = Self::new(snapshot_id);

        // Phase 1: register all files
        for file in extractions {
            builder.add_file(file.path.clone(), file.lang);
        }

        // Phase 2: register all symbols; symbol errors are non-fatal
        for file in extractions {
            for symbol in &file.symbols {
                if let Err(e) = builder.add_symbol(symbol) {
                    diagnostics.push(crate::error::Diagnostic {
                        path: file.path.clone(),
                        severity: crate::error::Severity::Warning,
                        message: format!("failed to add symbol to graph: {e}"),
                        source_range: None,
                    });
                }
            }
        }

        // Phase 3: build path -> FileId map needed by the resolver
        let path_to_file_id: HashMap<std::path::PathBuf, crate::model::FileId> = extractions
            .iter()
            .filter_map(|f| {
                builder
                    .file_id_for_path(&f.path)
                    .map(|fid| (f.path.clone(), fid))
            })
            .collect();

        // Phase 4: resolve language-specific import paths and add import edges
        for file in extractions {
            let Some(&source_fid) = path_to_file_id.get(&file.path) else {
                continue;
            };
            let source_dir = file.path.parent().unwrap_or(std::path::Path::new("."));
            let resolver_fn = file.lang.spec().import_path_resolver;
            for import in &file.imports {
                if let Some(target) = resolver_fn(&import.namespace, source_dir, root) {
                    builder.add_import(source_fid, target);
                }
            }
        }

        // Phase 5: cross-file reference resolution via FlattenedScopeCache
        let import_adjacency = builder.import_adjacency();
        let ctx = crate::graph::resolver::ResolutionContext::from_extractions(
            extractions,
            &path_to_file_id,
            import_adjacency,
        );
        let scope_cache = crate::graph::resolver::FlattenedScopeCache::build(&ctx, diagnostics);
        let ref_edges = crate::graph::resolver::resolve_all_references(
            extractions,
            &path_to_file_id,
            &scope_cache,
            diagnostics,
        );
        for (from, to) in ref_edges {
            builder.add_reference(from, to);
        }

        // Phase 6: finalize and compute SCC
        let graph = builder.build();
        let scc = crate::graph::SccAnalysis::analyze(&graph.graph);

        (graph, scc)
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

        let _sym_idx = builder.add_symbol(&symbol).unwrap();

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
    fn builder_creates_external_node_for_unknown_import() {
        let mut builder = GraphBuilder::new(SnapshotId(1));
        let path = PathBuf::from("main.py");

        let file_id = builder.add_file(path, LangId::Python);

        // Import of external module not in our project
        builder.add_import(file_id, PathBuf::from("external_module.py"));

        // Should create an external node and import edge
        assert_eq!(
            builder.edge_count(),
            1,
            "external import should create an edge"
        );
        assert_eq!(builder.external_count(), 1, "should have one external node");
    }

    #[test]
    fn builder_tracks_node_mappings() {
        let mut builder = GraphBuilder::new(SnapshotId(1));
        let path = PathBuf::from("test.py");

        let file_id = builder.add_file(path, LangId::Python);
        let symbol = test_symbol(42, "func", file_id.0);

        builder.add_symbol(&symbol).unwrap();

        // Verify mappings exist
        assert!(builder.file_to_index.contains_key(&file_id));
        assert!(builder.symbol_to_index.contains_key(&SymbolId(42)));
    }

    #[test]
    fn from_extractions_builds_graph_with_correct_node_count() {
        use crate::model::{FileExtraction, LineColumn, SourceRange, Symbol, SymbolId, SymbolKind};
        use std::path::PathBuf;
        let sym = Symbol {
            id: SymbolId(1),
            name: "foo".into(),
            kind: SymbolKind::Function,
            language: LangId::Python,
            file_path: PathBuf::from("/proj/a.py"),
            source_range: SourceRange {
                byte_start: 0,
                byte_end: 10,
                start: LineColumn { line: 0, column: 0 },
                end: LineColumn {
                    line: 0,
                    column: 10,
                },
            },
            visibility: None,
            signature: None,
            docstring: None,
            is_async: false,
        };
        let extractions = vec![FileExtraction {
            path: PathBuf::from("/proj/a.py"),
            lang: LangId::Python,
            symbols: vec![sym],
            imports: vec![],
            references: vec![],
            diagnostics: vec![],
        }];
        let mut diags = Vec::new();
        let (graph, _scc) = GraphBuilder::from_extractions(
            &extractions,
            std::path::Path::new("/proj"),
            SnapshotId(1),
            &mut diags,
        );
        assert_eq!(graph.file_count(), 1);
        assert_eq!(graph.symbol_count(), 1);
        assert!(diags.is_empty());
    }

    #[test]
    fn from_extractions_populates_scc_analysis() {
        use crate::model::FileExtraction;
        let extractions: Vec<FileExtraction> = vec![];
        let mut diags = Vec::new();
        let (graph, scc) = GraphBuilder::from_extractions(
            &extractions,
            std::path::Path::new("/proj"),
            SnapshotId(1),
            &mut diags,
        );
        assert_eq!(graph.node_count(), 0);
        assert!(!scc.components.iter().any(|c| c.is_cyclic));
    }

    #[test]
    fn from_extractions_accumulates_diagnostics_on_symbol_error() {
        use crate::model::{FileExtraction, LineColumn, SourceRange, Symbol, SymbolId, SymbolKind};
        use std::path::PathBuf;
        let sym = Symbol {
            id: SymbolId(99),
            name: "orphan".into(),
            kind: SymbolKind::Function,
            language: LangId::Python,
            file_path: PathBuf::from("/proj/missing.py"),
            source_range: SourceRange {
                byte_start: 0,
                byte_end: 5,
                start: LineColumn { line: 0, column: 0 },
                end: LineColumn { line: 0, column: 5 },
            },
            visibility: None,
            signature: None,
            docstring: None,
            is_async: false,
        };
        let extractions = vec![FileExtraction {
            path: PathBuf::from("/proj/a.py"),
            lang: LangId::Python,
            symbols: vec![sym],
            imports: vec![],
            references: vec![],
            diagnostics: vec![],
        }];
        let mut diags = Vec::new();
        let (_graph, _scc) = GraphBuilder::from_extractions(
            &extractions,
            std::path::Path::new("/proj"),
            SnapshotId(1),
            &mut diags,
        );
        assert!(!diags.is_empty(), "expected diagnostic for orphan symbol");
    }

    #[test]
    fn from_extractions_resolves_cross_file_imports() {
        use crate::model::{FileExtraction, LineColumn, SourceRange, UnresolvedImport};
        use std::path::PathBuf;
        let extractions = vec![
            FileExtraction {
                path: PathBuf::from("/proj/a.py"),
                lang: LangId::Python,
                symbols: vec![],
                imports: vec![UnresolvedImport {
                    namespace: "b".into(),
                    alias: None,
                    symbol: None,
                    star: false,
                    range: SourceRange {
                        byte_start: 0,
                        byte_end: 1,
                        start: LineColumn { line: 0, column: 0 },
                        end: LineColumn { line: 0, column: 1 },
                    },
                }],
                references: vec![],
                diagnostics: vec![],
            },
            FileExtraction {
                path: PathBuf::from("/proj/b.py"),
                lang: LangId::Python,
                symbols: vec![],
                imports: vec![],
                references: vec![],
                diagnostics: vec![],
            },
        ];
        let mut diags = Vec::new();
        let (graph, _scc) = GraphBuilder::from_extractions(
            &extractions,
            std::path::Path::new("/proj"),
            SnapshotId(1),
            &mut diags,
        );
        assert_eq!(graph.file_count(), 2);
        let import_edges: Vec<_> = graph
            .edges_of_kind(crate::graph::EdgeKind::Import)
            .collect();
        assert_eq!(
            import_edges.len(),
            1,
            "expected import edge from a.py to b.py"
        );
    }
}
