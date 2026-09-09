//! Extraction cache and BLAKE3 content fingerprinting.
//!
//! Re-parsing source code with tree-sitter on every edit is expensive. The
//! cache keeps the previous `Arc<FileExtraction>` for every file. A BLAKE3
//! fingerprint of the bytes decides whether the extraction is reused.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::model::FileExtraction;

/// BLAKE3 cryptographic hash of a file's raw bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fingerprint([u8; 32]);

impl Fingerprint {
    /// Compute the fingerprint of a byte slice.
    pub fn of(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    /// Raw 32-byte hash.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Compute a deterministic 256-bit BLAKE3 fingerprint for content bytes.
pub fn fingerprint(bytes: &[u8]) -> Fingerprint {
    Fingerprint::of(bytes)
}

/// Cache of per-file extraction results keyed by path.
///
/// IDs inside the cached extractions stay valid. New extractions must start
/// above [`ExtractionCache::max_symbol_id`].
#[derive(Debug, Default)]
pub struct ExtractionCache {
    pub(crate) extractions: HashMap<PathBuf, Arc<FileExtraction>>,
    pub(crate) fingerprints: HashMap<PathBuf, Fingerprint>,
}

/// Maximum raw ID in an iterator, or 0 when empty.
///
/// IDs grow monotonically and are never reused after deletion.
fn max_raw(ids: impl Iterator<Item = u32>) -> u32 {
    ids.max().unwrap_or(0)
}

impl ExtractionCache {
    /// Create an empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Cached extraction for a path.
    pub fn get(&self, path: &Path) -> Option<&Arc<FileExtraction>> {
        self.extractions.get(path)
    }

    /// Cached fingerprint for a path.
    pub fn fingerprint_of(&self, path: &Path) -> Option<Fingerprint> {
        self.fingerprints.get(path).copied()
    }

    /// Find the highest raw symbol ID allocated across all cached files.
    pub fn max_symbol_id(&self) -> u32 {
        max_raw(
            self.extractions
                .values()
                .flat_map(|ext| ext.symbols.iter().map(|s| s.id.to_raw())),
        )
    }

    /// Find the highest raw data node ID allocated across all cached files.
    #[cfg(feature = "dataflow")]
    pub fn max_data_node_id(&self) -> u32 {
        max_raw(
            self.extractions
                .values()
                .flat_map(|ext| ext.data_nodes.iter().map(|d| d.id.to_raw())),
        )
    }

    /// Update or insert a file's fingerprint and shared extraction.
    pub fn update(&mut self, path: PathBuf, fp: Fingerprint, extraction: Arc<FileExtraction>) {
        self.fingerprints.insert(path.clone(), fp);
        self.extractions.insert(path, extraction);
    }

    /// Remove a file from the extraction and fingerprint cache.
    pub fn remove(&mut self, path: &Path) {
        self.fingerprints.remove(path);
        self.extractions.remove(path);
    }

    /// Number of cached files.
    pub fn len(&self) -> usize {
        self.extractions.len()
    }

    /// True when no file is cached.
    pub fn is_empty(&self) -> bool {
        self.extractions.is_empty()
    }

    /// Iterate over cached paths.
    pub fn paths(&self) -> impl Iterator<Item = &PathBuf> {
        self.extractions.keys()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blake3_fingerprint_is_deterministic() {
        let content = b"fn main() { println!(\"hello\"); }";
        let fp1 = fingerprint(content);
        let fp2 = fingerprint(content);
        assert_eq!(fp1, fp2);
        assert_eq!(
            blake3::hash(content).as_bytes(),
            fp1.as_bytes(),
            "Fingerprint must match BLAKE3 hash"
        );
    }

    #[test]
    fn cache_update_and_remove() {
        let mut cache = ExtractionCache::new();
        let path = PathBuf::from("foo.py");
        let bytes = b"def foo(): pass\n";
        let fp = fingerprint(bytes);
        let mut base = FileExtraction::empty(path.clone(), crate::language::LangId::Python);
        base.ast_node_count = 5;
        let extraction = Arc::new(base);

        cache.update(path.clone(), fp, Arc::clone(&extraction));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.fingerprint_of(&path), Some(fp));

        let cached = cache.get(&path).unwrap();
        assert!(Arc::ptr_eq(&extraction, cached));

        cache.remove(&path);
        assert!(cache.is_empty());
        assert!(cache.fingerprint_of(&path).is_none());
    }

    #[test]
    fn max_symbol_id_scans_cached_files() {
        let mut cache = ExtractionCache::new();
        assert_eq!(cache.max_symbol_id(), 0);
        let path = PathBuf::from("a.py");
        let mut file = FileExtraction::empty(path.clone(), crate::language::LangId::Python);
        file.symbols.push(crate::model::Symbol {
            id: crate::model::SymbolId::new(7).unwrap(),
            name: "seven".to_string(),
            kind: crate::model::SymbolKind::Function,
            language: crate::language::LangId::Python,
            file_path: path.clone(),
            source_range: crate::model::SourceRange {
                byte_start: 0,
                byte_end: 1,
                start: crate::model::LineColumn { line: 0, column: 0 },
                end: crate::model::LineColumn { line: 0, column: 1 },
            },
            visibility: None,
            signature: None,
            docstring: None,
            is_async: false,
        });
        cache.update(path, fingerprint(b"x"), Arc::new(file));
        assert_eq!(cache.max_symbol_id(), 7);
    }
}
