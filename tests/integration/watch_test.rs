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

    let mut state = meta_ast::WatchState::new();
    let (analysis, cs, diags) = meta_ast::incremental_reanalyze(root, None, &mut state).unwrap();

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

    let mut state = meta_ast::WatchState::new();
    meta_ast::incremental_reanalyze(root, None, &mut state).unwrap();

    let (_, cs, _) = meta_ast::incremental_reanalyze(root, None, &mut state).unwrap();
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

    let mut state = meta_ast::WatchState::new();
    let (initial, _, _) = meta_ast::incremental_reanalyze(root, None, &mut state).unwrap();

    let initial_names: Vec<String> = initial
        .graph
        .symbols()
        .map(|(_, s)| s.name.clone())
        .collect();
    assert!(initial_names.contains(&"original".to_string()));

    std::fs::write(&a, "def modified(): pass\ndef also_new(): pass\n").unwrap();

    let (updated, cs, _) = meta_ast::incremental_reanalyze(root, None, &mut state).unwrap();

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

    let mut state = meta_ast::WatchState::new();
    let (initial, _, _) = meta_ast::incremental_reanalyze(root, None, &mut state).unwrap();
    assert_eq!(initial.graph.file_count(), 2);

    std::fs::remove_file(&a).unwrap();

    let (updated, cs, _) = meta_ast::incremental_reanalyze(root, None, &mut state).unwrap();

    assert_eq!(cs.files_removed, 1);
    assert_eq!(updated.graph.file_count(), 1);
}

#[test]
fn file_addition_picked_up() {
    let tmp = TmpDir::new();
    let root = tmp.path();
    write_file(root, "a.py", "def one(): pass\n");

    let mut state = meta_ast::WatchState::new();
    let (initial, _, _) = meta_ast::incremental_reanalyze(root, None, &mut state).unwrap();
    assert_eq!(initial.graph.file_count(), 1);

    write_file(root, "b.py", "def two(): pass\n");

    let (updated, cs, _) = meta_ast::incremental_reanalyze(root, None, &mut state).unwrap();

    assert_eq!(cs.files_added, 1);
    assert_eq!(updated.graph.file_count(), 2);
}

#[test]
fn symbol_ids_unique_across_cold_and_warm_runs() {
    let tmp = TmpDir::new();
    let root = tmp.path();
    write_file(root, "a.py", "def a(): pass\nclass A: pass\n");
    write_file(root, "b.py", "class B: pass\n");

    let mut state = meta_ast::WatchState::new();
    let (initial, _, _) = meta_ast::incremental_reanalyze(root, None, &mut state).unwrap();

    write_file(root, "c.py", "def c(): pass\n");

    let (updated, _, _) = meta_ast::incremental_reanalyze(root, None, &mut state).unwrap();

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

    let mut state = meta_ast::WatchState::new();
    let (analysis, cs, diags) = meta_ast::incremental_reanalyze(root, None, &mut state).unwrap();

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

    let mut state = meta_ast::WatchState::new();
    let (initial, _, _) = meta_ast::incremental_reanalyze(root, None, &mut state).unwrap();

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

    let (updated, _, _) = meta_ast::incremental_reanalyze(root, None, &mut state).unwrap();
    assert_eq!(updated.graph.file_count(), 3);

    let _ = second_path;
}

