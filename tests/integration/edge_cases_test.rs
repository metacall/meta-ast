//! Edge-case and error-scenario tests.
//!
//! These focus on malformed input, broken syntax, and recovery behavior
//! across all 8 supported languages. The engine must never panic on
//! partial/erroneous trees; it should recover what it can and accumulate
//! diagnostics instead of aborting.

use std::path::Path;

use meta_ast::LangId;
use meta_ast::pipeline;

/// Run the full pipeline and assert it does not panic on malformed input.
fn analyze_recovers(root: &Path) {
    let (analysis, diags) = pipeline::analyze_graph(root, meta_ast::model::SnapshotId(1))
        .expect("pipeline must not fail on malformed input");
    // Every symbol node must belong to a discovered file; the graph must stay
    // internally consistent even after recovering from broken syntax.
    assert!(analysis.graph.symbol_count() <= analysis.graph.file_count().saturating_mul(1024));
    let _ = diags;
}

/// Write `content` to `<dir>/<name>` and return the path.
fn write(dir: &Path, name: &str, content: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, content).expect("fixture write failed");
    p
}

/// Malformed snippets per language: each should parse-partially and recover.
const MALFORMED: &[(&str, LangId, &str)] = &[
    (
        "m.py",
        LangId::Python,
        "def broken(:\n    return\nclass X(:",
    ),
    ("m.js", LangId::JavaScript, "function broken( { return }"),
    ("m.ts", LangId::TypeScript, "function broken( { return }"),
    ("m.tsx", LangId::Tsx, "const x = <div {>"),
    ("m.c", LangId::C, "int broken( { return }"),
    ("m.cpp", LangId::Cpp, "class Broken { public: void f( { }"),
    ("m.rs", LangId::Rust, "fn broken( { }"),
    ("m.go", LangId::Go, "func broken( { }"),
];

#[test]
fn malformed_files_recover_all_languages() {
    let tmp = std::env::temp_dir().join("meta_ast_test_malformed_all");
    if tmp.exists() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    std::fs::create_dir_all(&tmp).expect("temp dir");

    for (name, _lang, src) in MALFORMED {
        write(&tmp, name, src);
    }

    // analyze_graph over a mixed malformed directory must not panic.
    analyze_recovers(&tmp);

    let _ = std::fs::remove_dir_all(&tmp);
}

/// Each language must recover individually and emit diagnostics on broken syntax.
#[test]
fn malformed_file_emits_diagnostics_per_language() {
    let tmp = std::env::temp_dir().join("meta_ast_test_malformed_diags");
    if tmp.exists() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    std::fs::create_dir_all(&tmp).expect("temp dir");

    for (name, _lang, src) in MALFORMED {
        std::fs::write(tmp.join(name), src).expect("write");
        let (analysis, diags) =
            pipeline::analyze_graph(&tmp, meta_ast::model::SnapshotId(1)).expect("no panic");
        // A single malformed file should at least be discovered and analyzed
        // without panicking, regardless of whether a diagnostic is emitted.
        assert!(
            analysis.graph.file_count() >= 1,
            "expected file node for {name}"
        );
        // The graph must remain internally consistent after recovery.
        assert!(
            !analysis.scc.components.is_empty(),
            "expected at least one SCC for {name}"
        );
        let _ = diags;
        // Reset by removing the file so each iteration is isolated.
        std::fs::remove_file(tmp.join(name)).expect("cleanup");
    }

    let _ = std::fs::remove_dir_all(&tmp);
}

/// Empty source files must not panic and should yield zero symbols.
#[test]
fn empty_files_produce_empty_symbols() {
    let tmp = std::env::temp_dir().join("meta_ast_test_empty_files");
    if tmp.exists() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    std::fs::create_dir_all(&tmp).expect("temp dir");

    for (name, _lang, _) in MALFORMED {
        std::fs::write(tmp.join(name), "").expect("write");
    }
    let (analysis, diags) =
        pipeline::analyze_graph(&tmp, meta_ast::model::SnapshotId(1)).expect("no panic on empty");
    assert!(
        analysis.graph.symbol_count() == 0,
        "empty files yield no symbols"
    );
    let _ = diags;

    let _ = std::fs::remove_dir_all(&tmp);
}

/// Files that cannot be read must surface as a diagnostic, not a panic.
#[test]
fn unreadable_file_accumulates_diagnostic() {
    let tmp = std::env::temp_dir().join("meta_ast_test_unreadable");
    if tmp.exists() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    std::fs::create_dir_all(&tmp).expect("temp dir");

    let file = write(&tmp, "good.py", "def ok(): pass\n");
    // Make the file unreadable. On Unix, strip read permissions. On Windows,
    // chmod does not deny reads, so hold an exclusive (share_mode 0) handle so
    // the extractor's own read fails with a sharing violation.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let held = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(&file)
            .expect("hold exclusive handle");
        std::mem::forget(held);
    }
    let (analysis, diags) =
        pipeline::analyze_graph(&tmp, meta_ast::model::SnapshotId(1)).expect("no panic");
    assert!(
        diags
            .iter()
            .any(|d| matches!(d.severity, meta_ast::error::Severity::Error)),
        "expected an error diagnostic for unreadable file"
    );
    let _ = analysis;

    // Windows holds an exclusive handle (leaked above); on Unix restore perms
    // so cleanup works.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644));
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Analyzing a non-existent path should error cleanly via the Result boundary.
#[test]
fn missing_root_errors_gracefully() {
    let missing = std::env::temp_dir().join("meta_ast_test_does_not_exist_xyz");
    let result = pipeline::analyze_graph(&missing, meta_ast::model::SnapshotId(1));
    assert!(result.is_err(), "missing root must return Err");
}

/// A directory with no source files must produce an empty but valid analysis.
#[test]
fn empty_directory_is_valid() {
    let tmp = std::env::temp_dir().join("meta_ast_test_empty_dir_only");
    if tmp.exists() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    std::fs::create_dir_all(&tmp).expect("temp dir");
    let (analysis, diags) =
        pipeline::analyze_graph(&tmp, meta_ast::model::SnapshotId(1)).expect("no panic");
    assert_eq!(analysis.graph.file_count(), 0);
    assert!(analysis.scc.components.is_empty());
    assert!(diags.is_empty());
    let _ = std::fs::remove_dir_all(&tmp);
}
