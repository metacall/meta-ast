pub mod error;
pub mod extractor;
pub mod graph;
pub mod input;
pub mod interface;
pub mod language;
pub mod model;
pub mod output;
pub mod parser;
pub mod pipeline;

pub use error::{Diagnostic, Error, Severity};
pub use input::detect_language;
pub use language::{LangId, LanguageSpec};
pub use model::{
    FileExtraction, Symbol, SymbolId, SymbolKind, UnresolvedImport, UnresolvedReference, Visibility,
};

// Graph module re-exports
pub use graph::{
    CodeGraph,
    builder::GraphBuilder,
    edge::{EdgeData, EdgeKind},
    node::{FileNode, NodeData, SymbolNode},
    scc::{DeployabilityHint, Scc, SccAnalysis},
};
