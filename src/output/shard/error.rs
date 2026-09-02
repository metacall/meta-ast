//! Error definitions for shard serialization and deserialization.

use std::path::PathBuf;

/// Failures that can occur during shard reading, writing, or restoration.
#[derive(Debug, thiserror::Error)]
pub enum ShardError {
    #[error("shard IO failed: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to encode shard record: {0}")]
    Encode(serde_json::Error),

    #[error("invalid shard JSON on line {line}: {source}")]
    Decode {
        line: usize,
        #[source]
        source: serde_json::Error,
    },

    #[error("unsupported shard schema version {found} on line {line}; expected {expected}")]
    SchemaVersion {
        line: usize,
        found: u32,
        expected: u32,
    },

    #[error("graph node {node_index} has no stable file owner")]
    MissingNodeOwner { node_index: usize },

    #[error("shard edge endpoint '{name}' does not exist in the rebuilt graph")]
    MissingEndpoint { name: String },

    #[error("path is not valid UTF-8: {path:?}")]
    NonUtf8Path { path: PathBuf },

    #[error("invalid edge {edge_index} on shard line {line}: {message}")]
    InvalidEdge {
        line: usize,
        edge_index: usize,
        message: String,
    },
}
