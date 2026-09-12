//! Graph builder for incremental construction from extraction results.
//!
//! Construction proceeds in stages:
//! 1. Ownership graph: files and their symbols
//! 2. Dataflow nodes and edges (requires `--features dataflow`)
//! 3. Dependency graph: imports and cross-file references
//!
//! The builder maintains index mappings from domain IDs (FileId, SymbolId)
//! to petgraph `NodeIndex` for efficient lookups.

use std::collections::HashMap;
use std::path::PathBuf;

use petgraph::graph::{DiGraph, EdgeIndex, NodeIndex};
use petgraph::visit::EdgeRef;

use crate::graph::CodeGraph;
use crate::graph::edge::{CONFIDENCE_CROSS_LANGUAGE, CONFIDENCE_OWN_OR_DIRECT, EdgeData, EdgeKind};
#[cfg(feature = "dataflow")]
use crate::graph::node::DataGraphNode;
use crate::graph::node::{ExternalNode, FileNode, NodeData, SymbolNode};
use crate::language::LangId;
#[cfg(feature = "dataflow")]
use crate::model::DataNodeId;
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

    /// O(1) dedup index mirroring `CodeGraph::edge_index`.
    edge_index: HashMap<(NodeIndex, NodeIndex, EdgeKind), EdgeIndex>,

    /// Map from external raw path to graph node index
    external_index: HashMap<String, NodeIndex>,

    /// Map from DataNodeId to graph node index
    #[cfg(feature = "dataflow")]
    data_to_index: HashMap<DataNodeId, NodeIndex>,
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
            edge_index: HashMap::new(),
            external_index: HashMap::new(),
            #[cfg(feature = "dataflow")]
            data_to_index: HashMap::new(),
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
    /// The containing file must already exist in the builder. The symbol's
    /// file path is resolved against registered files.
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

        // Add ownership edge: file -> symbol (always full confidence)
        self.add_edge_internal(file_idx, sym_idx, EdgeKind::Ownership, 1.0);

        Ok(sym_idx)
    }

    /// Adds a data-bearing node to the graph.
    ///
    /// Returns the NodeIndex for the data node. Idempotent: returns
    /// the existing index if a data node with the same DataNodeId exists.
    ///
    /// When the data node has a `symbol_id` that resolves to an existing
    /// SymbolNode in the graph, an Ownership edge is created from the
    /// symbol to the data node.
    #[cfg(feature = "dataflow")]
    pub fn add_data_node(&mut self, data_node: &crate::model::DataNode) -> NodeIndex {
        if let Some(&existing) = self.data_to_index.get(&data_node.id) {
            return existing;
        }
        let node = DataGraphNode {
            id: data_node.id,
            symbol_id: data_node.symbol_id,
            name: data_node.name.clone(),
            scope: data_node.scope,
            type_hint: data_node.type_hint.clone(),
            source_range: data_node.source_range.clone(),
        };
        let idx = self.graph.add_node(NodeData::Data(node));
        self.data_to_index.insert(data_node.id, idx);

        if let Some(symbol_id) = data_node.symbol_id
            && let Some(&sym_idx) = self.symbol_to_index.get(&symbol_id)
        {
            self.add_edge_internal(sym_idx, idx, EdgeKind::Ownership, 1.0);
        }

        idx
    }

    /// Adds a flow edge between two data nodes.
    #[cfg(feature = "dataflow")]
    pub fn add_flow_edge(
        &mut self,
        source: crate::model::DataNodeId,
        target: crate::model::DataNodeId,
        kind: crate::model::FlowKind,
        confidence: f32,
    ) {
        let Some(&src_idx) = self.data_to_index.get(&source) else {
            return;
        };
        let Some(&tgt_idx) = self.data_to_index.get(&target) else {
            return;
        };
        self.add_edge_internal_with_flow(src_idx, tgt_idx, EdgeKind::Flow, confidence, Some(kind));
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
        let Some(source_language) = self.graph[from_idx].as_file().map(|file| file.language) else {
            return; // Index without a file node: the graph is inconsistent
        };

        // Resolve target path to file ID if it exists in our graph
        if let Some(&to_id) = self.path_to_file.get(&to) {
            if let Some(&to_idx) = self.file_to_index.get(&to_id) {
                let target_language = self.graph[to_idx].as_file().map(|file| file.language);
                let confidence = if target_language == Some(source_language) {
                    CONFIDENCE_OWN_OR_DIRECT
                } else {
                    CONFIDENCE_CROSS_LANGUAGE
                };
                self.add_edge_internal(from_idx, to_idx, EdgeKind::Import, confidence);
            }
            return;
        }

        // External dependency: create or reuse external node
        let raw_path = to.to_string_lossy().to_string();
        let to_idx = if let Some(&idx) = self.external_index.get(&raw_path) {
            idx
        } else {
            let node = ExternalNode {
                raw_path: raw_path.clone(),
                language: source_language,
                classification: None,
            };
            let idx = self.graph.add_node(NodeData::External(node));
            self.external_index.insert(raw_path, idx);
            idx
        };

        self.add_edge_internal(from_idx, to_idx, EdgeKind::Import, CONFIDENCE_OWN_OR_DIRECT);
    }

    /// Internal edge addition with flow kind, respecting normalization.
    #[cfg(feature = "dataflow")]
    fn add_edge_internal_with_flow(
        &mut self,
        source: NodeIndex,
        target: NodeIndex,
        kind: EdgeKind,
        confidence: f32,
        flow_kind: Option<crate::model::FlowKind>,
    ) {
        self.insert_edge(source, target, kind, confidence, flow_kind);
    }

    /// Adds an edge through the one normalization rule: a repeated triple
    /// merges into the existing edge.
    fn insert_edge(
        &mut self,
        source: NodeIndex,
        target: NodeIndex,
        kind: EdgeKind,
        confidence: f32,
        flow_kind: Option<crate::model::FlowKind>,
    ) {
        let confidence = confidence.clamp(0.0, 1.0);
        let key = (source, target, kind);
        if let Some(&edge_idx) = self.edge_index.get(&key) {
            self.graph[edge_idx].merge_repeated(confidence, flow_kind);
            return;
        }
        let mut edge_data = EdgeData::with_confidence(kind, confidence);
        edge_data.flow_kind = flow_kind;
        let edge_idx = self.graph.add_edge(source, target, edge_data);
        self.edge_index.insert(key, edge_idx);
    }

    /// Adds a reference edge between two symbols with a confidence score.
    ///
    /// Both symbols must exist in the builder. If a duplicate edge
    /// already exists, the confidence is merged via max (the stronger
    /// signal is preserved).
    pub fn add_reference(&mut self, from: SymbolId, to: SymbolId, confidence: f32) {
        let Some(&from_idx) = self.symbol_to_index.get(&from) else {
            return;
        };
        let Some(&to_idx) = self.symbol_to_index.get(&to) else {
            return;
        };

        self.add_edge_internal(from_idx, to_idx, EdgeKind::Reference, confidence);
    }

    /// Internal method to add an edge with deduplication and confidence.
    ///
    /// If a duplicate `(source, target, kind)` already exists, the existing
    /// edge's confidence is bumped to `max(existing, new)` - never reduced.
    fn add_edge_internal(
        &mut self,
        source: NodeIndex,
        target: NodeIndex,
        kind: EdgeKind,
        confidence: f32,
    ) {
        self.insert_edge(source, target, kind, confidence, None);
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
        CodeGraph::from_parts(
            self.graph,
            self.edge_index,
            self.file_to_index,
            self.symbol_to_index,
            self.external_index,
            self.snapshot_id,
        )
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
    pub fn from_extractions<F>(
        extractions: &[F],
        root: &std::path::Path,
        snapshot_id: crate::model::SnapshotId,
        diagnostics: &mut Vec<crate::error::Diagnostic>,
    ) -> (CodeGraph, crate::graph::SccAnalysis)
    where
        F: std::borrow::Borrow<crate::model::FileExtraction> + Sync,
    {
        let (graph, scc, _) =
            Self::from_extractions_with_scope(extractions, root, snapshot_id, diagnostics);
        (graph, scc)
    }

    /// Build the graph, SCC, and the flattened scope cache.
    pub fn from_extractions_with_scope<F>(
        extractions: &[F],
        root: &std::path::Path,
        snapshot_id: crate::model::SnapshotId,
        diagnostics: &mut Vec<crate::error::Diagnostic>,
    ) -> (
        CodeGraph,
        crate::graph::SccAnalysis,
        crate::graph::resolver::FlattenedScopeCache,
    )
    where
        F: std::borrow::Borrow<crate::model::FileExtraction> + Sync,
    {
        let mut builder = Self::new(snapshot_id);

        register_files(&mut builder, extractions);
        register_symbols(&mut builder, extractions, diagnostics);
        #[cfg(feature = "dataflow")]
        register_dataflow(&mut builder, extractions);

        let path_to_file_id = path_to_file_id(&builder, extractions);
        resolve_imports(
            &mut builder,
            extractions,
            root,
            &path_to_file_id,
            diagnostics,
        );
        let scope_cache =
            resolve_references(&mut builder, extractions, &path_to_file_id, diagnostics);

        let graph = builder.build();
        #[cfg(feature = "metacall-deploy")]
        let mut graph = graph;
        #[cfg(feature = "metacall-deploy")]
        inject_client_call_edges(&mut graph, extractions, root, diagnostics);

        let scc = crate::graph::SccAnalysis::analyze(graph.graph());

        (graph, scc, scope_cache)
    }
}
/// Registers every file as a node.
fn register_files<F>(builder: &mut GraphBuilder, extractions: &[F])
where
    F: std::borrow::Borrow<crate::model::FileExtraction> + Sync,
{
    for file in extractions {
        let file = file.borrow();
        builder.add_file(file.path.clone(), file.lang);
    }
}

