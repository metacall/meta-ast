//! Shard header representation and JSON read/write operations.
//!
//! `.meta-ast/header.json` defines metadata for the index directory including
//! the schema version, tool version, and index creation timestamp.

use std::io::{Read, Write};

use serde::{Deserialize, Serialize};

use crate::output::shard::error::ShardError;
use crate::output::shard::file::SHARD_SCHEMA_VERSION;

/// Global index header stored in `.meta-ast/header.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShardHeader {
    /// Index schema version.
    pub schema_version: u32,
    /// Version of the creating tool (e.g., meta-ast version).
    pub tool_version: String,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
}

impl ShardHeader {
    /// Create a new index header with the current crate version.
    pub fn new(created_at: impl Into<String>) -> Self {
        Self {
            schema_version: SHARD_SCHEMA_VERSION,
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            created_at: created_at.into(),
        }
    }

    /// Create an index header with a custom tool version.
    pub fn with_tool_version(
        tool_version: impl Into<String>,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: SHARD_SCHEMA_VERSION,
            tool_version: tool_version.into(),
            created_at: created_at.into(),
        }
    }
}

/// Write the header to a writer as formatted JSON.
pub fn write_header<W: Write>(mut writer: W, header: &ShardHeader) -> Result<(), ShardError> {
    if header.schema_version != SHARD_SCHEMA_VERSION {
        return Err(ShardError::SchemaVersion {
            line: 1,
            found: header.schema_version,
            expected: SHARD_SCHEMA_VERSION,
        });
    }
    serde_json::to_writer_pretty(&mut writer, header).map_err(ShardError::Encode)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

/// Read the header from a reader.
pub fn read_header<R: Read>(reader: R) -> Result<ShardHeader, ShardError> {
    let header: ShardHeader =
        serde_json::from_reader(reader).map_err(|source| ShardError::Decode { line: 1, source })?;
    if header.schema_version != SHARD_SCHEMA_VERSION {
        return Err(ShardError::SchemaVersion {
            line: 1,
            found: header.schema_version,
            expected: SHARD_SCHEMA_VERSION,
        });
    }
    Ok(header)
}
