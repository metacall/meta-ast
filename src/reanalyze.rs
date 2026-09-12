//! Incremental re-analysis with buffer overlays.
//!
//! Evaluates project source files against the extraction cache, re-extracts
//! only changed or new files, and (optionally) rebuilds the dependency graph.
//! Open buffer text overrides disk text.
//!
//! This module is not gated by the `watch` feature. Only the OS watcher in
//! [`crate::watch`] needs that feature.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use rayon::prelude::*;

use crate::cache::{ExtractionCache, Fingerprint};
use crate::error::{Diagnostic, Severity};
use crate::extractor::{self, ExtractOptions, ExtractionIdGenerators, InMemorySource};
use crate::graph::GraphBuilder;
use crate::input;
use crate::language::LangId;
use crate::model::{FileExtraction, SnapshotId};
use crate::pipeline::GraphAnalysis;

/// In-memory buffer text that overrides disk text during re-analysis.
#[derive(Debug, Clone)]
pub struct Overlay {
    /// Document URI, as sent by the editor.
    pub uri: String,
    /// Absolute path of the document.
    pub path: PathBuf,
    /// Current buffer text.
    pub text: String,
    /// Document version, monotonic per document.
    pub version: i32,
    /// Language of the buffer.
    pub lang: LangId,
}

/// Metrics summarizing file changes between analysis passes.
#[derive(Debug, Clone, Default)]
pub struct ChangeSet {
    /// Number of newly discovered source files.
    pub files_added: usize,
    /// Number of deleted or missing source files.
    pub files_removed: usize,
    /// Number of modified files re-parsed in this pass.
    pub files_modified: usize,
    /// Number of untouched files reusing cached extractions.
    pub files_unchanged: usize,
}

/// Path-sorted extractions plus the change metrics and diagnostics.
pub type ReanalysisOutput = (Vec<Arc<FileExtraction>>, ChangeSet, Vec<Diagnostic>);

/// Mutable state preserved across incremental re-analysis passes.
pub struct WatchState {
    pub(crate) cache: ExtractionCache,
    pub(crate) snapshot_counter: u32,
}

impl WatchState {
    /// Create a new empty state.
    pub fn new() -> Self {
        Self {
            cache: ExtractionCache::new(),
            snapshot_counter: 0,
        }
    }

    /// Read access to the extraction cache.
    pub fn cache(&self) -> &ExtractionCache {
        &self.cache
    }

    /// Write access to the extraction cache. Use it to seed extractions
    /// loaded from shards before the first re-analysis pass.
    pub fn cache_mut(&mut self) -> &mut ExtractionCache {
        &mut self.cache
    }

    /// Allocate the next monotonic snapshot ID.
    pub(crate) fn next_snapshot_id(&mut self) -> Result<SnapshotId, crate::Error> {
        self.snapshot_counter = self
            .snapshot_counter
            .checked_add(1)
            .ok_or(crate::Error::IdExhausted)?;
        SnapshotId::new(self.snapshot_counter).ok_or(crate::Error::IdExhausted)
    }
}