/// Registers every symbol and its ownership edge.
///
/// A symbol whose file was not registered is a warning: the extraction order
/// broke, and the rest of the graph is still usable.
fn register_symbols<F>(
    builder: &mut GraphBuilder,
    extractions: &[F],
    diagnostics: &mut Vec<crate::error::Diagnostic>,
) where
    F: std::borrow::Borrow<crate::model::FileExtraction> + Sync,
{
    for file in extractions {
        let file = file.borrow();
        for symbol in &file.symbols {
            if let Err(error) = builder.add_symbol(symbol) {
                diagnostics.push(crate::error::Diagnostic {
                    path: file.path.clone(),
                    severity: crate::error::Severity::Warning,
                    message: format!("failed to add symbol to graph: {error}"),
                    source_range: None,
                });
            }
        }
    }
}

/// Registers data nodes first, then the flow edges that reference them.
#[cfg(feature = "dataflow")]
fn register_dataflow<F>(builder: &mut GraphBuilder, extractions: &[F])
where
    F: std::borrow::Borrow<crate::model::FileExtraction> + Sync,
{
    for file in extractions {
        let file = file.borrow();
        for data_node in &file.data_nodes {
            builder.add_data_node(data_node);
        }
    }
    for file in extractions {
        let file = file.borrow();
        for flow_edge in &file.flow_edges {
            builder.add_flow_edge(
                flow_edge.source,
                flow_edge.target,
                flow_edge.kind,
                flow_edge.confidence,
            );
        }
    }
}

