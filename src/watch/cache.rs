//! Incremental extraction cache and BLAKE3 content fingerprinting.
//!
//! Re-parsing source code with tree-sitter on every file edit is expensive.
//! Instead, `IncrementalCache` retains the previous `Arc<FileExtraction>`
//! for every source file. During re-analysis, file bytes are hashed using
//! BLAKE3. If the fingerprint matches the cached hash, the extraction is
//! reused without parsing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::model::FileExtraction;

/// BLAKE3 cryptographic hash of a file's raw bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Fingerprint(pub(crate) [u8; 32]);

/// Compute a deterministic 256-bit BLAKE3 fingerprint for a file's content bytes.
pub(crate) fn compute_fingerprint(bytes: &[u8]) -> Fingerprint {
    Fingerprint(*blake3::hash(bytes).as_bytes())
}

/// Cache of per-file extraction results keyed by canonical path.
#[derive(Debug)]
pub(crate) struct IncrementalCache {
    pub(crate) extractions: HashMap<PathBuf, Arc<FileExtraction>>,
    pub(crate) fingerprints: HashMap<PathBuf, Fingerprint>,
}

/// Maximum raw ID in an iterator, or 0 when empty.
///
/// Shared by symbol and data-node scans so both stay in sync.
/// IDs grow monotonically and are never reused after deletion.
fn max_raw(ids: impl Iterator<Item = u32>) -> u32 {
    ids.max().unwrap_or(0)
}

impl IncrementalCache {
    pub(crate) fn new() -> Self {
        Self {
            extractions: HashMap::new(),
            fingerprints: HashMap::new(),
        }
    }

    /// Find the highest raw symbol ID allocated across all cached files.
    pub(crate) fn max_symbol_id(&self) -> u32 {
        max_raw(
            self.extractions
                .values()
                .flat_map(|ext| ext.symbols.iter().map(|s| s.id.to_raw())),
        )
    }

    /// Find the highest raw data node ID allocated across all cached files.
    #[cfg(feature = "dataflow")]
    pub(crate) fn max_data_node_id(&self) -> u32 {
        max_raw(
            self.extractions
                .values()
                .flat_map(|ext| ext.data_nodes.iter().map(|d| d.id.to_raw())),
        )
    }

    /// Update or insert a file's fingerprint and shared extraction.
    pub(crate) fn update(
        &mut self,
        path: PathBuf,
        fp: Fingerprint,
        extraction: Arc<FileExtraction>,
    ) {
        self.fingerprints.insert(path.clone(), fp);
        self.extractions.insert(path, extraction);
    }

    /// Remove a file from the extraction and fingerprint cache.
    pub(crate) fn remove(&mut self, path: &Path) {
        self.fingerprints.remove(path);
        self.extractions.remove(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blake3_fingerprint_is_deterministic() {
        let content = b"fn main() { println!(\"hello\"); }";
        let fp1 = compute_fingerprint(content);
        let fp2 = compute_fingerprint(content);
        assert_eq!(fp1, fp2);
        assert_eq!(
            blake3::hash(content).as_bytes(),
            &fp1.0,
            "Fingerprint must match BLAKE3 hash"
        );
    }

    #[test]
    fn cache_update_and_remove() {
        let mut cache = IncrementalCache::new();
        let path = PathBuf::from("foo.py");
        let bytes = b"def foo(): pass\n";
        let fp = compute_fingerprint(bytes);
        let mut base = FileExtraction::empty(path.clone(), crate::language::LangId::Python);
        base.ast_node_count = 5;
        let extraction = Arc::new(base);

        cache.update(path.clone(), fp, Arc::clone(&extraction));
        assert_eq!(cache.extractions.len(), 1);
        assert_eq!(cache.fingerprints.len(), 1);

        let cached = cache.extractions.get(&path).unwrap();
        assert!(Arc::ptr_eq(&extraction, cached));

        cache.remove(&path);
        assert!(cache.extractions.is_empty());
        assert!(cache.fingerprints.is_empty());
    }
}
