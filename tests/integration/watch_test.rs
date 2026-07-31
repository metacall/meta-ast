//! Integration tests for the incremental re-analysis engine and
//! debounced watch mode.

use std::io::Write;
use std::path::Path;

/// Creates a unique temporary directory that is cleaned up on drop.
struct TmpDir {
    path: std::path::PathBuf,
    _guard: tempfile::TempDir,
}

impl TmpDir {
    fn new() -> Self {
        let guard = tempfile::TempDir::new().unwrap();
        let path = guard.path().to_path_buf();
        Self {
            path,
            _guard: guard,
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

fn write_file(root: &Path, name: &str, content: &str) -> std::path::PathBuf {
    let path = root.join(name);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(content.as_bytes()).unwrap();
    path
}

#[test]
fn cold_start_analyzes_all_files() {
    let tmp = TmpDir::new();
    let root = tmp.path();
    write_file(root, "a.py", "def alpha(): pass\n");
    write_file(root, "b.py", "def beta(): pass\n");

    let mut state = meta_ast::watch::WatchState::new();
    let (analysis, cs, diags) = meta_ast::watch::incremental_reanalyze(root, &mut state).unwrap();

    assert!(diags.is_empty());
    assert_eq!(cs.files_added, 2);
    assert_eq!(analysis.graph.file_count(), 2);
}

#[test]
fn unchanged_files_produce_zero_changed() {
    let tmp = TmpDir::new();
    let root = tmp.path();
    write_file(root, "a.py", "def foo(): pass\n");
    write_file(root, "b.py", "class Bar: pass\n");

    let mut state = meta_ast::watch::WatchState::new();
    meta_ast::watch::incremental_reanalyze(root, &mut state).unwrap();

    let (_, cs, _) = meta_ast::watch::incremental_reanalyze(root, &mut state).unwrap();
    assert_eq!(cs.files_unchanged, 2);
    assert_eq!(cs.files_modified, 0);
    assert_eq!(cs.files_added, 0);
    assert_eq!(cs.files_removed, 0);
}

#[test]
fn file_modification_detected_and_re_extracted() {
    let tmp = TmpDir::new();
    let root = tmp.path();
    let a = write_file(root, "a.py", "def original(): pass\n");
    write_file(root, "b.py", "class B: pass\n");

    let mut state = meta_ast::watch::WatchState::new();
    let (initial, _, _) = meta_ast::watch::incremental_reanalyze(root, &mut state).unwrap();

    let initial_names: Vec<String> = initial
        .graph
        .symbols()
        .map(|(_, s)| s.name.clone())
        .collect();
    assert!(initial_names.contains(&"original".to_string()));

    std::fs::write(&a, "def modified(): pass\ndef also_new(): pass\n").unwrap();

    let (updated, cs, _) = meta_ast::watch::incremental_reanalyze(root, &mut state).unwrap();

    assert_eq!(cs.files_modified, 1);
    assert_eq!(cs.files_unchanged, 1);
    assert_eq!(updated.graph.symbol_count(), 3);

    let updated_names: Vec<String> = updated
        .graph
        .symbols()
        .map(|(_, s)| s.name.clone())
        .collect();
    assert!(updated_names.contains(&"modified".to_string()));
    assert!(updated_names.contains(&"also_new".to_string()));
    assert!(!updated_names.contains(&"original".to_string()));
}

#[test]
fn file_removal_cleans_up() {
    let tmp = TmpDir::new();
    let root = tmp.path();
    let a = write_file(root, "a.py", "def foo(): pass\n");
    write_file(root, "b.py", "def bar(): pass\n");

    let mut state = meta_ast::watch::WatchState::new();
    let (initial, _, _) = meta_ast::watch::incremental_reanalyze(root, &mut state).unwrap();
    assert_eq!(initial.graph.file_count(), 2);

    std::fs::remove_file(&a).unwrap();

    let (updated, cs, _) = meta_ast::watch::incremental_reanalyze(root, &mut state).unwrap();

    assert_eq!(cs.files_removed, 1);
    assert_eq!(updated.graph.file_count(), 1);
}

#[test]
fn file_addition_picked_up() {
    let tmp = TmpDir::new();
    let root = tmp.path();
    write_file(root, "a.py", "def one(): pass\n");

    let mut state = meta_ast::watch::WatchState::new();
    let (initial, _, _) = meta_ast::watch::incremental_reanalyze(root, &mut state).unwrap();
    assert_eq!(initial.graph.file_count(), 1);

    write_file(root, "b.py", "def two(): pass\n");

    let (updated, cs, _) = meta_ast::watch::incremental_reanalyze(root, &mut state).unwrap();

    assert_eq!(cs.files_added, 1);
    assert_eq!(updated.graph.file_count(), 2);
}

#[test]
fn symbol_ids_unique_across_cold_and_warm_runs() {
    let tmp = TmpDir::new();
    let root = tmp.path();
    write_file(root, "a.py", "def a(): pass\nclass A: pass\n");
    write_file(root, "b.py", "class B: pass\n");

    let mut state = meta_ast::watch::WatchState::new();
    let (initial, _, _) = meta_ast::watch::incremental_reanalyze(root, &mut state).unwrap();

    write_file(root, "c.py", "def c(): pass\n");

    let (updated, _, _) = meta_ast::watch::incremental_reanalyze(root, &mut state).unwrap();

    let ids_initial: std::collections::HashSet<u32> =
        initial.graph.symbols().map(|(id, _)| id.to_raw()).collect();

    let ids_updated: std::collections::HashSet<u32> =
        updated.graph.symbols().map(|(id, _)| id.to_raw()).collect();

    assert_eq!(ids_initial.len(), 3);
    assert_eq!(ids_updated.len(), 4);
    assert!(ids_initial.is_subset(&ids_updated));
}

#[test]
fn mixed_language_project_handled() {
    let tmp = TmpDir::new();
    let root = tmp.path();
    write_file(root, "main.py", "def run(): pass\n");
    write_file(root, "util.rs", "fn helper() {}\n");
    write_file(root, "index.js", "function handle() {}\n");

    let mut state = meta_ast::watch::WatchState::new();
    let (analysis, cs, diags) = meta_ast::watch::incremental_reanalyze(root, &mut state).unwrap();

    assert!(diags.is_empty());
    assert_eq!(cs.files_added, 3);
    assert_eq!(analysis.graph.file_count(), 3);

    let langs: std::collections::HashSet<_> = analysis
        .graph
        .symbols()
        .map(|(_, s)| s.name.clone())
        .collect();
    assert!(langs.contains("run"));
    assert!(langs.contains("helper"));
    assert!(langs.contains("handle"));
}

#[test]
fn scc_analysis_recomputed_on_each_tick() {
    let tmp = TmpDir::new();
    let root = tmp.path();
    write_file(root, "a.py", "import b\ndef a(): pass\n");
    write_file(root, "b.py", "import a\ndef b(): pass\n");

    let mut state = meta_ast::watch::WatchState::new();
    let (initial, _, _) = meta_ast::watch::incremental_reanalyze(root, &mut state).unwrap();

    let has_cycle = initial
        .scc
        .components
        .iter()
        .any(|c| c.is_cyclic && c.nodes.len() > 1);
    assert!(
        has_cycle,
        "cross-file import cycle should produce cyclic SCC"
    );

    let second_path = write_file(root, "c.py", "import b\ndef c(): pass\n");

    let (updated, _, _) = meta_ast::watch::incremental_reanalyze(root, &mut state).unwrap();
    assert_eq!(updated.graph.file_count(), 3);

    let _ = second_path;
}

#[test]
fn snapshot_id_increments_across_ticks() {
    let tmp = TmpDir::new();
    let root = tmp.path();
    write_file(root, "a.py", "def foo(): pass\n");

    let mut state = meta_ast::watch::WatchState::new();
    let (a1, _, _) = meta_ast::watch::incremental_reanalyze(root, &mut state).unwrap();
    let (a2, _, _) = meta_ast::watch::incremental_reanalyze(root, &mut state).unwrap();

    assert_ne!(a1.snapshot_id, a2.snapshot_id);
    assert!(a2.snapshot_id.to_raw() > a1.snapshot_id.to_raw());
}

/// Smoke test exercising the real `notify` debouncer.
/// Skipped by default because it depends on OS-level file events and timing.
/// Run manually with: `cargo test --features watch -- --include-ignored`
#[test]
#[ignore]
fn debounced_watcher_smoke() {
    let tmp = TmpDir::new();
    let root = tmp.path();
    write_file(root, "a.py", "def start(): pass\n");

    let config = meta_ast::watch::WatchConfig {
        debounce: std::time::Duration::from_millis(300),
        format: meta_ast::output::OutputFormat::Json,
        output: None,
        html: false,
        open_browser: false,
    };

    let (tx, rx) = std::sync::mpsc::channel();

    let root_clone = root.to_path_buf();
    let handle = std::thread::spawn(move || {
        let _ = meta_ast::watch::run_watch(root_clone, config, move |analysis, cs| {
            tx.send((analysis.graph.node_count(), cs.files_unchanged))
                .unwrap();
            Ok(())
        });
    });

    let (node_count, _unchanged) = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
    assert!(node_count > 0);

    std::thread::sleep(std::time::Duration::from_millis(500));
    write_file(root, "b.py", "def added(): pass\n");

    let (node_count2, _) = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
    assert!(
        node_count2 > node_count,
        "graph should grow after adding a file"
    );

    drop(rx);
    let _ = handle.join();
}