/// Maps every registered file back to its identifier, for the resolver.
fn path_to_file_id<F>(
    builder: &GraphBuilder,
    extractions: &[F],
) -> HashMap<std::path::PathBuf, crate::model::FileId>
where
    F: std::borrow::Borrow<crate::model::FileExtraction> + Sync,
{
    extractions
        .iter()
        .filter_map(|file| {
            let file = file.borrow();
            builder
                .file_id_for_path(&file.path)
                .map(|id| (file.path.clone(), id))
        })
        .collect()
}

/// Resolves every import specifier and adds the import edges.
///
/// A specifier no resolver resolves becomes an external node, unless it is
/// relative: a relative import that does not resolve is a warning, because the
/// file it names is missing from the tree.
fn resolve_imports<F>(
    builder: &mut GraphBuilder,
    extractions: &[F],
    root: &std::path::Path,
    path_to_file_id: &HashMap<std::path::PathBuf, crate::model::FileId>,
    diagnostics: &mut Vec<crate::error::Diagnostic>,
) where
    F: std::borrow::Borrow<crate::model::FileExtraction> + Sync,
{
    let mut resolvers = HashMap::new();
    for lang in crate::language::LangId::all() {
        resolvers.insert(lang, crate::language::import_resolver::make_resolver(lang));
    }

    for file in extractions {
        let file = file.borrow();
        let Some(&source_id) = path_to_file_id.get(&file.path) else {
            continue;
        };
        let source_dir = file
            .path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        let Some(resolver) = resolvers.get(&file.lang) else {
            continue;
        };
        for import in &file.imports {
            let specifier = import_specifier_for(file.lang, import);
            match resolver.resolve(&specifier, source_dir, root) {
                Some(target) => builder.add_import(source_id, target),
                None if is_relative_specifier(&specifier) => {
                    diagnostics.push(crate::error::Diagnostic {
                        path: file.path.clone(),
                        severity: crate::error::Severity::Warning,
                        message: format!("unresolved relative import: {specifier}"),
                        source_range: Some(import.range.clone()),
                    });
                }
                None => builder.add_import(
                    source_id,
                    std::path::PathBuf::from(external_name(file.lang, &specifier)),
                ),
            }
        }
    }
}

