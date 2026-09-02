//! Core incremental re-analysis logic.
//!
//! Evaluates project source files against the cache state, re-extracts only
//! modified or new files, and reconstructs the dependency graph and SCC.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use rayon::prelude::*;

use crate::error::Diagnostic;
use crate::extractor;
use crate::graph::GraphBuilder;
use crate::input;
use crate::language::LangId;
use crate::model::FileExtraction;
use crate::pipeline::GraphAnalysis;
use crate::watch::cache::{Fingerprint, compute_fingerprint};
use crate::watch::state::{ChangeSet, WatchState};

/// Run one step of incremental re-analysis.
///
/// Discovers all source files under `root`, reads and fingerprints each,
/// diffs against the cached state, re-extracts **only** files whose
/// fingerprints changed (or are new), and rebuilds the full dependency
/// graph + SCC from the merged extraction set.
///
/// On the very first call (fresh `WatchState`), every file is treated as
/// added, running a full cold analysis. Subsequent calls only pay the
/// parse+extract cost for changed files.
pub fn incremental_reanalyze(
    root: &Path,
    languages: Option<&[LangId]>,
    state: &mut WatchState,
) -> Result<(GraphAnalysis, ChangeSet, Vec<Diagnostic>), crate::Error> {
    let started = Instant::now();

    let files = input::discover_files(root, languages)?;

    let (current_fingerprints, read_diagnostics): (HashMap<PathBuf, Fingerprint>, Vec<Diagnostic>) =
        files
            .par_iter()
            .fold(
                || (HashMap::new(), Vec::new()),
                |(mut map, mut diags), (path, _)| {
                    match std::fs::read(path) {
                        Ok(bytes) => {
                            map.insert(path.clone(), compute_fingerprint(&bytes));
                        }
                        Err(err) => {
                            diags.push(Diagnostic {
                                path: path.clone(),
                                severity: crate::error::Severity::Error,
                                message: format!("Failed to read file: {err}"),
                                source_range: None,
                            });
                        }
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

    let mut changed: Vec<(PathBuf, LangId)> = Vec::new();
    let mut change_set = ChangeSet::default();

    for (path, lang) in &files {
        let Some(curr_fp) = current_fingerprints.get(path) else {
            continue;
        };

        match state.cache.fingerprints.get(path) {
            Some(cached_fp) if cached_fp == curr_fp => {
                change_set.files_unchanged += 1;
            }
            Some(_) => {
                change_set.files_modified += 1;
                changed.push((path.clone(), *lang));
            }
            None => {
                change_set.files_added += 1;
                changed.push((path.clone(), *lang));
            }
        }
    }

    let current_paths: std::collections::HashSet<_> = current_fingerprints.keys().collect();
    let stale: Vec<PathBuf> = state
        .cache
        .fingerprints
        .keys()
        .filter(|p| !current_paths.contains(*p))
        .cloned()
        .collect();
    if !stale.is_empty() {
        change_set.files_removed += stale.len();
        for p in &stale {
            state.cache.remove(p);
        }
    }

    let max_id = state.cache.max_symbol_id();
    #[cfg(feature = "dataflow")]
    let max_data_id = state.cache.max_data_node_id();
    #[cfg(feature = "dataflow")]
    let id_generators = extractor::ExtractionIdGenerators::with_starts(max_id + 1, max_data_id + 1);
    #[cfg(not(feature = "dataflow"))]
    let id_generators = extractor::ExtractionIdGenerators::with_symbol_start(max_id + 1);

    let new_extractions = if changed.is_empty() {
        Vec::new()
    } else {
        extractor::extract_with_id_gen(
            &changed,
            &extractor::ExtractOptions {
                skip_imports_and_refs: false,
            },
            &id_generators,
        )
        .files
    };

    let mut merged: Vec<Arc<FileExtraction>> =
        Vec::with_capacity(state.cache.extractions.len() + new_extractions.len());

    for (path, fp) in &current_fingerprints {
        if state.cache.fingerprints.get(path) == Some(fp)
            && let Some(ext) = state.cache.extractions.get(path)
        {
            merged.push(Arc::clone(ext));
        }
    }

    for ext in new_extractions {
        let arc_ext = Arc::new(ext);
        if let Some(fp) = current_fingerprints.get(&arc_ext.path) {
            state
                .cache
                .update(arc_ext.path.clone(), *fp, Arc::clone(&arc_ext));
        }
        merged.push(arc_ext);
    }

    merged.sort_by(|a, b| a.path.cmp(&b.path));

    let snapshot_id = state.next_snapshot_id();
    let mut diagnostics: Vec<Diagnostic> = merged
        .iter()
        .flat_map(|f| f.diagnostics.iter().cloned())
        .collect();
    let mut read_diagnostics = read_diagnostics;
    read_diagnostics.sort_by(|a, b| (&a.path, &a.message).cmp(&(&b.path, &b.message)));
    diagnostics.extend(read_diagnostics);

    let (graph, scc) = GraphBuilder::from_extractions(&merged, root, snapshot_id, &mut diagnostics);
    diagnostics.sort_by(|a, b| (&a.path, &a.message).cmp(&(&b.path, &b.message)));

    let elapsed = started.elapsed();
    tracing::info!(
        total = merged.len(),
        added = change_set.files_added,
        removed = change_set.files_removed,
        modified = change_set.files_modified,
        unchanged = change_set.files_unchanged,
        elapsed_ms = elapsed.as_millis(),
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
        let dir = std::env::temp_dir().join(format!("meta_ast_watch_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_file(root: &Path, name: &str, content: &str) -> PathBuf {
        let path = root.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
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

        let names: Vec<String> = analysis
            .graph
            .symbols()
            .map(|(_, s)| s.name.clone())
            .collect();
        assert!(names.contains(&"original".to_string()));

        std::fs::write(&a, "def modified(): pass\ndef extra(): pass\n").unwrap();

        let (analysis2, cs2, _) = incremental_reanalyze(&root, None, &mut state).unwrap();
        assert_eq!(cs2.files_unchanged, 1);
        assert_eq!(cs2.files_modified, 1);
        assert_eq!(analysis2.graph.symbol_count(), 3);

        let names2: Vec<String> = analysis2
            .graph
            .symbols()
            .map(|(_, s)| s.name.clone())
            .collect();
        assert!(names2.contains(&"modified".to_string()));
        assert!(!names2.contains(&"original".to_string()));
        assert!(names2.contains(&"bar".to_string()));
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
        let orig_len = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(
            ids.len(),
            orig_len,
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
}
