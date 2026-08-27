//! Shard manifest representation and JSONL read/write operations.
//!
//! `.meta-ast/manifest.jsonl` tracks one record per indexed file with its
//! BLAKE3 content hash, byte size, modification timestamp, shard location,
//! and schema version.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::output::shard::error::ShardError;
use crate::output::shard::file::SHARD_SCHEMA_VERSION;
use crate::output::shard::name::normalized_path;

/// One line in `.meta-ast/manifest.jsonl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShardManifestRecord {
    /// Relative path of the indexed file.
    pub path: PathBuf,
    /// BLAKE3 hex hash of the file contents.
    pub content_hash: String,
    /// File size in bytes.
    pub size: u64,
    /// Modification timestamp in seconds since Unix epoch.
    pub mtime: u64,
    /// Shard filename (e.g., "0.jsonl" or "shards/0.jsonl").
    pub shard: String,
    /// Shard schema version.
    pub schema_version: u32,
}

impl ShardManifestRecord {
    /// Create a new manifest record.
    pub fn new(path: PathBuf, content_hash: String, size: u64, mtime: u64, shard: String) -> Self {
        Self {
            path,
            content_hash,
            size,
            mtime,
            shard,
            schema_version: SHARD_SCHEMA_VERSION,
        }
    }

    /// Compute the BLAKE3 hex hash for source bytes.
    pub fn compute_hash(bytes: &[u8]) -> String {
        blake3::hash(bytes).to_hex().to_string()
    }

    /// Create a record by hashing in-memory bytes directly.
    pub fn from_file_bytes(path: PathBuf, bytes: &[u8], mtime: u64, shard: String) -> Self {
        let content_hash = Self::compute_hash(bytes);
        let size = bytes.len() as u64;
        Self::new(path, content_hash, size, mtime, shard)
    }
}

/// Write manifest records to a writer in JSONL format.
pub fn write_manifest<W: Write>(
    mut writer: W,
    records: &[ShardManifestRecord],
) -> Result<(), ShardError> {
    for (line_index, record) in records.iter().enumerate() {
        validate_manifest_record(record, line_index + 1)?;
        serde_json::to_writer(&mut writer, record).map_err(ShardError::Encode)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

/// Read manifest records from a buffered JSONL reader.
pub fn read_manifest<R: BufRead>(reader: R) -> Result<Vec<ShardManifestRecord>, ShardError> {
    let mut records = Vec::new();
    for (line_index, line) in reader.lines().enumerate() {
        let line_number = line_index + 1;
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let record: ShardManifestRecord =
            serde_json::from_str(&line).map_err(|source| ShardError::Decode {
                line: line_number,
                source,
            })?;
        validate_manifest_record(&record, line_number)?;
        records.push(record);
    }
    Ok(records)
}

fn validate_manifest_record(record: &ShardManifestRecord, line: usize) -> Result<(), ShardError> {
    if record.schema_version != SHARD_SCHEMA_VERSION {
        return Err(ShardError::SchemaVersion {
            line,
            found: record.schema_version,
            expected: SHARD_SCHEMA_VERSION,
        });
    }
    normalized_path(&record.path)?;
    Ok(())
}
