//! Graph node types for the dependency graph.
//!
//! This module defines the heterogeneous node types used in the CodeGraph:
//! - `FileNode`: Represents a source file
//! - `SymbolNode`: Represents an extracted symbol (function, class, etc.)
//!
//! Nodes are stored in a unified `NodeData` enum for use with petgraph.

use std::path::PathBuf;

use crate::language::LangId;
use crate::model::{DataNodeId, FileId, SnapshotId, SourceRange, SymbolId, SymbolKind, Visibility};

/// Unified node data enum for the dependency graph.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum NodeData {
    File(FileNode),
    Symbol(SymbolNode),
    External(ExternalNode),
    Data(DataGraphNode),
}

impl NodeData {
    /// Returns a string identifier for the node kind.
    pub fn kind_str(&self) -> &'static str {
        match self {
            NodeData::File(_) => "file",
            NodeData::Symbol(_) => "symbol",
            NodeData::External(_) => "external",
            NodeData::Data(_) => "data",
        }
    }

    /// Returns the FileId if this is a FileNode.
    pub fn as_file(&self) -> Option<&FileNode> {
        if let NodeData::File(f) = self {
            Some(f)
        } else {
            None
        }
    }

    /// Returns the SymbolId and associated data if this is a SymbolNode.
    pub fn as_symbol(&self) -> Option<&SymbolNode> {
        if let NodeData::Symbol(s) = self {
            Some(s)
        } else {
            None
        }
    }

    /// Returns the ExternalNode if this is an ExternalNode.
    pub fn as_external(&self) -> Option<&ExternalNode> {
        if let NodeData::External(e) = self {
            Some(e)
        } else {
            None
        }
    }

    /// Returns the DataGraphNode if this is a Data node.
    pub fn as_data(&self) -> Option<&DataGraphNode> {
        if let NodeData::Data(d) = self {
            Some(d)
        } else {
            None
        }
    }

    /// Returns the file path if this is a FileNode.
    pub fn file_path(&self) -> Option<&PathBuf> {
        self.as_file().map(|f| &f.path)
    }

    /// Returns the symbol name if this is a SymbolNode.
    pub fn symbol_name(&self) -> Option<&str> {
        self.as_symbol().map(|s| s.name.as_str())
    }
}

/// Represents a source file in the dependency graph.
#[derive(Debug, Clone)]
pub struct FileNode {
    /// Stable identifier for this file.
    pub id: FileId,
    /// Project-root-relative path for stable identification.
    pub path: PathBuf,
    /// Detected language for this file.
    pub language: LangId,
    /// Snapshot identifier for versioning support.
    pub snapshot_id: SnapshotId,
}

/// Represents an external dependency (not in the project).
#[derive(Debug, Clone)]
pub struct ExternalNode {
    /// Raw import path string (e.g., "react", "std::collections::HashMap")
    pub raw_path: String,
    /// Language of the importing file
    pub language: LangId,
    /// Classification result - `None` until dependency resolution runs.
    pub classification: Option<ExternalClassification>,
}

/// Classification of an external dependency.
#[derive(Debug, Clone, serde::Serialize)]
#[non_exhaustive]
pub enum ExternalClassification {
    /// Successfully resolved to a known package (lockfile or manifest).
    Classified {
        package_name: String,
        version: Option<String>,
        language: LangId,
        source: DependencySource,
    },
    /// Best-effort failed; kept as unresolved for transparency.
    Unresolved { raw_path: String, reason: String },
}

/// Where the dependency information came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[non_exhaustive]
pub enum DependencySource {
    Lockfile,
    Manifest,
}

/// Represents an extracted symbol in the dependency graph.
///
/// Symbols are owned by exactly one FileNode (via Ownership edge) and may
/// reference other symbols (via Reference edges).
#[derive(Debug, Clone)]
pub struct SymbolNode {
    /// Stable identifier for this symbol.
    pub id: SymbolId,
    /// Symbol name as extracted from source.
    pub name: String,
    /// Symbol classification (function, class, etc.).
    pub kind: SymbolKind,
    /// Reference to the containing file.
    pub file_id: FileId,
    /// Visibility modifier if applicable.
    pub visibility: Option<Visibility>,
    /// Source location within the file.
    pub source_range: SourceRange,
}

/// Represents a data-bearing node in the data/flow subgraph.
#[derive(Debug, Clone)]
pub struct DataGraphNode {
    pub id: DataNodeId,
    pub symbol_id: Option<SymbolId>,
    pub name: Option<String>,
    pub scope: crate::model::DataScope,
    pub type_hint: Option<String>,
    pub source_range: SourceRange,
}

impl FileNode {
    /// Creates a new FileNode with the given properties.
    pub fn new(id: FileId, path: PathBuf, language: LangId, snapshot_id: SnapshotId) -> Self {
        Self {
            id,
            path,
            language,
            snapshot_id,
        }
    }

    /// Returns the file name component of the path.
    pub fn file_name(&self) -> Option<&str> {
        self.path.file_name().and_then(|n| n.to_str())
    }

    /// Returns the file extension if present.
    pub fn extension(&self) -> Option<&str> {
        self.path.extension().and_then(|e| e.to_str())
    }
}

