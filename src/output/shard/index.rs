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

use super::edge::{ShardEdge, ShardEdgeKind};
use super::error::ShardError;
use super::file::{LoadedShard, ShardFile, read_shard};
use super::header::{ShardHeader, read_header};
use super::manifest::{ShardManifestRecord, read_manifest};
use super::name::is_device_stem;

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

/// A manifest record that was not loaded, with the reason it was skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardSkip {
    pub path: PathBuf,
    pub reason: String,
}

/// Rebuilt index contents.
#[derive(Debug)]
pub struct LoadedIndex {
    pub header: ShardHeader,
    pub extractions: Vec<Arc<FileExtraction>>,
    pub edges: Vec<ShardEdge>,
    pub stats: IndexLoadStats,
    /// One entry per skipped manifest record, in manifest order. A stale file
    /// and a corrupt shard are not the same condition, so the caller gets the
    /// reason instead of a bare count.
    pub skips: Vec<ShardSkip>,
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

/// Reports whether a shard name can be written on every platform this index
/// supports: it stays inside `shards/`, it does not end in a dot or a space,
/// and its stem is not a reserved Windows device.
pub fn is_writable_name(name: &str) -> bool {
    if !is_safe_shard_name(name) {
        return false;
    }
    let Some(file_name) = Path::new(name).file_name().and_then(|part| part.to_str()) else {
        return false;
    };
    if file_name.ends_with('.') || file_name.ends_with(' ') {
        return false;
    }
    !is_device_stem(file_name.split('.').next().unwrap_or_default())
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
    let mut unreadable_shards: BTreeMap<String, String> = BTreeMap::new();
    for record in &manifest {
        if !is_safe_shard_name(&record.shard) {
            return Err(ShardError::UnsafeShardName {
                name: record.shard.clone(),
            });
        }
        // Containment and writability stay separate, so a traversal attempt
        // and a name that no portable writer creates do not look alike.
        if !is_writable_name(&record.shard) {
            return Err(ShardError::UnwritableShardName {
                name: record.shard.clone(),
                reason: "the name is reserved or ends in a dot or a space",
            });
        }
        if shards.contains_key(&record.shard) {
            continue;
        }
        let path = dir.join(&record.shard);
        let files = match File::open(&path) {
            Ok(file) => match read_shard(BufReader::new(file)) {
                Ok(files) => files,
                Err(error) => {
                    unreadable_shards.insert(record.shard.clone(), error.to_string());
                    continue;
                }
            },
            Err(error) => {
                unreadable_shards.insert(record.shard.clone(), error.to_string());
                continue;
            }
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
    let mut skips: Vec<ShardSkip> = Vec::new();
    for record in &manifest {
        let Some(absolute) = resolve_record_path(&root_canon, &record.path)? else {
            record_skip(
                &mut stats,
                &mut skips,
                &record.path,
                "the file is gone or sits outside the index root".to_string(),
            );
            continue;
        };
        let Some(shard) = by_path.remove(&record.path) else {
            let reason = unreadable_shards
                .get(&record.shard)
                .cloned()
                .unwrap_or_else(|| format!("no record for this path in {}", record.shard));
            record_skip(&mut stats, &mut skips, &record.path, reason);
            continue;
        };
        let Ok(metadata) = std::fs::metadata(&absolute) else {
            record_skip(
                &mut stats,
                &mut skips,
                &record.path,
                "the file cannot be read".to_string(),
            );
            continue;
        };
        if metadata.len() != record.size {
            record_skip(
                &mut stats,
                &mut skips,
                &record.path,
                format!(
                    "size changed: manifest claims {} bytes, the file has {}",
                    record.size,
                    metadata.len()
                ),
            );
            continue;
        }
        let Ok(bytes) = std::fs::read(&absolute) else {
            record_skip(
                &mut stats,
                &mut skips,
                &record.path,
                "the file cannot be read".to_string(),
            );
            continue;
        };
        if options.verify_content_hash
            && ShardManifestRecord::compute_hash(&bytes) != record.content_hash
        {
            record_skip(
                &mut stats,
                &mut skips,
                &record.path,
                "the content hash does not match the manifest".to_string(),
            );
            continue;
        }
        let LoadedShard {
            file,
            edges: shard_edges,
        } = match shard.load(id_gen) {
            Ok(loaded) => loaded,
            Err(error) => {
                record_skip(&mut stats, &mut skips, &record.path, error.to_string());
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
        skips,
    })
}

impl LoadedIndex {
    /// Persisted edges per kind, for a caller that reports index contents.
    pub fn edge_counts_by_kind(&self) -> BTreeMap<ShardEdgeKind, usize> {
        let mut counts = BTreeMap::new();
        for edge in &self.edges {
            *counts.entry(edge.kind).or_default() += 1;
        }
        counts
    }
}

fn record_skip(
    stats: &mut IndexLoadStats,
    skips: &mut Vec<ShardSkip>,
    path: &Path,
    reason: String,
) {
    stats.skipped += 1;
    skips.push(ShardSkip {
        path: path.to_path_buf(),
        reason,
    });
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::shard::header::write_header;
    use crate::output::shard::manifest::{ShardManifestRecord, write_manifest};

    /// A record the reader refuses is skipped with its reason, so a stale work
    /// tree, a corrupt payload and a schema mismatch stay distinguishable.
    #[test]
    fn skipped_records_carry_the_reason() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let index_dir = root.join(INDEX_DIR_NAME);
        std::fs::create_dir_all(index_dir.join("shards")).unwrap();

        let source = root.join("a.py");
        std::fs::write(&source, "def a():\n    pass\n").unwrap();

        let mut header_bytes = Vec::new();
        write_header(&mut header_bytes, &ShardHeader::new("2026-01-01T00:00:00Z")).unwrap();
        std::fs::write(index_dir.join(HEADER_FILE), header_bytes).unwrap();

        // Schema version 79 is not this build's version, so the record fails
        // to load. Written raw because the writer rejects it on purpose.
        let record = serde_json::json!({
            "schema_version": 79,
            "path": "a.py",
            "language": "python",
            "symbols": [],
            "imports": [],
            "references": [],
            "diagnostics": [],
            "ast_node_count": 1,
            "edges": [],
        });
        std::fs::write(index_dir.join("shards/0.jsonl"), format!("{record}\n")).unwrap();

        let bytes = std::fs::read(&source).unwrap();
        let manifest = ShardManifestRecord::from_file_bytes(
            PathBuf::from("a.py"),
            &bytes,
            0,
            "shards/0.jsonl".to_string(),
        );
        let mut manifest_bytes = Vec::new();
        write_manifest(&mut manifest_bytes, std::slice::from_ref(&manifest)).unwrap();
        std::fs::write(index_dir.join(MANIFEST_FILE), manifest_bytes).unwrap();

        let loaded = load_index(
            root,
            &IdGenerator::with_start(1),
            &IndexLoadOptions::default(),
        )
        .unwrap();

        assert_eq!(loaded.stats.loaded, 0);
        assert_eq!(loaded.stats.skipped, 1);
        assert_eq!(loaded.skips.len(), 1);
        assert_eq!(loaded.skips[0].path, PathBuf::from("a.py"));
        assert!(
            loaded.skips[0].reason.contains("79"),
            "the reason names the refused version: {}",
            loaded.skips[0].reason
        );
    }
}