/// Resolves cross-file references and returns the scope cache they were
/// resolved against.
fn resolve_references<F>(
    builder: &mut GraphBuilder,
    extractions: &[F],
    path_to_file_id: &HashMap<std::path::PathBuf, crate::model::FileId>,
    diagnostics: &mut Vec<crate::error::Diagnostic>,
) -> crate::graph::resolver::FlattenedScopeCache
where
    F: std::borrow::Borrow<crate::model::FileExtraction> + Sync,
{
    let import_adjacency = builder.import_adjacency();
    let context = crate::graph::resolver::ResolutionContext::from_extractions(
        extractions,
        path_to_file_id,
        import_adjacency,
    );
    let scope_cache = crate::graph::resolver::FlattenedScopeCache::build(&context, diagnostics);
    let reference_edges = crate::graph::resolver::resolve_all_references(
        extractions,
        path_to_file_id,
        &scope_cache,
        diagnostics,
    );
    for (from, to, confidence) in reference_edges {
        builder.add_reference(from, to, confidence);
    }
    scope_cache
}

/// Adds the client-call projections as ordinary reference edges.
///
/// Resolution needs the file and symbol nodes, so this runs after the graph is
/// built. Navigation and SCC see these edges like any other reference.
#[cfg(feature = "metacall-deploy")]
fn inject_client_call_edges<F>(
    graph: &mut CodeGraph,
    extractions: &[F],
    root: &std::path::Path,
    diagnostics: &mut Vec<crate::error::Diagnostic>,
) where
    F: std::borrow::Borrow<crate::model::FileExtraction> + Sync,
{
    let call_sites: Vec<crate::deploy::scanner::CallSite> = extractions
        .iter()
        .flat_map(|file| file.borrow().call_sites.iter().cloned())
        .collect();
    if call_sites.is_empty() {
        return;
    }
    let projections = crate::deploy::client_call::resolve_client_call_projections(
        graph,
        extractions,
        &call_sites,
        root,
    );
    diagnostics.extend(projections.diagnostics);
    for (from, to, confidence) in projections.file_edges {
        graph.add_edge_normalized(from, to, EdgeKind::Reference, confidence);
    }
    for (from, to, confidence) in projections.symbol_edges {
        if let (Some(from_idx), Some(to_idx)) =
            (graph.symbol_node_index(from), graph.symbol_node_index(to))
        {
            graph.add_edge_normalized(from_idx, to_idx, EdgeKind::Reference, confidence);
        }
    }
}

