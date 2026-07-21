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

#[cfg(feature = "metacall-deploy")]
pub mod deploy;

#[cfg(feature = "dataflow")]
pub mod sink;

pub use error::{Diagnostic, Error, Severity};
pub use input::detect_language;
pub use language::{LangId, LanguageSpec};
pub use model::{
    DataNode, DataNodeId, DataScope, FileExtraction, FlowEdge, FlowKind, Symbol, SymbolId,
    SymbolKind, UnresolvedImport, UnresolvedReference, Visibility,
};

// Graph module re-exports
pub use graph::{
    CodeGraph,
    builder::GraphBuilder,
    edge::{EdgeData, EdgeKind},
    node::{ExternalNode, FileNode, NodeData, SymbolNode},
    resolver::FlattenedScopeCache,
    scc::{DeployabilityHint, Scc, SccAnalysis},
};

// Pipeline re-exports
pub use pipeline::{GraphAnalysis, SnapshotMeta, snapshot_meta};