impl SymbolNode {
    /// Creates a new SymbolNode from an extracted Symbol.
    ///
    /// This factory method bridges the model layer to the graph layer.
    pub fn from_symbol(symbol: &crate::model::Symbol, file_id: FileId) -> Self {
        Self {
            id: symbol.id,
            name: symbol.name.clone(),
            kind: symbol.kind,
            file_id,
            visibility: symbol.visibility,
            source_range: symbol.source_range.clone(),
        }
    }
}

impl DataGraphNode {
    /// Creates a new DataGraphNode from an extracted DataNode.
    pub fn from_data_node(data: &crate::model::DataNode) -> Self {
        Self {
            id: data.id,
            symbol_id: data.symbol_id,
            name: data.name.clone(),
            scope: data.scope,
            type_hint: data.type_hint.clone(),
            source_range: data.source_range.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{LineColumn, SourceRange};

    fn test_path() -> PathBuf {
        PathBuf::from("src/main.rs")
    }

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

    #[test]
    fn file_node_creation() {
        let file_id = FileId::from(0);
        let snapshot_id = SnapshotId::from(1);
        let node = FileNode::new(file_id, test_path(), LangId::Rust, snapshot_id);

        assert_eq!(node.id, file_id);
        assert_eq!(node.path, test_path());
        assert_eq!(node.language, LangId::Rust);
        assert_eq!(node.snapshot_id, snapshot_id);
    }

    #[test]
    fn file_node_file_name() {
        let node = FileNode::new(
            FileId::from(0),
            PathBuf::from("src/main.rs"),
            LangId::Rust,
            SnapshotId::from(0),
        );
        assert_eq!(node.file_name(), Some("main.rs"));
    }

    #[test]
    fn file_node_extension() {
        let node = FileNode::new(
            FileId::from(0),
            PathBuf::from("test.py"),
            LangId::Python,
            SnapshotId::from(0),
        );
        assert_eq!(node.extension(), Some("py"));
    }

    #[test]
    fn symbol_node_creation() {
        let symbol = crate::model::Symbol {
            id: SymbolId::from(42),
            name: "test_function".to_string(),
            kind: SymbolKind::Function,
            language: LangId::Rust,
            file_path: test_path(),
            source_range: test_source_range(),
            visibility: Some(Visibility::Public),
            signature: None,
            docstring: None,
            is_async: false,
        };

        let file_id = FileId::from(7);
        let node = SymbolNode::from_symbol(&symbol, file_id);

        assert_eq!(node.id, SymbolId::from(42));
        assert_eq!(node.name, "test_function");
        assert_eq!(node.kind, SymbolKind::Function);
        assert_eq!(node.file_id, file_id);
        assert_eq!(node.visibility, Some(Visibility::Public));
    }

    #[test]
    fn node_data_file_variant() {
        let file_node = FileNode::new(
            FileId::from(0),
            test_path(),
            LangId::Rust,
            SnapshotId::from(0),
        );
        let node_data = NodeData::File(file_node);

        assert_eq!(node_data.kind_str(), "file");
        assert!(node_data.as_file().is_some());
        assert!(node_data.as_symbol().is_none());
        assert_eq!(node_data.file_path(), Some(&test_path()));
        assert_eq!(node_data.symbol_name(), None);
    }

    #[test]
    fn node_data_symbol_variant() {
        let symbol_node = SymbolNode {
            id: SymbolId::from(1),
            name: "my_func".to_string(),
            kind: SymbolKind::Function,
            file_id: FileId::from(0),
            visibility: None,
            source_range: test_source_range(),
        };
        let node_data = NodeData::Symbol(symbol_node);

        assert_eq!(node_data.kind_str(), "symbol");
        assert!(node_data.as_symbol().is_some());
        assert!(node_data.as_file().is_none());
        assert_eq!(node_data.file_path(), None);
        assert_eq!(node_data.symbol_name(), Some("my_func"));
    }

    #[test]
    fn data_graph_node_from_model() {
        use crate::model::{DataNode, DataNodeId, DataScope};
        let data = DataNode {
            id: DataNodeId(5),
            symbol_id: None,
            name: Some("var_x".into()),
            scope: DataScope::Local,
            type_hint: Some("int".into()),
            source_range: test_source_range(),
        };
        let gnode = DataGraphNode::from_data_node(&data);
        assert_eq!(gnode.id, DataNodeId(5));
        assert_eq!(gnode.name.as_deref(), Some("var_x"));
        assert_eq!(gnode.scope, DataScope::Local);
        assert_eq!(gnode.type_hint.as_deref(), Some("int"));
        assert!(gnode.symbol_id.is_none());
    }

    #[test]
    fn node_data_data_variant() {
        let dnode = DataGraphNode {
            id: crate::model::DataNodeId(1),
            symbol_id: Some(SymbolId::from(3)),
            name: Some("param".into()),
            scope: crate::model::DataScope::Parameter,
            type_hint: Some("str".into()),
            source_range: test_source_range(),
        };
        let node_data = NodeData::Data(dnode);
        assert_eq!(node_data.kind_str(), "data");
        assert!(node_data.as_data().is_some());
        assert!(node_data.as_file().is_none());
        assert!(node_data.as_symbol().is_none());
        assert_eq!(node_data.as_data().unwrap().name.as_deref(), Some("param"));
    }
}