/// The specifier the resolver sees.
///
/// Python spells a bare relative import (`from . import x`) as a package plus
/// a name, so the name joins the module path before resolution.
fn import_specifier_for<'a>(
    lang: crate::language::LangId,
    import: &'a crate::model::UnresolvedImport,
) -> std::borrow::Cow<'a, str> {
    if lang == crate::language::LangId::Python {
        crate::language::python::normalize_relative_specifier(
            &import.import_specifier,
            import.symbol.as_deref(),
            import.star,
        )
    } else {
        std::borrow::Cow::Borrowed(import.import_specifier.as_str())
    }
}

/// A relative specifier addresses the project, so a miss is a config error.
fn is_relative_specifier(specifier: &str) -> bool {
    let stripped = specifier.trim_matches(|c| c == '\'' || c == '"');
    stripped.starts_with('.') || stripped.starts_with('/')
}

/// Node name for an unresolved non-relative import (ADR 0003).
fn external_name(lang: crate::language::LangId, specifier: &str) -> String {
    use crate::language::{LangId, import_resolver};
    let name = match lang {
        LangId::C | LangId::Cpp => import_resolver::strip_c_family_quotes(specifier),
        _ => import_resolver::strip_import_quotes(specifier),
    };
    name.to_string()
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
            id: SymbolId::new(id).unwrap(),
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
        let mut builder = GraphBuilder::new(SnapshotId::new(1).unwrap());
        let path = PathBuf::from("src/main.py");

        let id1 = builder.add_file(path.clone(), LangId::Python);
        let id2 = builder.add_file(path, LangId::Python);

        assert_eq!(id1, id2, "same path should return same FileId");
        assert_eq!(builder.node_count(), 1);
    }

    #[test]
    fn builder_creates_symbol_with_ownership() {
        let mut builder = GraphBuilder::new(SnapshotId::new(1).unwrap());
        let path = PathBuf::from("test.py");

        let file_id = builder.add_file(path.clone(), LangId::Python);
        let symbol = test_symbol(1, "hello", file_id.to_raw());

        let _sym_idx = builder.add_symbol(&symbol).unwrap();

        assert_eq!(builder.node_count(), 2); // file + symbol
        assert_eq!(builder.edge_count(), 1); // ownership edge
    }

    #[test]
    fn builder_deduplicates_edges() {
        let mut builder = GraphBuilder::new(SnapshotId::new(1).unwrap());
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
    fn builder_reference_max_merges_confidence() {
        let mut builder = GraphBuilder::new(SnapshotId::new(1).unwrap());
        builder.add_file(PathBuf::from("test.py"), LangId::Python);
        let low = test_symbol(10, "low_fn", 0);
        let high = test_symbol(11, "high_fn", 0);
        builder.add_symbol(&low).unwrap();
        builder.add_symbol(&high).unwrap();

        builder.add_reference(low.id, high.id, 0.5);
        builder.add_reference(low.id, high.id, 0.9);
        assert_eq!(builder.edge_count(), 3);

        let graph = builder.build();
        assert_eq!(graph.edge_count(), 3);
        let refs: Vec<_> = graph.edges_of_kind(EdgeKind::Reference).collect();
        assert_eq!(refs.len(), 1);
        let (src, dst) = refs[0];
        let edge = graph.graph().edges_connecting(src, dst).next().unwrap();
        assert_eq!(edge.weight().confidence, 0.9);
    }

    #[test]
    fn builder_creates_external_node_for_unknown_import() {
        let mut builder = GraphBuilder::new(SnapshotId::new(1).unwrap());
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
        let mut builder = GraphBuilder::new(SnapshotId::new(1).unwrap());
        let path = PathBuf::from("test.py");

        let file_id = builder.add_file(path, LangId::Python);
        let symbol = test_symbol(42, "func", file_id.to_raw());

        builder.add_symbol(&symbol).unwrap();

        // Verify mappings exist
        assert!(builder.file_to_index.contains_key(&file_id));
        assert!(
            builder
                .symbol_to_index
                .contains_key(&SymbolId::new(42).unwrap())
        );
    }

    #[test]
    fn from_extractions_builds_graph_with_correct_node_count() {
        use crate::model::{FileExtraction, LineColumn, SourceRange, Symbol, SymbolId, SymbolKind};
        use std::path::PathBuf;
        let sym = Symbol {
            id: SymbolId::new(1).unwrap(),
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
        let mut base = FileExtraction::empty(PathBuf::from("/proj/a.py"), LangId::Python);
        base.symbols = vec![sym];
        let extractions = vec![base];
        let mut diags = Vec::new();
        let (graph, _scc) = GraphBuilder::from_extractions(
            &extractions,
            std::path::Path::new("/proj"),
            SnapshotId::new(1).unwrap(),
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
            SnapshotId::new(1).unwrap(),
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
            id: SymbolId::new(99).unwrap(),
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
        let mut base = FileExtraction::empty(PathBuf::from("/proj/a.py"), LangId::Python);
        base.symbols = vec![sym];
        let extractions = vec![base];
        let mut diags = Vec::new();
        let (_graph, _scc) = GraphBuilder::from_extractions(
            &extractions,
            std::path::Path::new("/proj"),
            SnapshotId::new(1).unwrap(),
            &mut diags,
        );
        assert!(!diags.is_empty(), "expected diagnostic for orphan symbol");
    }

    #[test]
    fn from_extractions_resolves_cross_file_imports() {
        use crate::model::{FileExtraction, LineColumn, SourceRange, UnresolvedImport};
        use std::path::PathBuf;
        let mut first = FileExtraction::empty(PathBuf::from("/proj/a.py"), LangId::Python);
        first.imports = vec![UnresolvedImport {
            import_specifier: "b".into(),
            alias: None,
            symbol: None,
            star: false,
            range: SourceRange {
                byte_start: 0,
                byte_end: 1,
                start: LineColumn { line: 0, column: 0 },
                end: LineColumn { line: 0, column: 1 },
            },
        }];
        let second = FileExtraction::empty(PathBuf::from("/proj/b.py"), LangId::Python);
        let extractions = vec![first, second];
        let mut diags = Vec::new();
        let (graph, _scc) = GraphBuilder::from_extractions(
            &extractions,
            std::path::Path::new("/proj"),
            SnapshotId::new(1).unwrap(),
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

    #[cfg(feature = "dataflow")]
    #[test]
    fn add_data_node_creates_ownership_edge_to_symbol() {
        use crate::model::{DataNode, DataNodeId, DataScope};
        let mut builder = GraphBuilder::new(SnapshotId::new(1).unwrap());
        let file_path = PathBuf::from("test.rs");
        let _file_id = builder.add_file(file_path.clone(), LangId::Rust);
        let sym = Symbol {
            id: SymbolId::new(1).unwrap(),
            name: "my_fn".into(),
            kind: SymbolKind::Function,
            language: LangId::Rust,
            file_path,
            source_range: test_source_range(),
            visibility: Some(Visibility::Public),
            signature: None,
            docstring: None,
            is_async: false,
        };
        let sym_idx = builder.add_symbol(&sym).unwrap();

        let data = DataNode {
            id: DataNodeId::new(1).unwrap(),
            symbol_id: Some(SymbolId::new(1).unwrap()),
            name: Some("x".into()),
            scope: DataScope::Local,
            type_hint: Some("i32".into()),
            source_range: test_source_range(),
        };
        builder.add_data_node(&data);

        let graph = builder.build();
        let ownership_edges: Vec<_> = graph.edges_of_kind(EdgeKind::Ownership).collect();
        assert_eq!(
            ownership_edges.len(),
            2,
            "expected file→symbol and symbol→data ownership edges"
        );

        let data_ownership_edges: Vec<_> = ownership_edges
            .into_iter()
            .filter(|(src, _)| *src == sym_idx)
            .collect();
        assert!(
            !data_ownership_edges.is_empty(),
            "expected symbol→data ownership edge"
        );
    }
    /// Stage: registration. Files first, then each symbol owns its file node.
    #[test]
    fn registration_stage_adds_files_and_symbol_ownership() {
        use crate::model::FileExtraction;
        let mut file = FileExtraction::empty(PathBuf::from("/proj/a.py"), LangId::Python);
        file.symbols = vec![symbol_for(1, "alpha", std::path::Path::new("/proj/a.py"))];
        let extractions = vec![file];

        let mut builder = GraphBuilder::new(SnapshotId::new(1).unwrap());
        let mut diagnostics = Vec::new();
        register_files(&mut builder, &extractions);
        register_symbols(&mut builder, &extractions, &mut diagnostics);

        assert!(diagnostics.is_empty());
        assert_eq!(builder.node_count(), 2, "one file node and one symbol node");
        assert_eq!(builder.edge_count(), 1, "the symbol owns its file");
    }

    /// Stage: dataflow. Nodes are registered before the edges that join them.
    #[cfg(feature = "dataflow")]
    #[test]
    fn dataflow_stage_registers_nodes_and_flow_edges() {
        use crate::model::{DataNodeId, FileExtraction, FlowEdge, FlowKind};
        let mut file = FileExtraction::empty(PathBuf::from("/proj/a.py"), LangId::Python);
        file.symbols = vec![symbol_for(1, "alpha", std::path::Path::new("/proj/a.py"))];
        file.data_nodes = vec![data_node(1, "x"), data_node(2, "y")];
        file.flow_edges = vec![FlowEdge {
            source: DataNodeId::new(1).unwrap(),
            target: DataNodeId::new(2).unwrap(),
            kind: FlowKind::Argument,
            confidence: 0.8,
        }];
        let extractions = vec![file];

        let mut builder = GraphBuilder::new(SnapshotId::new(1).unwrap());
        register_files(&mut builder, &extractions);
        register_symbols(&mut builder, &extractions, &mut Vec::new());
        register_dataflow(&mut builder, &extractions);

        assert_eq!(builder.node_count(), 4, "file, symbol and two data nodes");
        let graph = builder.build();
        assert_eq!(graph.edges_of_kind(EdgeKind::Flow).count(), 1);
    }

    /// Stage: imports. A project import joins two files, an unresolved
    /// specifier becomes an external node.
    #[test]
    fn import_stage_resolves_project_and_external_targets() {
        use crate::model::FileExtraction;
        // The Python resolver checks the file system, so the project is real.
        let project = tempfile::tempdir().unwrap();
        let root = project.path();
        std::fs::write(root.join("a.py"), "import b\nimport requests\n").unwrap();
        std::fs::write(root.join("b.py"), "def beta(): pass\n").unwrap();

        let mut first = FileExtraction::empty(root.join("a.py"), LangId::Python);
        first.imports = vec![import("b"), import("requests")];
        let second = FileExtraction::empty(root.join("b.py"), LangId::Python);
        let extractions = vec![first, second];

        let mut builder = GraphBuilder::new(SnapshotId::new(1).unwrap());
        register_files(&mut builder, &extractions);
        let file_ids = path_to_file_id(&builder, &extractions);
        let mut diagnostics = Vec::new();
        resolve_imports(
            &mut builder,
            &extractions,
            root,
            &file_ids,
            &mut diagnostics,
        );

        assert!(diagnostics.is_empty(), "both imports resolve");
        let graph = builder.build();
        assert_eq!(graph.edges_of_kind(EdgeKind::Import).count(), 2);
        assert_eq!(graph.external_count(), 1, "requests is an external node");
    }

    /// Stage: references. The returned cache resolves the imported name, and
    /// the edge reaches the graph.
    #[test]
    fn reference_stage_returns_a_usable_scope_cache() {
        use crate::model::{FileExtraction, UnresolvedReference};
        let project = tempfile::tempdir().unwrap();
        let root = project.path();
        let first_path = root.join("a.py");
        let second_path = root.join("b.py");
        std::fs::write(&first_path, "def alpha(): pass\n").unwrap();
        std::fs::write(&second_path, "import a\ndef beta(): return alpha()\n").unwrap();

        let mut first = FileExtraction::empty(first_path.clone(), LangId::Python);
        first.symbols = vec![symbol_for(1, "alpha", &first_path)];
        let mut second = FileExtraction::empty(second_path.clone(), LangId::Python);
        second.symbols = vec![symbol_for(2, "beta", &second_path)];
        second.imports = vec![import("a")];
        second.references = vec![UnresolvedReference {
            name: "alpha".to_string(),
            range: test_source_range(),
        }];
        let extractions = vec![first, second];

        let mut builder = GraphBuilder::new(SnapshotId::new(1).unwrap());
        register_files(&mut builder, &extractions);
        register_symbols(&mut builder, &extractions, &mut Vec::new());
        let file_ids = path_to_file_id(&builder, &extractions);
        let mut diagnostics = Vec::new();
        resolve_imports(
            &mut builder,
            &extractions,
            root,
            &file_ids,
            &mut diagnostics,
        );
        let scope = resolve_references(&mut builder, &extractions, &file_ids, &mut diagnostics);

        assert_eq!(scope.iter_scopes().count(), 2);
        let graph = builder.build();
        assert_eq!(
            graph.edges_of_kind(EdgeKind::Reference).count(),
            1,
            "beta references alpha"
        );
    }

    /// Stage: client calls. Resolution runs after the graph exists, and the
    /// call projects onto the symbol that encloses it.
    #[cfg(feature = "metacall-deploy")]
    #[test]
    fn client_call_stage_projects_the_call_onto_its_symbol() {
        use crate::deploy::scanner::CallSite;
        use crate::model::FileExtraction;
        let mut caller =
            FileExtraction::empty(PathBuf::from("/proj/orchestrator.py"), LangId::Python);
        caller.symbols = vec![symbol_for(
            1,
            "compute",
            std::path::Path::new("/proj/orchestrator.py"),
        )];
        caller.call_sites = vec![CallSite::call(
            PathBuf::from("/proj/orchestrator.py"),
            LangId::Python,
            "multiply".to_string(),
            false,
            Some(test_source_range()),
            1.0,
        )];
        let mut callee = FileExtraction::empty(PathBuf::from("/proj/math.js"), LangId::JavaScript);
        callee.symbols = vec![symbol_for(
            2,
            "multiply",
            std::path::Path::new("/proj/math.js"),
        )];
        let extractions = vec![caller, callee];

        let mut builder = GraphBuilder::new(SnapshotId::new(1).unwrap());
        register_files(&mut builder, &extractions);
        register_symbols(&mut builder, &extractions, &mut Vec::new());
        let mut graph = builder.build();
        let mut diagnostics = Vec::new();
        inject_client_call_edges(
            &mut graph,
            &extractions,
            std::path::Path::new("/proj"),
            &mut diagnostics,
        );

        assert!(
            graph.reference_edges().count() >= 1,
            "the invocation must project onto its symbol"
        );
    }

    fn symbol_for(id: u32, name: &str, file_path: &std::path::Path) -> Symbol {
        Symbol {
            id: SymbolId::new(id).unwrap(),
            name: name.to_string(),
            kind: SymbolKind::Function,
            language: LangId::Python,
            file_path: file_path.to_path_buf(),
            source_range: test_source_range(),
            visibility: Some(Visibility::Public),
            signature: None,
            docstring: None,
            is_async: false,
        }
    }

    fn import(specifier: &str) -> crate::model::UnresolvedImport {
        crate::model::UnresolvedImport {
            import_specifier: specifier.to_string(),
            alias: None,
            symbol: None,
            star: false,
            range: test_source_range(),
        }
    }

    #[cfg(feature = "dataflow")]
    fn data_node(id: u32, name: &str) -> crate::model::DataNode {
        use crate::model::{DataNodeId, DataScope};
        crate::model::DataNode {
            id: DataNodeId::new(id).unwrap(),
            symbol_id: None,
            name: Some(name.to_string()),
            scope: DataScope::Local,
            type_hint: None,
            source_range: test_source_range(),
        }
    }
}