#[test]
fn snapshot_id_increments_across_ticks() {
    let tmp = TmpDir::new();
    let root = tmp.path();
    write_file(root, "a.py", "def foo(): pass\n");

    let mut state = meta_ast::WatchState::new();
    let (a1, _, _) = meta_ast::incremental_reanalyze(root, None, &mut state).unwrap();
    let (a2, _, _) = meta_ast::incremental_reanalyze(root, None, &mut state).unwrap();

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
        emit: meta_ast::output::emitter::EmitConfig {
            output: None,
            format: meta_ast::output::OutputFormat::Json,
            html: false,
            open_browser: false,
        },
        fail_on: meta_ast::interface::report::FailOn::Error,
        languages: None,
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

/// Watch configuration for the policy tests: one broken file, no output.
fn policy_config(fail_on: meta_ast::interface::report::FailOn) -> meta_ast::watch::WatchConfig {
    meta_ast::watch::WatchConfig {
        debounce: std::time::Duration::from_millis(50),
        emit: meta_ast::output::emitter::EmitConfig {
            output: None,
            format: meta_ast::output::OutputFormat::Json,
            html: false,
            open_browser: false,
        },
        fail_on,
        languages: None,
    }
}

/// Run the watcher over a project with one unparsable file and return the
/// result of the loop once it stops.
fn run_policy_watch(
    root: &Path,
    fail_on: meta_ast::interface::report::FailOn,
) -> anyhow::Result<()> {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let stop = Arc::new(AtomicBool::new(false));
    let (seen_tx, seen_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let root_clone = root.to_path_buf();
    let stop_clone = Arc::clone(&stop);

    let handle = std::thread::spawn(move || {
        let result = meta_ast::watch::watcher::run_watch_until(
            root_clone,
            policy_config(fail_on),
            &stop_clone,
            move |_analysis, _change_set| {
                let _ = seen_tx.send(());
                Ok(())
            },
        );
        let _ = done_tx.send(());
        result
    });

    seen_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the initial analysis reaches the callback");
    stop.store(true, Ordering::Relaxed);
    done_rx
        .recv_timeout(std::time::Duration::from_secs(15))
        .expect("the watch loop must stop once the flag is set");

    handle.join().unwrap()
}

#[test]
fn a_tripped_policy_fails_the_watch_run_when_it_stops() {
    let tmp = TmpDir::new();
    let root = tmp.path();
    write_file(root, "broken.py", "def oops(:\n");

    let result = run_policy_watch(root, meta_ast::interface::report::FailOn::Warning);

    assert!(
        result.is_err(),
        "the warning policy must fail a watch run that reported a warning"
    );
}

#[test]
fn a_warning_keeps_a_default_watch_run_alive() {
    let tmp = TmpDir::new();
    let root = tmp.path();
    write_file(root, "broken.py", "def oops(:\n");

    let result = run_policy_watch(root, meta_ast::interface::report::FailOn::Error);

    assert!(
        result.is_ok(),
        "a parse warning must not fail the default policy: {result:?}"
    );
}

/// The stop flag must end the loop promptly, even while a burst of changes is
/// still being debounced, and it must leave a tree that still analyzes to a
/// deterministic graph.
#[test]
fn the_stop_flag_ends_the_loop_during_a_change_burst() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let tmp = TmpDir::new();
    let root = tmp.path();
    write_file(root, "a.py", "def alpha(): pass\n");

    let config = meta_ast::watch::WatchConfig {
        debounce: std::time::Duration::from_millis(50),
        emit: meta_ast::output::emitter::EmitConfig {
            output: None,
            format: meta_ast::output::OutputFormat::Json,
            html: false,
            open_browser: false,
        },
        fail_on: meta_ast::interface::report::FailOn::Error,
        languages: None,
    };

    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();

    let root_clone = root.to_path_buf();
    let stop_clone = Arc::clone(&stop);
    let handle = std::thread::spawn(move || {
        let result = meta_ast::watch::watcher::run_watch_until(
            root_clone,
            config,
            &stop_clone,
            move |analysis, _change_set| {
                let _ = tx.send(analysis.graph.file_count());
                Ok(())
            },
        );
        let _ = done_tx.send(result.is_ok());
    });

    let files = rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the initial analysis reaches the callback");
    assert_eq!(files, 1);

    for index in 0..24 {
        write_file(root, &format!("burst_{index}.py"), "def burst(): pass\n");
    }
    std::fs::remove_file(root.join("burst_0.py")).unwrap();

    let stop_requested = std::time::Instant::now();
    stop.store(true, Ordering::Relaxed);
    let stopped = done_rx.recv_timeout(std::time::Duration::from_secs(15));
    assert!(
        matches!(stopped, Ok(true)),
        "the watch loop must end cleanly once the stop flag is set"
    );
    let stop_latency = stop_requested.elapsed();
    assert!(
        stop_latency < std::time::Duration::from_secs(10),
        "the watch loop took {stop_latency:?} to stop"
    );
    handle.join().unwrap();

    let mut state = meta_ast::WatchState::new();
    let (analysis, change_set, _) =
        meta_ast::incremental_reanalyze(root, None, &mut state).unwrap();
    assert_eq!(
        change_set.files_added,
        analysis.graph.file_count(),
        "every remaining file must join the graph"
    );

    let mut cold = meta_ast::WatchState::new();
    let (repeated, _, _) = meta_ast::incremental_reanalyze(root, None, &mut cold).unwrap();
    assert_eq!(analysis.graph.file_count(), repeated.graph.file_count());
    assert_eq!(analysis.graph.edge_count(), repeated.graph.edge_count());
    assert_eq!(analysis.graph.symbol_count(), repeated.graph.symbol_count());
}
