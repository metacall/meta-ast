//! Shard file record representation and JSONL read/write operations.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use petgraph::visit::EdgeRef;
use serde::{Deserialize, Serialize};

use crate::error::Diagnostic;
use crate::graph::{CodeGraph, NodeData};
use crate::language::LangId;
use crate::model::{
    FileExtraction, IdGenerator, SourceRange, Symbol, SymbolId, SymbolKind, UnresolvedImport,
    UnresolvedReference, Visibility,
};
use crate::output::shard::edge::{ShardEdge, validate_edge};
use crate::output::shard::error::ShardError;
use crate::output::shard::name::{node_belongs_to_file, normalized_path, stable_node_name};

pub const SHARD_SCHEMA_VERSION: u32 = 2;

/// A per-file shard record stored in `.meta-ast/shards/<n>.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShardFile {
    pub schema_version: u32,
    pub path: PathBuf,
    pub language: LangId,
    pub symbols: Vec<ShardSymbol>,
    pub imports: Vec<UnresolvedImport>,
    pub references: Vec<UnresolvedReference>,
    pub diagnostics: Vec<Diagnostic>,
    pub ast_node_count: usize,
    pub edges: Vec<ShardEdge>,
}

/// Symbol representation in a shard file without runtime-local identifiers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShardSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub source_range: SourceRange,
    pub visibility: Option<Visibility>,
    pub signature: Option<String>,
    pub docstring: Option<String>,
    pub is_async: bool,
}

/// Extraction result reconstituted from a shard record.
#[derive(Debug)]
pub struct LoadedShard {
    pub file: FileExtraction,
    pub edges: Vec<ShardEdge>,
}

impl ShardFile {
    /// Convert an in-memory `FileExtraction` and graph into a shard record.
    pub fn from_extraction(
        extraction: &FileExtraction,
        graph: &CodeGraph,
    ) -> Result<Self, ShardError> {
        normalized_path(&extraction.path)?;
        for diagnostic in &extraction.diagnostics {
            normalized_path(&diagnostic.path)?;
        }
        let symbols = extraction.symbols.iter().map(ShardSymbol::from).collect();
        let mut edges = graph
            .graph()
            .edge_references()
            .filter(|edge| {
                !matches!(graph.graph()[edge.source()], NodeData::Data(_))
                    && !matches!(graph.graph()[edge.target()], NodeData::Data(_))
                    && (node_belongs_to_file(graph, edge.source(), &extraction.path)
                        || node_belongs_to_file(graph, edge.target(), &extraction.path))
            })
            .map(|edge| {
                let source_name = stable_node_name(graph, edge.source())?;
                let target_name = stable_node_name(graph, edge.target())?;
                Ok(ShardEdge {
                    source_name,
                    target_name,
                    kind: edge.weight().kind.into(),
                    confidence: edge.weight().confidence,
                    flow_kind: edge.weight().flow_kind.map(Into::into),
                })
            })
            .collect::<Result<Vec<_>, ShardError>>()?;
        edges.sort_by(|left, right| {
            (
                &left.source_name,
                &left.target_name,
                left.kind,
                left.flow_kind,
            )
                .cmp(&(
                    &right.source_name,
                    &right.target_name,
                    right.kind,
                    right.flow_kind,
                ))
        });
        for (edge_index, edge) in edges.iter().enumerate() {
            validate_edge(edge, 1, edge_index)?;
        }

        Ok(Self {
            schema_version: SHARD_SCHEMA_VERSION,
            path: extraction.path.clone(),
            language: extraction.lang,
            symbols,
            imports: extraction.imports.clone(),
            references: extraction.references.clone(),
            diagnostics: extraction.diagnostics.clone(),
            ast_node_count: extraction.ast_node_count,
            edges,
        })
    }

    /// Reconstitute a `FileExtraction` and assign fresh IDs using the provided generator.
    pub fn load(self, id_gen: &IdGenerator<SymbolId>) -> Result<LoadedShard, ShardError> {
        if self.schema_version != SHARD_SCHEMA_VERSION {
            return Err(ShardError::SchemaVersion {
                line: 1,
                found: self.schema_version,
                expected: SHARD_SCHEMA_VERSION,
            });
        }

        let symbols = self
            .symbols
            .into_iter()
            .map(|symbol| symbol.into_symbol(&self.path, self.language, id_gen.next()))
            .collect();
        let file = FileExtraction {
            path: self.path,
            lang: self.language,
            symbols,
            imports: self.imports,
            references: self.references,
            diagnostics: self.diagnostics,
            ast_node_count: self.ast_node_count,
            #[cfg(feature = "metacall-deploy")]
            call_sites: Vec::new(),
            #[cfg(feature = "dataflow")]
            data_nodes: Vec::new(),
            #[cfg(feature = "dataflow")]
            flow_edges: Vec::new(),
        };

        Ok(LoadedShard {
            file,
            edges: self.edges,
        })
    }
}

impl From<&Symbol> for ShardSymbol {
    fn from(symbol: &Symbol) -> Self {
        Self {
            name: symbol.name.clone(),
            kind: symbol.kind,
            source_range: symbol.source_range.clone(),
            visibility: symbol.visibility,
            signature: symbol.signature.clone(),
            docstring: symbol.docstring.clone(),
            is_async: symbol.is_async,
        }
    }
}

impl ShardSymbol {
    fn into_symbol(self, file_path: &Path, language: LangId, id: SymbolId) -> Symbol {
        Symbol {
            id,
            name: self.name,
            kind: self.kind,
            language,
            file_path: file_path.to_path_buf(),
            source_range: self.source_range,
            visibility: self.visibility,
            signature: self.signature,
            docstring: self.docstring,
            is_async: self.is_async,
        }
    }
}

/// Write shard records to a writer in JSONL format.
///
/// Use `std::io::BufWriter` when writing to a file to prevent frequent write system calls.
pub fn write_shard<W: Write>(mut writer: W, files: &[ShardFile]) -> Result<(), ShardError> {
    for (line_index, file) in files.iter().enumerate() {
        validate_file(file, line_index + 1)?;
        serde_json::to_writer(&mut writer, file).map_err(ShardError::Encode)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

pub fn read_shard<R: BufRead>(reader: R) -> Result<Vec<ShardFile>, ShardError> {
    let mut files = Vec::new();
    for (line_index, line) in reader.lines().enumerate() {
        let line_number = line_index + 1;
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let file: ShardFile = serde_json::from_str(&line).map_err(|source| ShardError::Decode {
            line: line_number,
            source,
        })?;
        validate_file(&file, line_number)?;
        files.push(file);
    }
    Ok(files)
}

pub(crate) fn validate_file(file: &ShardFile, line: usize) -> Result<(), ShardError> {
    if file.schema_version != SHARD_SCHEMA_VERSION {
        return Err(ShardError::SchemaVersion {
            line,
            found: file.schema_version,
            expected: SHARD_SCHEMA_VERSION,
        });
    }
    normalized_path(&file.path)?;
    for diagnostic in &file.diagnostics {
        normalized_path(&diagnostic.path)?;
    }
    for (edge_index, edge) in file.edges.iter().enumerate() {
        validate_edge(edge, line, edge_index)?;
    }
    Ok(())
}
