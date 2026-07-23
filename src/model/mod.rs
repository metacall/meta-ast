//! Canonical symbol model and IR types.
//!
//! Defines `Symbol`, `UnresolvedImport`, `UnresolvedReference`,
//! `FileExtraction`, and supporting types (`SymbolKind`, `Visibility`,
//! `SourceRange`, `LineColumn`). This is the core data model with
//! zero knowledge of parsing, I/O, or language specifics.

pub mod ids;
pub mod output;

use std::path::PathBuf;

pub use ids::{DataNodeId, FileId, IdGenerator, SnapshotId, SymbolId};
pub use output::{ClassEntry, FuncEntry, InspectOutput, ObjectEntry};

use crate::language::LangId;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct UnresolvedImport {
    pub import_specifier: String,
    pub alias: Option<String>,
    pub symbol: Option<String>,
    pub star: bool,
    pub range: SourceRange,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnresolvedReference {
    pub name: String,
    pub range: SourceRange,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileExtraction {
    pub path: PathBuf,
    pub lang: LangId,
    pub symbols: Vec<Symbol>,
    pub imports: Vec<UnresolvedImport>,
    pub references: Vec<UnresolvedReference>,
    pub diagnostics: Vec<crate::error::Diagnostic>,
    /// Total number of tree-sitter AST nodes in the parse tree.
    pub ast_node_count: usize,
    #[cfg(feature = "metacall-deploy")]
    pub call_sites: Vec<crate::deploy::scanner::CallSite>,
    #[cfg(feature = "dataflow")]
    pub data_nodes: Vec<DataNode>,
    #[cfg(feature = "dataflow")]
    pub flow_edges: Vec<FlowEdge>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LineColumn {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceRange {
    pub byte_start: usize,
    pub byte_end: usize,
    pub start: LineColumn,
    pub end: LineColumn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Struct,
    Interface,
    Trait,
    Enum,
    Object,
    Constant,
    Static,
    Module,
    Namespace,
    TypeAlias,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub enum Visibility {
    Public,
    Private,
}

#[derive(Debug, Clone, Serialize)]
pub struct Symbol {
    pub id: SymbolId,
    pub name: String,
    pub kind: SymbolKind,
    pub language: LangId,
    pub file_path: PathBuf,
    pub source_range: SourceRange,
    pub visibility: Option<Visibility>,
    pub signature: Option<String>,
    pub docstring: Option<String>,
    pub is_async: bool,
}

/// Classification of a data-bearing entity's visibility scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub enum DataScope {
    Local,
    Parameter,
    Member,
    Closure,
    Temporary,
}

impl DataScope {
    pub fn as_str(self) -> &'static str {
        match self {
            DataScope::Local => "local",
            DataScope::Parameter => "parameter",
            DataScope::Member => "member",
            DataScope::Closure => "closure",
            DataScope::Temporary => "temporary",
        }
    }
}

/// A value-bearing node for def-use and flow analysis.
#[derive(Debug, Clone, Serialize)]
pub struct DataNode {
    pub id: DataNodeId,
    pub symbol_id: Option<SymbolId>,
    pub name: Option<String>,
    pub scope: DataScope,
    pub type_hint: Option<String>,
    pub source_range: SourceRange,
}

/// A def-use or dataflow edge between two DataNodes.
#[derive(Debug, Clone, Serialize)]
pub struct FlowEdge {
    pub source: DataNodeId,
    pub target: DataNodeId,
    pub kind: FlowKind,
    pub confidence: f32,
}

/// Semantic kind of a dataflow edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub enum FlowKind {
    DefUse,
    Argument,
    Return,
    FieldAccess,
}

impl FlowKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FlowKind::DefUse => "def_use",
            FlowKind::Argument => "argument",
            FlowKind::Return => "return",
            FlowKind::FieldAccess => "field_access",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    fn sample_source_range() -> SourceRange {
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

    #[test]
    fn symbol_construction_all_fields() {
        let sym = Symbol {
            id: SymbolId(42),
            name: "my_func".into(),
            kind: SymbolKind::Function,
            language: LangId::Rust,
            file_path: PathBuf::from("src/main.rs"),
            source_range: sample_source_range(),
            visibility: Some(Visibility::Public),
            signature: Some("fn my_func() -> bool".into()),
            docstring: Some("does a thing".into()),
            is_async: true,
        };
        assert_eq!(sym.id, SymbolId(42));
        assert_eq!(sym.name, "my_func");
        assert!(matches!(sym.kind, SymbolKind::Function));
        assert_eq!(sym.language, LangId::Rust);
        assert_eq!(sym.file_path, PathBuf::from("src/main.rs"));
        assert_eq!(sym.visibility, Some(Visibility::Public));
        assert_eq!(sym.signature.as_deref(), Some("fn my_func() -> bool"));
        assert_eq!(sym.docstring.as_deref(), Some("does a thing"));
        assert!(sym.is_async);
    }

    #[test]
    fn symbol_with_optional_fields_none() {
        let sym = Symbol {
            id: SymbolId(1),
            name: "x".into(),
            kind: SymbolKind::Constant,
            language: LangId::Python,
            file_path: PathBuf::from("a.py"),
            source_range: sample_source_range(),
            visibility: None,
            signature: None,
            docstring: None,
            is_async: false,
        };
        assert!(sym.visibility.is_none());
        assert!(sym.signature.is_none());
        assert!(sym.docstring.is_none());
        assert!(!sym.is_async);
    }

    #[test]
    fn source_range_fields() {
        let sr = sample_source_range();
        assert_eq!(sr.byte_start, 0);
        assert_eq!(sr.byte_end, 10);
        assert_eq!(sr.start, LineColumn { line: 1, column: 0 });
        assert_eq!(
            sr.end,
            LineColumn {
                line: 1,
                column: 10
            }
        );
    }

    #[test]
    fn line_column_zero_indexed() {
        let lc = LineColumn { line: 0, column: 0 };
        assert_eq!(lc.line, 0);
        assert_eq!(lc.column, 0);
    }

    #[test]
    fn visibility_serialization() {
        assert_eq!(
            serde_json::to_string(&Visibility::Public).unwrap(),
            "\"Public\""
        );
        assert_eq!(
            serde_json::to_string(&Visibility::Private).unwrap(),
            "\"Private\""
        );
    }

    #[test]
    fn symbol_kind_all_variants_serialize() {
        let variants: Vec<SymbolKind> = vec![
            SymbolKind::Function,
            SymbolKind::Method,
            SymbolKind::Class,
            SymbolKind::Struct,
            SymbolKind::Interface,
            SymbolKind::Trait,
            SymbolKind::Enum,
            SymbolKind::Object,
            SymbolKind::Constant,
            SymbolKind::Static,
            SymbolKind::Module,
            SymbolKind::Namespace,
            SymbolKind::TypeAlias,
        ];
        for v in &variants {
            let json = serde_json::to_string(v).unwrap();
            assert!(
                json.starts_with('"') && json.ends_with('"'),
                "expected a JSON string, got: {json}"
            );
            assert!(
                json.len() > 2,
                "expected non-empty variant name, got: {json}"
            );
        }
    }

    #[test]
    fn symbol_serde_roundtrip() {
        let sym = Symbol {
            id: SymbolId(7),
            name: "roundtrip_fn".into(),
            kind: SymbolKind::Method,
            language: LangId::Go,
            file_path: PathBuf::from("main.go"),
            source_range: sample_source_range(),
            visibility: Some(Visibility::Private),
            signature: Some("func (t T) roundtripFn()".into()),
            docstring: Some("doc".into()),
            is_async: false,
        };
        let json = serde_json::to_string(&sym).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["name"], "roundtrip_fn");
        assert_eq!(val["kind"], "Method");
        assert_eq!(val["is_async"], false);
    }

    #[test]
    fn data_scope_as_str_all_variants() {
        assert_eq!(DataScope::Local.as_str(), "local");
        assert_eq!(DataScope::Parameter.as_str(), "parameter");
        assert_eq!(DataScope::Member.as_str(), "member");
        assert_eq!(DataScope::Closure.as_str(), "closure");
        assert_eq!(DataScope::Temporary.as_str(), "temporary");
    }

    #[test]
    fn data_scope_serialization() {
        assert_eq!(
            serde_json::to_string(&DataScope::Local).unwrap(),
            "\"Local\""
        );
        assert_eq!(
            serde_json::to_string(&DataScope::Parameter).unwrap(),
            "\"Parameter\""
        );
    }

    #[test]
    fn flow_kind_as_str_all_variants() {
        assert_eq!(FlowKind::DefUse.as_str(), "def_use");
        assert_eq!(FlowKind::Argument.as_str(), "argument");
        assert_eq!(FlowKind::Return.as_str(), "return");
        assert_eq!(FlowKind::FieldAccess.as_str(), "field_access");
    }

    #[test]
    fn flow_kind_serialization() {
        assert_eq!(
            serde_json::to_string(&FlowKind::DefUse).unwrap(),
            "\"DefUse\""
        );
        assert_eq!(
            serde_json::to_string(&FlowKind::Argument).unwrap(),
            "\"Argument\""
        );
        assert_eq!(
            serde_json::to_string(&FlowKind::Return).unwrap(),
            "\"Return\""
        );
        assert_eq!(
            serde_json::to_string(&FlowKind::FieldAccess).unwrap(),
            "\"FieldAccess\""
        );
    }

    #[test]
    fn data_node_construction_and_serde() {
        let data = DataNode {
            id: DataNodeId(10),
            symbol_id: Some(SymbolId(5)),
            name: Some("x".into()),
            scope: DataScope::Local,
            type_hint: Some("int".into()),
            source_range: sample_source_range(),
        };
        let json = serde_json::to_string(&data).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["name"], "x");
        assert_eq!(val["scope"], "Local");
        assert_eq!(val["type_hint"], "int");
    }

    #[test]
    fn data_node_optional_fields_none() {
        let data = DataNode {
            id: DataNodeId(0),
            symbol_id: None,
            name: None,
            scope: DataScope::Temporary,
            type_hint: None,
            source_range: sample_source_range(),
        };
        assert!(data.symbol_id.is_none());
        assert!(data.name.is_none());
        assert!(data.type_hint.is_none());
    }

    #[test]
    fn flow_edge_serde() {
        let edge = FlowEdge {
            source: DataNodeId(1),
            target: DataNodeId(2),
            kind: FlowKind::Argument,
            confidence: 0.85,
        };
        let json = serde_json::to_string(&edge).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["source"], 1);
        assert_eq!(val["target"], 2);
        assert_eq!(val["kind"], "Argument");
        assert_eq!(val["confidence"], 0.85);
    }

    #[test]
    fn data_node_id_serde_roundtrip() {
        let original = DataNodeId(99);
        let json = serde_json::to_string(&original).unwrap();
        let roundtrip: DataNodeId = serde_json::from_str(&json).unwrap();
        assert_eq!(original, roundtrip);
    }
}