impl Default for WatchState {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract and reuse, without building a graph.
///
/// Discovers source files under `root`, applies overlays over disk text,
/// diffs against the cache, and re-extracts only changed or new files.
///
/// Overlay paths join the file set, so a new unsaved file is indexed. An
/// overlay path outside `root` is ignored. The returned vector is path-sorted
/// and holds `Arc` handles to reused extractions.
pub fn reanalyze_extractions(
    root: &Path,
    languages: Option<&[LangId]>,
    overlays: &[Overlay],
    state: &mut WatchState,
) -> Result<ReanalysisOutput, crate::Error> {
    let files = input::discover_files(root, languages)?;

    // Key every overlay by the same path form the walk produces, so one file
    // never enters the target set under two keys.
    let overlay_by_path: HashMap<PathBuf, &Overlay> = overlays
        .iter()
        .map(|overlay| (input::simplified_path(&overlay.path), overlay))
        .filter(|(path, _)| path.starts_with(root))
        .collect();

    let mut targets: BTreeMap<PathBuf, LangId> = files.into_iter().collect();
    for (path, overlay) in &overlay_by_path {
        targets.entry(path.clone()).or_insert(overlay.lang);
    }

    let (current_fingerprints, read_diagnostics): (HashMap<PathBuf, Fingerprint>, Vec<Diagnostic>) =
        targets
            .par_iter()
            .fold(
                || (HashMap::new(), Vec::new()),
                |(mut map, mut diags), (path, _)| {
                    match overlay_by_path.get(path) {
                        Some(overlay) => {
                            map.insert(path.clone(), Fingerprint::of(overlay.text.as_bytes()));
                        }
                        None => match std::fs::read(path) {
                            Ok(bytes) => {
                                map.insert(path.clone(), Fingerprint::of(&bytes));
                            }
                            Err(err) => diags.push(Diagnostic {
                                path: path.clone(),
                                severity: Severity::Error,
                                message: format!("Failed to read file: {err}"),
                                source_range: None,
                            }),
                        },
                    }
                    (map, diags)
                },
            )
            .reduce(
                || (HashMap::new(), Vec::new()),
                |(mut m1, mut d1), (m2, d2)| {
                    m1.extend(m2);
                    d1.extend(d2);
                    (m1, d1)
                },
            );

    let mut change_set = ChangeSet::default();
    let mut changed_disk: Vec<(PathBuf, LangId)> = Vec::new();
    let mut changed_overlays: Vec<(PathBuf, &Overlay)> = Vec::new();

    // A file that cannot be read keeps its cached extraction. Its fingerprint is
    // missing, so the stale sweep must not treat it as deleted.
    let failed_reads: HashSet<PathBuf> = read_diagnostics
        .iter()
        .map(|diag| diag.path.clone())
        .collect();

    for (path, lang) in &targets {
        let Some(curr_fp) = current_fingerprints.get(path) else {
            if failed_reads.contains(path) {
                change_set.files_unchanged += 1;
            }
            continue;
        };
        let changed = match state.cache.fingerprint_of(path) {
            Some(cached) if cached == *curr_fp => {
                change_set.files_unchanged += 1;
                false
            }
            Some(_) => {
                change_set.files_modified += 1;
                true
            }
            None => {
                change_set.files_added += 1;
                true
            }
        };
        if !changed {
            continue;
        }
        match overlay_by_path.get(path) {
            Some(overlay) => changed_overlays.push((path.clone(), overlay)),
            None => changed_disk.push((path.clone(), *lang)),
        }
    }

    let stale: Vec<PathBuf> = state
        .cache
        .paths()
        .filter(|path| !current_fingerprints.contains_key(*path) && !failed_reads.contains(*path))
        .cloned()
        .collect();
    if !stale.is_empty() {
        change_set.files_removed += stale.len();
        for path in &stale {
            state.cache.remove(path);
        }
    }

    let max_id = state.cache.max_symbol_id();
    let next_symbol_id = max_id.checked_add(1).ok_or(crate::Error::IdExhausted)?;
    #[cfg(feature = "dataflow")]
    let next_data_id = state
        .cache
        .max_data_node_id()
        .checked_add(1)
        .ok_or(crate::Error::IdExhausted)?;
    #[cfg(feature = "dataflow")]
    let id_generators = ExtractionIdGenerators::with_starts(next_symbol_id, next_data_id);
    #[cfg(not(feature = "dataflow"))]
    let id_generators = ExtractionIdGenerators::with_symbol_start(next_symbol_id);

    let options = ExtractOptions {
        skip_imports_and_refs: false,
    };

    let mut new_extractions: Vec<FileExtraction> = if changed_disk.is_empty() {
        Vec::new()
    } else {
        extractor::extract_with_id_gen(&changed_disk, &options, &id_generators).files
    };

    let mut overlay_diagnostics: Vec<Diagnostic> = Vec::new();
    for (path, overlay) in &changed_overlays {
        match extractor::extract_text_with_id_gen(
            InMemorySource {
                uri: overlay.uri.as_str(),
                text: overlay.text.as_str(),
                version: overlay.version,
                language: overlay.lang,
            },
            &options,
            &id_generators,
        ) {
            Ok(mut versioned) => {
                versioned.file.path = path.clone();
                new_extractions.push(versioned.file);
            }
            Err(error) => {
                let message = error.to_string();
                overlay_diagnostics.push(Diagnostic {
                    path: path.clone(),
                    severity: Severity::Error,
                    message: message.clone(),
                    source_range: None,
                });
                new_extractions.push(FileExtraction::failed(path.clone(), overlay.lang, message));
            }
        }
    }

    let mut merged: Vec<Arc<FileExtraction>> =
        Vec::with_capacity(state.cache.len() + new_extractions.len());
    for path in targets.keys() {
        let Some(fp) = current_fingerprints.get(path) else {
            continue;
        };
        if state.cache.fingerprint_of(path) == Some(*fp)
            && let Some(extraction) = state.cache.get(path)
        {
            merged.push(Arc::clone(extraction));
        }
    }
    for extraction in new_extractions {
        let arc = Arc::new(extraction);
        if let Some(fp) = current_fingerprints.get(&arc.path) {
            state.cache.update(arc.path.clone(), *fp, Arc::clone(&arc));
        }
        merged.push(arc);
    }
    merged.sort_by(|a, b| a.path.cmp(&b.path));

    let mut diagnostics: Vec<Diagnostic> = merged
        .iter()
        .flat_map(|file| file.diagnostics.iter().cloned())
        .collect();
    let mut read_diagnostics = read_diagnostics;
    read_diagnostics.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
    diagnostics.extend(read_diagnostics);
    diagnostics.extend(overlay_diagnostics);
    diagnostics.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));

    Ok((merged, change_set, diagnostics))
}

