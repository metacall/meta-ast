//! Hardened `.meta-ast` index loader.
//!
//! Reads the header, manifest, and shard files under a project root and
//! rebuilds the extraction set. Shard names, record paths, and BLAKE3 content
//! hashes are verified. Stale records are skipped and counted; tampered names
//! or paths are hard errors.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::model::{FileExtraction, IdGenerator, SymbolId};

use super::edge::ShardEdge;
use super::error::ShardError;
use super::file::{LoadedShard, ShardFile, read_shard};
use super::header::{ShardHeader, read_header};
use super::manifest::{ShardManifestRecord, read_manifest};

/// Directory name of the generated index under the project root.
pub const INDEX_DIR_NAME: &str = ".meta-ast";
const HEADER_FILE: &str = "header.json";
const MANIFEST_FILE: &str = "manifest.jsonl";

/// Controls index verification during [`load_index`].
#[derive(Debug)]
pub struct IndexLoadOptions {
    /// Verify the BLAKE3 content hash of every record.
    pub verify_content_hash: bool,
    /// Require the index tool version to match this build.
    pub require_matching_tool_version: bool,
}

impl Default for IndexLoadOptions {
    fn default() -> Self {
        Self {
            verify_content_hash: true,
            require_matching_tool_version: true,
        }
    }
}

/// Counts of records that were loaded or skipped.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct IndexLoadStats {
    pub loaded: usize,
    pub skipped: usize,
}

/// Rebuilt index contents.
#[derive(Debug)]
pub struct LoadedIndex {
    pub header: ShardHeader,
    pub extractions: Vec<Arc<FileExtraction>>,
    pub edges: Vec<ShardEdge>,
    pub stats: IndexLoadStats,
}

/// Reports whether a manifest shard name stays inside `shards/`.
pub fn is_safe_shard_name(name: &str) -> bool {
    let path = Path::new(name);
    !path.is_absolute()
        && path.starts_with("shards")
        && !path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
}

/// Load and verify a `.meta-ast` index.
///
/// IDs are assigned in manifest order, so the caller must pass a generator
/// whose start exceeds any cached ID.
pub fn load_index(
    root: &Path,
    id_gen: &IdGenerator<SymbolId>,
    options: &IndexLoadOptions,
) -> Result<LoadedIndex, ShardError> {
    let dir = root.join(INDEX_DIR_NAME);
    let header = read_header(BufReader::new(File::open(dir.join(HEADER_FILE))?))?;
    let expected_version = env!("CARGO_PKG_VERSION");
    if options.require_matching_tool_version && header.tool_version != expected_version {
        return Err(ShardError::ToolVersionMismatch {
            found: header.tool_version,
            expected: expected_version.to_string(),
        });
    }
    let manifest = read_manifest(BufReader::new(File::open(dir.join(MANIFEST_FILE))?))?;

    let mut shards: BTreeMap<String, Vec<ShardFile>> = BTreeMap::new();
    for record in &manifest {
        if !is_safe_shard_name(&record.shard) {
            return Err(ShardError::UnsafeShardName {
                name: record.shard.clone(),
            });
        }
        if shards.contains_key(&record.shard) {
            continue;
        }
        let path = dir.join(&record.shard);
        let files = match File::open(&path) {
            Ok(file) => read_shard(BufReader::new(file))?,
            Err(_) => continue,
        };
        shards.insert(record.shard.clone(), files);
    }

    let root_canon = dunce::canonicalize(root)?;
    let mut by_path: BTreeMap<PathBuf, ShardFile> = BTreeMap::new();
    for files in shards.into_values() {
        for file in files {
            by_path.insert(file.path.clone(), file);
        }
    }

    let mut extractions = Vec::new();
    let mut edges = Vec::new();
    let mut stats = IndexLoadStats::default();
    for record in &manifest {
        let Some(absolute) = resolve_record_path(&root_canon, &record.path)? else {
            stats.skipped += 1;
            continue;
        };
        let Some(shard) = by_path.remove(&record.path) else {
            stats.skipped += 1;
            continue;
        };
        let Ok(metadata) = std::fs::metadata(&absolute) else {
            stats.skipped += 1;
            continue;
        };
        if metadata.len() != record.size {
            stats.skipped += 1;
            continue;
        }
        let Ok(bytes) = std::fs::read(&absolute) else {
            stats.skipped += 1;
            continue;
        };
        if options.verify_content_hash
            && ShardManifestRecord::compute_hash(&bytes) != record.content_hash
        {
            stats.skipped += 1;
            continue;
        }
        let LoadedShard {
            file,
            edges: shard_edges,
        } = match shard.load(id_gen) {
            Ok(loaded) => loaded,
            Err(_) => {
                stats.skipped += 1;
                continue;
            }
        };
        extractions.push(Arc::new(file));
        edges.extend(shard_edges);
        stats.loaded += 1;
    }

    Ok(LoadedIndex {
        header,
        extractions,
        edges,
        stats,
    })
}

/// Canonical record path when it exists under the root.
///
/// Returns `Ok(None)` for stale records whose file no longer exists. Returns
/// an error when the resolved path escapes the root.
fn resolve_record_path(root_canon: &Path, path: &Path) -> Result<Option<PathBuf>, ShardError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root_canon.join(path)
    };
    let Ok(canonical) = dunce::canonicalize(&absolute) else {
        return Ok(None);
    };
    if !canonical.starts_with(root_canon) {
        return Err(ShardError::PathOutsideRoot {
            path: path.to_path_buf(),
        });
    }
    Ok(Some(canonical))
}
