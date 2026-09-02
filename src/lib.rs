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

#[cfg(feature = "watch")]
pub mod watch;

#[cfg(feature = "metacall-deploy")]
pub mod deploy;

#[cfg(feature = "dataflow")]
pub mod sink;

pub use error::{Diagnostic, Error, Severity};
pub use extractor::{
    ExtractOptions, ExtractionIdGenerators, ExtractionResult, InMemorySource, VersionedExtraction,
    extract_text_with_id_gen, extract_with_id_gen,
};
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

// Shard and index re-exports
pub use output::shard::{
    LoadedShard, SHARD_SCHEMA_VERSION, ShardEdge, ShardEdgeKind, ShardError, ShardFile,
    ShardFlowKind, ShardHeader, ShardManifestRecord, ShardSymbol, read_header, read_manifest,
    read_shard, restore_shard_edges, write_header, write_manifest, write_shard,
};
pub use pipeline::{GraphAnalysis, SnapshotMeta, snapshot_meta};

// Watch-mode re-exports
#[cfg(feature = "watch")]
pub use watch::{ChangeSet, WatchConfig, WatchState, incremental_reanalyze, run_watch};