/// Run one step of incremental re-analysis and rebuild the graph.
///
/// Discovers all source files under `root`, reads and fingerprints each,
/// diffs against the cached state, re-extracts only changed files, and
/// rebuilds the dependency graph plus SCC from the merged extraction set.
///
/// On the first call (fresh state), every file is treated as added.
pub fn incremental_reanalyze(
    root: &Path,
    languages: Option<&[LangId]>,
    state: &mut WatchState,
) -> Result<(GraphAnalysis, ChangeSet, Vec<Diagnostic>), crate::Error> {
    let started = Instant::now();
    let (merged, change_set, mut diagnostics) = reanalyze_extractions(root, languages, &[], state)?;

    let snapshot_id = state.next_snapshot_id()?;
    let (graph, scc) = GraphBuilder::from_extractions(&merged, root, snapshot_id, &mut diagnostics);
    diagnostics.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));

    tracing::info!(
        total = merged.len(),
        added = change_set.files_added,
        removed = change_set.files_removed,
        modified = change_set.files_modified,
        unchanged = change_set.files_unchanged,
        elapsed_ms = started.elapsed().as_millis(),
        "Incremental re-analysis complete",
    );

    let analysis = GraphAnalysis {
        graph,
        scc,
        snapshot_id,
        extractions: merged,
    };

    Ok((analysis, change_set, diagnostics))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("meta_ast_reanalyze_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_file(root: &Path, name: &str, content: &str) -> PathBuf {
        let path = root.join(name);
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
        path
    }

    fn overlay(root: &Path, name: &str, content: &str) -> Overlay {
        let path = root.join(name);
        let uri = url::Url::from_file_path(&path)
            .expect("absolute path")
            .to_string();
        Overlay {
            uri,
            path,
            text: content.to_string(),
            version: 1,
            lang: LangId::Python,
        }
    }

    fn symbol_names(extractions: &[Arc<FileExtraction>]) -> Vec<String> {
        extractions
            .iter()
            .flat_map(|file| file.symbols.iter().map(|s| s.name.clone()))
            .collect()
    }

    /// Root ignores file permissions, so the read-failure case cannot run as root.
    #[cfg(unix)]
    fn writes_as_root(path: &Path) -> bool {
        std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(path).unwrap()) == 0
    }

    #[test]
    fn cold_analysis_populates_state() {
        let root = temp_dir("cold");
        write_file(&root, "a.py", "def foo(): pass\n");

        let mut state = WatchState::new();
        let (analysis, cs, diags) = incremental_reanalyze(&root, None, &mut state).unwrap();

        assert!(diags.is_empty());
        assert_eq!(cs.files_added, 1);
        assert_eq!(cs.files_unchanged, 0);
        assert_eq!(analysis.graph.file_count(), 1);
        assert_eq!(analysis.graph.symbol_count(), 1);
        assert_eq!(state.cache.extractions.len(), 1);
    }

    #[test]
    fn warm_analysis_reuses_cached_unchanged_files() {
        let root = temp_dir("warm");
        write_file(&root, "a.py", "def foo(): pass\n");
        write_file(&root, "b.py", "def bar(): pass\n");

        let mut state = WatchState::new();
        let (analysis, cs, _) = incremental_reanalyze(&root, None, &mut state).unwrap();
        assert_eq!(cs.files_added, 2);
        assert_eq!(analysis.graph.symbol_count(), 2);

        let (analysis2, cs2, _) = incremental_reanalyze(&root, None, &mut state).unwrap();
        assert_eq!(cs2.files_unchanged, 2);
        assert_eq!(cs2.files_modified, 0);
        assert_eq!(cs2.files_added, 0);
        assert_eq!(cs2.files_removed, 0);
        assert_eq!(analysis2.graph.symbol_count(), 2);
    }

    #[test]
    fn merged_extractions_stay_path_sorted() {
        let root = temp_dir("sorted");
        write_file(&root, "b.py", "def bar(): pass\n");
        write_file(&root, "a.py", "def foo(): pass\n");

        let mut state = WatchState::new();
        let (analysis, _, _) = incremental_reanalyze(&root, None, &mut state).unwrap();
        let paths: Vec<_> = analysis
            .extractions
            .iter()
            .map(|f| f.path.clone())
            .collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted);
    }

    #[test]
    fn detects_modified_file_and_re_extracts() {
        let root = temp_dir("mod");
        let a = write_file(&root, "a.py", "def original(): pass\n");
        write_file(&root, "b.py", "def bar(): pass\n");

        let mut state = WatchState::new();
        let (analysis, cs, _) = incremental_reanalyze(&root, None, &mut state).unwrap();
        assert_eq!(cs.files_added, 2);
        assert_eq!(analysis.graph.symbol_count(), 2);

        std::fs::write(&a, "def modified(): pass\ndef extra(): pass\n").unwrap();

        let (analysis2, cs2, _) = incremental_reanalyze(&root, None, &mut state).unwrap();
        assert_eq!(cs2.files_unchanged, 1);
        assert_eq!(cs2.files_modified, 1);
        assert_eq!(analysis2.graph.symbol_count(), 3);

        let names: Vec<String> = analysis2
            .graph
            .symbols()
            .map(|(_, s)| s.name.clone())
            .collect();
        assert!(names.contains(&"modified".to_string()));
        assert!(!names.contains(&"original".to_string()));
        assert!(names.contains(&"bar".to_string()));
    }

    #[test]
    fn detects_removed_file() {
        let root = temp_dir("rem");
        let a = write_file(&root, "a.py", "def foo(): pass\n");
        write_file(&root, "b.py", "def bar(): pass\n");

        let mut state = WatchState::new();
        let (analysis, cs, _) = incremental_reanalyze(&root, None, &mut state).unwrap();
        assert_eq!(cs.files_added, 2);
        assert_eq!(analysis.graph.file_count(), 2);

        std::fs::remove_file(&a).unwrap();

        let (analysis2, cs2, _) = incremental_reanalyze(&root, None, &mut state).unwrap();
        assert_eq!(cs2.files_removed, 1);
        assert_eq!(analysis2.graph.file_count(), 1);
        assert_eq!(state.cache.extractions.len(), 1);
    }

    #[test]
    fn detects_added_file() {
        let root = temp_dir("add");
        write_file(&root, "a.py", "def foo(): pass\n");

        let mut state = WatchState::new();
        let (_, cs, _) = incremental_reanalyze(&root, None, &mut state).unwrap();
        assert_eq!(cs.files_added, 1);

        write_file(&root, "b.py", "def bar(): pass\n");

        let (analysis2, cs2, _) = incremental_reanalyze(&root, None, &mut state).unwrap();
        assert_eq!(cs2.files_added, 1);
        assert_eq!(analysis2.graph.file_count(), 2);
    }

    #[test]
    fn symbol_ids_no_collision_on_re_extract() {
        let root = temp_dir("idcol");
        write_file(&root, "a.py", "def one(): pass\n");
        write_file(&root, "b.py", "def two(): pass\n");

        let mut state = WatchState::new();
        let (_, _, _) = incremental_reanalyze(&root, None, &mut state).unwrap();

        write_file(&root, "c.py", "def three(): pass\n");

        let (analysis, _, _) = incremental_reanalyze(&root, None, &mut state).unwrap();

        let mut ids: Vec<u32> = analysis
            .graph
            .symbols()
            .map(|(id, _)| id.to_raw())
            .collect();
        ids.sort();
        let expected: Vec<u32> = (1..=3).collect();
        assert_eq!(ids, expected, "symbol IDs must be unique and contiguous");
    }

    #[cfg(feature = "dataflow")]
    #[test]
    fn data_node_ids_no_collision_on_re_extract() {
        let root = temp_dir("data_idcol");
        write_file(&root, "a.py", "x = 1\ny = x + 1\n");
        write_file(&root, "b.py", "z = 2\n");

        let mut state = WatchState::new();
        let (_, _, _) = incremental_reanalyze(&root, None, &mut state).unwrap();

        write_file(&root, "b.py", "z = 2\nw = z + 3\n");

        let (_, _, _) = incremental_reanalyze(&root, None, &mut state).unwrap();

        let mut ids: Vec<u32> = state
            .cache
            .extractions
            .values()
            .flat_map(|ext| ext.data_nodes.iter().map(|d| d.id.to_raw()))
            .collect();
        let original_len = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(
            ids.len(),
            original_len,
            "data node IDs must be unique across re-extractions"
        );
    }

    #[test]
    fn diff_counts_are_exact() {
        let root = temp_dir("diffcounts");
        let a = write_file(&root, "a.py", "def a(): pass\n");
        let _b = write_file(&root, "b.py", "def b(): pass\n");

        let mut state = WatchState::new();
        let (_, cs1, _) = incremental_reanalyze(&root, None, &mut state).unwrap();
        assert_eq!(cs1.files_added, 2);
        assert_eq!(cs1.files_removed, 0);

        std::fs::remove_file(&a).unwrap();

        let (_, cs2, _) = incremental_reanalyze(&root, None, &mut state).unwrap();
        assert_eq!(cs2.files_removed, 1);
        assert_eq!(cs2.files_added, 0);
        assert_eq!(cs2.files_modified, 0);
        assert_eq!(cs2.files_unchanged, 1);
    }

    #[test]
    fn unreadable_file_emits_diagnostic() {
        let root = temp_dir("unread_diag");
        let a = write_file(&root, "a.py", "def a(): pass\n");

        // Root ignores file permissions, so the read failure cannot be provoked.
        #[cfg(unix)]
        if writes_as_root(&a) {
            return;
        }

        let mut state = WatchState::new();
        let (_, _, diags1) = incremental_reanalyze(&root, None, &mut state).unwrap();
        assert!(diags1.is_empty());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&a).unwrap().permissions();
            perms.set_mode(0o000);
            let _ = std::fs::set_permissions(&a, perms);
        }

        let (_, _, diags2) = incremental_reanalyze(&root, None, &mut state).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&a).unwrap().permissions();
            perms.set_mode(0o644);
            let _ = std::fs::set_permissions(&a, perms);
        }

        #[cfg(unix)]
        assert!(!diags2.is_empty(), "Unreadable file must emit diagnostic");
    }

    #[test]
    fn empty_project_handled() {
        let root = temp_dir("empty");
        let mut state = WatchState::new();
        let (analysis, cs, diags) = incremental_reanalyze(&root, None, &mut state).unwrap();
        assert!(diags.is_empty());
        assert_eq!(cs.files_added, 0);
        assert_eq!(analysis.graph.node_count(), 0);
    }

    #[test]
    fn overlay_overrides_disk_text() {
        let root = temp_dir("overlay");
        write_file(&root, "a.py", "def disk(): pass\n");

        let mut state = WatchState::new();
        let first = overlay(&root, "a.py", "def from_buffer(): pass\n");
        let (extractions, cs, _) =
            reanalyze_extractions(&root, None, std::slice::from_ref(&first), &mut state).unwrap();
        assert_eq!(cs.files_added, 1);
        let names = symbol_names(&extractions);
        assert!(names.contains(&"from_buffer".to_string()));
        assert!(!names.contains(&"disk".to_string()));

        let (_, cs2, _) = reanalyze_extractions(&root, None, &[first], &mut state).unwrap();
        assert_eq!(cs2.files_unchanged, 1);

        let (extractions3, cs3, _) = reanalyze_extractions(&root, None, &[], &mut state).unwrap();
        assert_eq!(cs3.files_modified, 1);
        assert!(symbol_names(&extractions3).contains(&"disk".to_string()));
    }

    #[test]
    fn overlay_adds_file_not_on_disk() {
        let root = temp_dir("overlay_new");
        let mut state = WatchState::new();
        let pending = overlay(&root, "new.py", "def fresh(): pass\n");
        let (extractions, cs, _) =
            reanalyze_extractions(&root, None, &[pending], &mut state).unwrap();
        assert_eq!(cs.files_added, 1);
        assert_eq!(extractions.len(), 1);
        assert!(symbol_names(&extractions).contains(&"fresh".to_string()));
    }

    #[test]
    fn overlay_outside_root_is_ignored() {
        let root = temp_dir("overlay_outside");
        let mut state = WatchState::new();
        let outside = Overlay {
            uri: "file:///elsewhere.py".to_string(),
            path: PathBuf::from("/definitely/outside/elsewhere.py"),
            text: "def ignored(): pass\n".to_string(),
            version: 1,
            lang: LangId::Python,
        };
        let (extractions, cs, _) =
            reanalyze_extractions(&root, None, &[outside], &mut state).unwrap();
        assert!(extractions.is_empty());
        assert_eq!(cs.files_added, 0);
    }

    /// Only Unix can deny a read to a normal user.
    #[cfg(unix)]
    #[test]
    fn read_failure_keeps_the_cached_entry() {
        let root = temp_dir("read_failure");
        let a = write_file(&root, "a.py", "def kept(): pass\n");

        let mut state = WatchState::new();
        let (first, cs, _) = incremental_reanalyze(&root, None, &mut state).unwrap();
        assert_eq!(cs.files_added, 1);
        let id_before = first.graph.symbols().next().unwrap().0.to_raw();

        if writes_as_root(&a) {
            return;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&a).unwrap().permissions();
            perms.set_mode(0o000);
            std::fs::set_permissions(&a, perms).unwrap();
        }

        let (_, cs2, diags2) = incremental_reanalyze(&root, None, &mut state).unwrap();
        assert_eq!(cs2.files_removed, 0, "a read failure is not a removal");
        assert_eq!(
            cs2.files_unchanged, 1,
            "a read failure keeps the cached file"
        );
        assert_eq!(state.cache.extractions.len(), 1);
        assert!(!diags2.is_empty(), "the read failure must be reported");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&a).unwrap().permissions();
            perms.set_mode(0o644);
            std::fs::set_permissions(&a, perms).unwrap();
        }

        let (third, cs3, _) = incremental_reanalyze(&root, None, &mut state).unwrap();
        assert_eq!(cs3.files_unchanged, 1);
        assert_eq!(
            third.graph.symbols().next().unwrap().0.to_raw(),
            id_before,
            "the transient failure must not renumber the cached file"
        );
    }

    #[test]
    fn id_exhaustion_is_an_error() {
        let root = temp_dir("id_exhaustion");
        write_file(&root, "a.py", "def a(): pass\n");

        let mut state = WatchState::new();
        let mut seated = FileExtraction::empty(root.join("a.py"), LangId::Python);
        seated.symbols.push(crate::model::Symbol {
            id: crate::model::SymbolId::new(u32::MAX).unwrap(),
            name: "seated".into(),
            kind: crate::model::SymbolKind::Function,
            language: LangId::Python,
            file_path: root.join("a.py"),
            source_range: crate::model::SourceRange {
                byte_start: 0,
                byte_end: 0,
                start: crate::model::LineColumn { line: 0, column: 0 },
                end: crate::model::LineColumn { line: 0, column: 0 },
            },
            visibility: None,
            signature: None,
            docstring: None,
            is_async: false,
        });
        state.cache_mut().update(
            root.join("a.py"),
            Fingerprint::of(b"stale"),
            Arc::new(seated),
        );

        assert!(
            incremental_reanalyze(&root, None, &mut state).is_err(),
            "an exhausted id space must report an error"
        );
    }

    /// The verbatim prefix only exists on Windows, so this key-form defect is Windows-only.
    #[cfg(windows)]
    #[test]
    fn overlay_verbatim_path_shares_the_discovered_key() {
        let root = temp_dir("overlay_key");
        write_file(&root, "a.py", "def shared(): pass\n");

        let mut state = WatchState::new();
        let mut verbatim = overlay(&root, "a.py", "def shared(): pass\n");
        verbatim.path = PathBuf::from(format!(r"\\?\{}", verbatim.path.display()));

        let (extractions, cs, _) =
            reanalyze_extractions(&root, None, std::slice::from_ref(&verbatim), &mut state)
                .unwrap();
        assert_eq!(
            cs.files_added, 1,
            "one file must not be counted under two keys"
        );
        assert_eq!(extractions.len(), 1);

        let (_, cs2, _) = reanalyze_extractions(&root, None, &[verbatim], &mut state).unwrap();
        assert_eq!(cs2.files_unchanged, 1);
        assert_eq!(cs2.files_added + cs2.files_modified, 0);
    }

    #[test]
    fn snapshot_id_allocation_is_monotonic() {
        let mut state = WatchState::new();
        let s1 = state.next_snapshot_id().unwrap();
        let s2 = state.next_snapshot_id().unwrap();
        assert_eq!(s1.to_raw(), 1);
        assert_eq!(s2.to_raw(), 2);
    }

    #[test]
    fn snapshot_counter_exhaustion_is_an_error() {
        let mut state = WatchState::new();
        state.snapshot_counter = u32::MAX;
        assert!(state.next_snapshot_id().is_err());
    }
}
